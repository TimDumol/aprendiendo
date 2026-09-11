use rmcp::{
    Json, ServerHandler,
    handler::server::{router::tool::ToolRouter, tool::ToolCallContext, wrapper::Parameters},
    model::{
        CallToolRequestParams, CallToolResponse, Implementation, ServerCapabilities, ServerInfo,
    },
    tool, tool_handler, tool_router,
};
use std::time::Instant;
use tracing::error;

use crate::{
    activities,
    db::SharedStore,
    model::{
        DomainResult, LearningContextRequest, PracticeBriefRequest, RecentPracticeRequest,
        RecordPracticeSessionRequest, RecordPracticeSessionResponse, ReviewQueueRequest,
        TaxonomyRequest, UpsertConceptRequest, UpsertWeaknessRequest,
    },
    telemetry::{self, CompletionMetrics, PhaseTimer, ServerTelemetry},
    tutoring::RecordTutoringSessionRequest,
};

fn inline_input_schema(
    value: &serde_json::Value,
    root: &serde_json::Value,
    depth: usize,
) -> serde_json::Value {
    // None of the domain input models are recursive. Preserve a reference if a
    // future recursive model reaches the guard instead of expanding forever.
    if depth > 32 {
        return value.clone();
    }
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            if let Some(reference) = map
                .get("$ref")
                .and_then(|v| v.as_str())
                .and_then(|s| s.strip_prefix('#'))
            {
                if let Some(target) = root.pointer(reference) {
                    if let Some(expanded) = inline_input_schema(target, root, depth + 1).as_object()
                    {
                        out = expanded.clone();
                    }
                }
            }
            for (key, child) in map {
                if key == "$defs" {
                    out.insert(key.clone(), child.clone());
                    continue;
                }
                if key == "$ref" && !out.is_empty() {
                    continue;
                }
                out.insert(key.clone(), inline_input_schema(child, root, depth + 1));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .iter()
                .map(|v| inline_input_schema(v, root, depth + 1))
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// Extract only known recording text fields for the separately filterable text
/// telemetry event. This intentionally does not walk arbitrary strings:
/// identifiers, secrets and client metadata must never be copied to logs.
fn recording_text_fields(
    tool: &str,
    arguments: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Vec<serde_json::Value> {
    if !matches!(tool, "record_tutoring_session" | "record_practice_session") {
        return Vec::new();
    }
    const TEXT_KEYS: &[&str] = &[
        "prompt",
        "transcript",
        "response",
        "corrected_response",
        "reference_answer",
        "feedback",
        "produced",
        "correction",
        "error_span",
        "notes",
        "original",
        "suggestion",
        "note",
        "text",
        "rationale",
        "scenario",
        "topic",
        "evidence",
    ];
    const CONTAINER_KEYS: &[&str] = &[
        "turns",
        "attempts",
        "observations",
        "findings",
        "interventions_after",
        "evidence",
        "production_evidence",
        "items",
        "activity_runs",
        "stimuli",
        "new_weaknesses",
        "target_relations",
        "concept_links",
        "reviews",
    ];
    fn walk(value: &serde_json::Value, path: &str, fields: &mut Vec<serde_json::Value>) {
        if fields.len() >= 150 {
            return;
        }
        match value {
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    let child_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    if TEXT_KEYS.contains(&key.as_str()) {
                        if let Some(text) = child.as_str()
                            && !text.trim().is_empty()
                            && text.len() <= 4_000
                        {
                            fields.push(serde_json::json!({"path": child_path, "text": text}));
                        }
                    }
                    // A text key is allowlisted at any known recording
                    // object, but arbitrary unknown objects are not traversed:
                    // malformed client metadata must not become log payload.
                    if CONTAINER_KEYS.contains(&key.as_str()) {
                        walk(child, &child_path, fields);
                    }
                    if fields.len() >= 150 {
                        return;
                    }
                }
            }
            serde_json::Value::Array(values) => {
                for (index, child) in values.iter().enumerate() {
                    walk(child, &format!("{path}[{index}]"), fields);
                    if fields.len() >= 150 {
                        return;
                    }
                }
            }
            _ => {}
        }
    }
    let value = serde_json::Value::Object(arguments.cloned().unwrap_or_default());
    let mut fields = Vec::new();
    walk(&value, "", &mut fields);
    fields
}

#[derive(Clone)]
pub struct LearningServer {
    store: SharedStore,
    tool_router: ToolRouter<Self>,
    telemetry: ServerTelemetry,
}

impl LearningServer {
    pub fn new(store: SharedStore, oauth_scope: Option<String>) -> Self {
        let mut tool_router = Self::tool_router();
        let schemes = match oauth_scope {
            Some(scope) => serde_json::json!([{"type":"oauth2","scopes":[scope]}]),
            None => serde_json::json!([{"type":"noauth"}]),
        };
        for route in tool_router.map.values_mut() {
            let mut meta = rmcp::model::MetaObject::new();
            meta.insert("securitySchemes".into(), schemes.clone());
            route.attr.meta = Some(meta);
            // ChatGPT's connector can lose types behind nested local $refs.
            // Publish concrete input shapes at the source, not generated client files.
            let schema = serde_json::Value::Object((*route.attr.input_schema).clone());
            let expanded = inline_input_schema(&schema, &schema, 0);
            if let Some(expanded_object) = expanded.as_object() {
                route.attr.input_schema = std::sync::Arc::new(expanded_object.clone());
            }
        }
        Self {
            store,
            tool_router,
            telemetry: ServerTelemetry::new(),
        }
    }
    fn text(v: &str, field: &str, max: usize) -> Result<(), String> {
        if v.trim().is_empty() {
            return Err(format!("{field} must not be empty"));
        }
        if v.len() > max {
            return Err(format!("{field} must be at most {max} bytes"));
        }
        Ok(())
    }
    fn optional(v: Option<&str>, field: &str, max: usize) -> Result<(), String> {
        if let Some(v) = v {
            Self::text(v, field, max)?;
        }
        Ok(())
    }
    fn list<T>(v: Option<&Vec<T>>, field: &str, max: usize) -> Result<(), String> {
        if let Some(v) = v
            && v.len() > max
        {
            return Err(format!("{field} may contain at most {max} values"));
        }
        Ok(())
    }
    fn production_error(operation: &'static str, err: anyhow::Error) -> String {
        if err.downcast_ref::<rusqlite::Error>().is_some() {
            Self::operation_error(operation, err)
        } else {
            err.to_string()
        }
    }
    fn operation_error(operation: &'static str, err: anyhow::Error) -> String {
        error!(operation,error=%err,"learning database operation failed");
        format!("{operation} failed; the server log contains the diagnostic detail")
    }
    fn record_operation_error(error: crate::db::RecordPracticeError) -> String {
        match error {
            crate::db::RecordPracticeError::InvalidArgument(message) => {
                telemetry::format_validation_error("invalid_argument", &message)
            }
            crate::db::RecordPracticeError::UnknownReference(message) => {
                telemetry::format_validation_error("unknown_reference", &message)
            }
            crate::db::RecordPracticeError::IdempotencyConflict => {
                "idempotency_conflict: idempotency_key was already used with a different payload"
                    .into()
            }
            crate::db::RecordPracticeError::Internal(error) => {
                error!(operation = "record_practice_session", error = %error, "learning database operation failed");
                "record_practice_session failed; the server log contains the diagnostic detail"
                    .into()
            }
        }
    }
    fn domain(data: serde_json::Value) -> Json<DomainResult> {
        Json(DomainResult { data })
    }

    fn record_operation_error_named(
        operation: &'static str,
        error: crate::db::RecordPracticeError,
    ) -> String {
        match error {
            crate::db::RecordPracticeError::InvalidArgument(message) => {
                telemetry::format_validation_error("invalid_argument", &message)
            }
            crate::db::RecordPracticeError::UnknownReference(message) => {
                telemetry::format_validation_error("unknown_reference", &message)
            }
            crate::db::RecordPracticeError::IdempotencyConflict => {
                "idempotency_conflict: idempotency_key was already used with a different payload"
                    .into()
            }
            crate::db::RecordPracticeError::Internal(error) => {
                error!(operation, error = %error, "learning database operation failed");
                format!("{operation} failed; the server log contains the diagnostic detail")
            }
        }
    }
}

#[tool_router]
impl LearningServer {
    /// Return durable learner preferences without loading historical sessions.
    #[tool(
        name = "get_practice_preferences",
        description = "Inspect durable practice preferences when needed or when the learner requested a change. Ordinary session recording does not require rewriting preferences.",
        annotations(read_only_hint = true, destructive_hint = false)
    )]
    pub async fn get_practice_preferences(&self) -> Result<Json<DomainResult>, String> {
        self.store
            .production_action("get", serde_json::json!({}))
            .await
            .map(|data| Json(DomainResult { data }))
            .map_err(|e| Self::production_error("get_practice_preferences", e))
    }
    /// Partially update durable learner preferences with a version check and learner-intent provenance. Session-only requests belong in session_overrides.
    #[tool(
        name = "update_practice_preferences",
        description = "Apply a learner-requested preference change with its version check. Session-specific choices belong in session_overrides; do not update preferences as a recording prerequisite.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    pub async fn update_practice_preferences(
        &self,
        Parameters(request): Parameters<crate::production::UpdatePreferencesRequest>,
    ) -> Result<Json<DomainResult>, String> {
        self.store
            .production_action(
                "update",
                serde_json::to_value(request).map_err(|e| e.to_string())?,
            )
            .await
            .map(|data| Json(DomainResult { data }))
            .map_err(|e| Self::production_error("update_practice_preferences", e))
    }
    /// Validate one proposed initial turn or adaptive follow-up before delivery. Rule checks require additional tutor semantic review; a pass does not prove spontaneous cognition.
    #[tool(
        name = "validate_practice_plan",
        description = "Normal tutoring preparation: validate each required communicative prompt before delivery, then adapt follow-ups after the learner responds. This is a prompt check, not a prerequisite to recording a completed session.",
        annotations(read_only_hint = true, destructive_hint = false)
    )]
    pub async fn validate_practice_plan(
        &self,
        Parameters(request): Parameters<crate::production::ValidatePlanRequest>,
    ) -> Result<Json<DomainResult>, String> {
        self.store
            .production_action(
                "validate",
                serde_json::to_value(request).map_err(|e| e.to_string())?,
            )
            .await
            .map(|data| Json(DomainResult { data }))
            .map_err(|e| Self::production_error("validate_practice_plan", e))
    }
    #[tool(
        name = "get_learning_context",
        description = "Read broader learner context when needed. Reuse information already returned by get_practice_brief or a recording response; do not require this read before recording.",
        annotations(
            title = "Get learning context",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn get_learning_context(
        &self,
        Parameters(mut request): Parameters<LearningContextRequest>,
    ) -> Result<Json<DomainResult>, String> {
        let n = request.recent_sessions.unwrap_or(5);
        if n > 20 {
            return Err("recent_sessions must be between 0 and 20".into());
        }
        if request
            .weakness_limit
            .is_some_and(|x| !(1..=50).contains(&x))
        {
            return Err("weakness_limit must be between 1 and 50".into());
        }
        Self::list(request.concept_keys.as_ref(), "concept_keys", 50)?;
        Self::list(request.scheme_keys.as_ref(), "scheme_keys", 10)?;
        Self::list(request.collection_keys.as_ref(), "collection_keys", 20)?;
        Self::list(request.target_types.as_ref(), "target_types", 8)?;
        for key in request
            .concept_keys
            .iter()
            .flatten()
            .chain(request.scheme_keys.iter().flatten())
            .chain(request.collection_keys.iter().flatten())
        {
            Self::text(key, "filter key", 160)?;
        }
        request.recent_sessions = Some(n);
        self.store
            .learning_context(request)
            .await
            .map(Self::domain)
            .map_err(|e| Self::operation_error("get_learning_context", e))
    }

    #[tool(
        name = "get_recent_practice",
        description = "Read recent practice when needed for context or feedback. Reuse information already returned by get_practice_brief or a recording response; do not require this read before recording. Use session_id for one exact session, or task_ref with limit=1 for the immediately preceding related task. Use detail=evidence for a compact evidence view retaining outcomes, hints/corrections, exposure, independence metadata and source references without raw attempt payloads.",
        annotations(
            title = "Get recent practice",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn get_recent_practice(
        &self,
        Parameters(request): Parameters<RecentPracticeRequest>,
    ) -> Result<Json<DomainResult>, String> {
        let n = request.limit.unwrap_or(3);
        if !(1..=50).contains(&n) {
            return Err("limit must be between 1 and 50".into());
        }
        if request.session_id.is_some_and(|id| id <= 0) {
            return Err("session_id must be a positive integer".into());
        }
        Self::optional(request.task_ref.as_deref(), "task_ref", 160)?;
        Self::optional(request.skill.as_deref(), "skill", 120)?;
        Self::list(request.exercise_type_keys.as_ref(), "exercise_type_keys", 6)?;
        Self::list(request.drill_types.as_ref(), "drill_types", 12)?;
        Self::list(request.weakness_keys.as_ref(), "weakness_keys", 20)?;
        Self::list(request.categories.as_ref(), "categories", 10)?;
        Self::list(request.concept_keys.as_ref(), "concept_keys", 50)?;
        Self::list(request.scheme_keys.as_ref(), "scheme_keys", 10)?;
        Self::list(request.collection_keys.as_ref(), "collection_keys", 20)?;
        Self::list(request.target_types.as_ref(), "target_types", 8)?;
        Self::list(request.activity_types.as_ref(), "activity_types", 10)?;
        Self::list(request.response_modes.as_ref(), "response_modes", 2)?;
        if request.activity_types.as_ref().is_some_and(Vec::is_empty) {
            return Err("activity_types must not be an empty list".into());
        }
        if request.response_modes.as_ref().is_some_and(Vec::is_empty) {
            return Err("response_modes must not be an empty list".into());
        }
        for key in request
            .concept_keys
            .iter()
            .flatten()
            .chain(request.scheme_keys.iter().flatten())
            .chain(request.collection_keys.iter().flatten())
        {
            Self::text(key, "filter key", 160)?;
        }
        for key in request.weakness_keys.iter().flatten() {
            Self::text(key, "weakness_key", 160)?;
        }
        for category in request.categories.iter().flatten() {
            Self::text(category, "category", 80)?;
        }

        for v in request.weakness_keys.iter().flatten() {
            Self::text(v, "weakness_key", 160)?;
        }
        for v in request.categories.iter().flatten() {
            Self::text(v, "category", 80)?;
        }
        Self::optional(request.from_date.as_deref(), "from_date", 10)?;
        Self::optional(request.to_date.as_deref(), "to_date", 10)?;
        self.store
            .recent_practice(request)
            .await
            .map(Self::domain)
            .map_err(|e| Self::operation_error("get_recent_practice", e))
    }

    #[tool(
        name = "get_practice_brief",
        description = "Normal tutoring context: reuse the returned known weakness keys and policy version, keep target wording private, and validate each delivered prompt before the learner responds.",
        annotations(
            title = "Get practice brief",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn get_practice_brief(
        &self,
        Parameters(request): Parameters<PracticeBriefRequest>,
    ) -> Result<Json<DomainResult>, String> {
        let mix = request.drill_mix.unwrap_or(crate::model::DrillMix::Auto);
        if request.activity_type.is_some()
            && matches!(
                mix,
                crate::model::DrillMix::TranslationOnly
                    | crate::model::DrillMix::RecognitionToProduction
                    | crate::model::DrillMix::Fluency
            )
        {
            return Err(format!(
                "drill_mix {:?} is incompatible with activity requests",
                mix
            ));
        }
        match mix {
            crate::model::DrillMix::Custom
                if request
                    .allowed_drill_types
                    .as_ref()
                    .is_none_or(Vec::is_empty) =>
            {
                return Err("allowed_drill_types is required for custom drill_mix".into());
            }
            crate::model::DrillMix::Fluency if request.count.is_some_and(|x| x != 1) => {
                return Err("fluency briefs require count=1".into());
            }
            _ => {}
        }
        if request.count.is_some_and(|x| x == 0 || x > 20) {
            return Err("count must be between 1 and 20".into());
        }
        if request
            .planned_duration_seconds
            .is_some_and(|x| !(1..=7_200).contains(&x))
        {
            return Err("planned_duration_seconds must be between 1 and 7200".into());
        }
        if request.activity_type.is_none() && request.planned_duration_seconds.is_some() {
            return Err("planned_duration_seconds requires activity_type".into());
        }
        if let Some(activity_type) = request.activity_type {
            if activities::is_single_response(activity_type)
                && request.count.is_some_and(|value| value != 1)
            {
                return Err(format!("{} requires count=1", activity_type.as_str()));
            }
            if let Some(config) = &request.activity_config {
                activities::validate_config_fields(activity_type, config)
                    .map_err(|e| e.to_string())?;
                activities::validate_config_bounds(config).map_err(|e| e.to_string())?;
                if config.is_empty() {
                    return Err(
                        "activity config must contain at least one field when supplied".into(),
                    );
                }
            }
        } else if request.activity_config.is_some() {
            return Err("activity_config requires activity_type".into());
        }
        Self::list(
            request.allowed_drill_types.as_ref(),
            "allowed_drill_types",
            12,
        )?;
        Self::list(request.weakness_keys.as_ref(), "weakness_keys", 20)?;
        Self::list(request.categories.as_ref(), "categories", 10)?;
        Self::list(request.concept_keys.as_ref(), "concept_keys", 50)?;
        Self::list(request.scheme_keys.as_ref(), "scheme_keys", 10)?;
        Self::list(request.collection_keys.as_ref(), "collection_keys", 20)?;
        Self::list(request.target_types.as_ref(), "target_types", 8)?;
        for key in request
            .concept_keys
            .iter()
            .flatten()
            .chain(request.scheme_keys.iter().flatten())
            .chain(request.collection_keys.iter().flatten())
        {
            Self::text(key, "filter key", 160)?;
        }
        for key in request.weakness_keys.iter().flatten() {
            Self::text(key, "weakness_key", 160)?;
        }
        for category in request.categories.iter().flatten() {
            Self::text(category, "category", 80)?;
        }
        Self::optional(request.as_of.as_deref(), "as_of", 10)?;
        if request.recent_prompts_per_weakness.is_some_and(|x| x > 10) {
            return Err("recent_prompts_per_weakness must be between 0 and 10".into());
        }
        Self::optional(request.level.as_deref(), "level", 20)?;
        self.store
            .practice_brief(request)
            .await
            .map(Self::domain)
            .map_err(|e| Self::operation_error("get_practice_brief", e))
    }

    #[tool(
        name = "record_practice_session",
        description = "Advanced canonical fallback for one completed practice session (contract_version=2), including activity runs and workflows not covered by record_tutoring_session. Supply production_evidence with actual prompts linked through items, separate attempts, observation assessments and interventions. Valid evidence is saved even when reviews are skipped; inspect review_decisions. Unknown cueing cannot support review. Two materially varied independent observations and supported rating evidence are required. Use one exact canonical exercise_type_key and session-wide observation numbers. Retrying an identical request returns status=replayed; a changed request with the same idempotency_key conflicts.",
        annotations(
            title = "Record practice session",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false,
            idempotent_hint = true
        )
    )]
    pub async fn record_practice_session(
        &self,
        Parameters(request): Parameters<RecordPracticeSessionRequest>,
    ) -> Result<Json<DomainResult<RecordPracticeSessionResponse>>, String> {
        self.store
            .record_practice(request)
            .await
            .map(|data| Json(DomainResult { data }))
            .map_err(Self::record_operation_error)
    }

    #[tool(
        name = "record_tutoring_session",
        description = "Preferred normal tutoring recorder: atomically save a completed session with turns, attempts, hints, retries, observations, findings and optional conservative review proposals in one call. Array order supplies item, attempt and observation numbers; use explicit references only for review evidence or a retry across turns. Record-only sessions may omit production evidence and retain transcripts, observations, findings and intervention text without scheduling reviews. This expands deterministically into record_practice_session and preserves exact idempotent replay. Use record_practice_session for advanced activity runs or other canonical workflows; do not call upserts or read tools as recording prerequisites.",
        annotations(
            title = "Record tutoring session",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false,
            idempotent_hint = true
        )
    )]
    pub async fn record_tutoring_session(
        &self,
        Parameters(request): Parameters<RecordTutoringSessionRequest>,
    ) -> Result<Json<DomainResult<RecordPracticeSessionResponse>>, String> {
        let canonical = {
            let _expansion_timer = PhaseTimer::new("compact_expansion");
            crate::tutoring::expand(request).map_err(|message| {
                telemetry::format_validation_error("invalid_argument", &message)
            })?
        };
        self.store
            .record_practice(canonical)
            .await
            .map(|data| Json(DomainResult { data }))
            .map_err(|error| Self::record_operation_error_named("record_tutoring_session", error))
    }

    #[tool(
        name = "get_review_queue",
        description = "Read the supported review queue when needed. Recording may validly produce zero schedule changes; never add practice merely to obtain a rating.",
        annotations(
            title = "Get weakness review queue",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn get_review_queue(
        &self,
        Parameters(request): Parameters<ReviewQueueRequest>,
    ) -> Result<Json<DomainResult>, String> {
        let n = request.limit.unwrap_or(20);
        if !(1..=50).contains(&n) {
            return Err("limit must be between 1 and 50".into());
        }
        Self::optional(request.category.as_deref(), "category", 80)?;
        Self::optional(request.as_of.as_deref(), "as_of", 10)?;
        Self::list(request.concept_keys.as_ref(), "concept_keys", 50)?;
        Self::list(request.scheme_keys.as_ref(), "scheme_keys", 10)?;
        Self::list(request.collection_keys.as_ref(), "collection_keys", 20)?;
        Self::list(request.target_types.as_ref(), "target_types", 8)?;
        self.store
            .review_queue(request)
            .await
            .map(Self::domain)
            .map_err(|e| Self::operation_error("get_review_queue", e))
    }

    #[tool(
        name = "get_taxonomy",
        description = "Occasional taxonomy classification or diagnostics. Do not call this as a prerequisite to ordinary session recording.",
        annotations(
            title = "Get taxonomy",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn get_taxonomy(
        &self,
        Parameters(request): Parameters<TaxonomyRequest>,
    ) -> Result<Json<DomainResult>, String> {
        let depth = request.depth.unwrap_or(3);
        if !(1..=6).contains(&depth) {
            return Err("depth must be between 1 and 6".into());
        }
        if request.limit.is_some_and(|n| n == 0 || n > 500) {
            return Err("limit must be between 1 and 500".into());
        }
        Self::list(request.scheme_keys.as_ref(), "scheme_keys", 10)?;
        Self::list(request.root_concept_keys.as_ref(), "root_concept_keys", 50)?;
        for key in request
            .scheme_keys
            .iter()
            .flatten()
            .chain(request.root_concept_keys.iter().flatten())
        {
            Self::text(key, "taxonomy key", 160)?;
        }
        self.store
            .taxonomy(request)
            .await
            .map(Self::domain)
            .map_err(|e| Self::operation_error("get_taxonomy", e))
    }

    #[tool(
        name = "upsert_concept",
        description = "Maintenance only: explicitly edit or curate taxonomy concepts outside session recording. Defer taxonomy curation during a tutoring recording and do not call this before saving a session.",
        annotations(
            title = "Maintain taxonomy concept",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn upsert_concept(
        &self,
        Parameters(request): Parameters<UpsertConceptRequest>,
    ) -> Result<Json<DomainResult>, String> {
        Self::text(&request.key, "key", 160)?;
        Self::text(&request.scheme_key, "scheme_key", 80)?;
        Self::text(&request.label, "label", 500)?;
        Self::optional(request.definition.as_deref(), "definition", 4_000)?;
        Self::optional(request.source_uri.as_deref(), "source_uri", 2_000)?;
        Self::list(
            request.broader_concept_keys.as_ref(),
            "broader_concept_keys",
            20,
        )?;
        Self::list(Some(&request.edges), "edges", 20)?;
        for key in request.broader_concept_keys.iter().flatten() {
            Self::text(key, "broader_concept_key", 160)?;
        }
        for edge in &request.edges {
            Self::text(&edge.other_concept_key, "edge.other_concept_key", 160)?;
            Self::text(&edge.predicate, "edge.predicate", 40)?;
            Self::optional(edge.provenance.as_deref(), "edge.provenance", 20)?;
            Self::optional(edge.notes.as_deref(), "edge.notes", 2_000)?;
        }
        self.store
            .upsert_concept_json(serde_json::to_value(&request).map_err(|e| e.to_string())?)
            .await
            .map(Self::domain)
            .map_err(|e| Self::operation_error("upsert_concept", e))
    }

    #[tool(
        name = "upsert_weakness",
        description = "Maintenance only: explicitly edit an existing weakness or curate learning targets outside session recording. For discoveries during practice, use the recorder's new_weaknesses. Do not call this before recording a session. Standalone upsert can activate a new weakness and overwrite existing metadata.",
        annotations(
            title = "Maintain weakness",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn upsert_weakness(
        &self,
        Parameters(request): Parameters<UpsertWeaknessRequest>,
    ) -> Result<Json<DomainResult>, String> {
        Self::text(&request.key, "key", 160)?;
        Self::text(&request.category, "category", 80)?;
        Self::text(&request.description, "description", 1_000)?;
        Self::optional(request.target_pattern.as_deref(), "target_pattern", 1_000)?;
        Self::list(
            request.recommended_drill_types.as_ref(),
            "recommended_drill_types",
            12,
        )?;
        Self::list(Some(&request.concept_links), "concept_links", 20)?;
        Self::list(Some(&request.target_relations), "target_relations", 20)?;
        for link in &request.concept_links {
            Self::text(&link.concept_key, "concept_link.concept_key", 160)?;
        }
        for relation in &request.target_relations {
            Self::text(
                &relation.other_weakness_key,
                "target_relation.other_weakness_key",
                160,
            )?;
            Self::text(&relation.predicate, "target_relation.predicate", 40)?;
            Self::optional(relation.notes.as_deref(), "target_relation.notes", 2_000)?;
        }
        let patch = serde_json::to_value(&request).map_err(|e| e.to_string())?;
        self.store
            .upsert_weakness_json(patch)
            .await
            .map(Self::domain)
            .map_err(|e| Self::operation_error("upsert_weakness", e))
    }

    #[tool(
        name = "get_data_status",
        description = "Occasional storage and scheduler diagnostics. Do not call this as a prerequisite to ordinary session recording.",
        annotations(
            title = "Get learning data status",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn get_data_status(&self) -> Result<Json<DomainResult>, String> {
        self.store
            .data_status()
            .await
            .map(Self::domain)
            .map_err(|e| Self::operation_error("get_data_status", e))
    }
}

fn bounded_tool_error_outcome(response: &CallToolResponse) -> &'static str {
    let CallToolResponse::Complete(result) = response else {
        return "incomplete";
    };
    if result.is_error != Some(true) {
        return "success";
    }
    let text = result
        .content
        .iter()
        .find_map(|content| content.as_text().map(|value| value.text.as_str()))
        .unwrap_or_default();
    let prefix = text.split(':').next().unwrap_or_default();
    match prefix {
        "invalid_argument"
        | "preference_version_conflict"
        | "failed to deserialize parameters"
        | "Failed to parse parameters" => "invalid_argument",
        "unknown_reference" => "unknown_reference",
        "idempotency_conflict" => "idempotency_conflict",
        _ if text.contains("failed; the server log contains the diagnostic detail") => {
            "internal_error"
        }
        _ => "transport_error",
    }
}

fn recording_metrics(response: &CallToolResponse) -> (Option<&'static str>, CompletionMetrics) {
    let mut metrics = CompletionMetrics::default();
    let CallToolResponse::Complete(result) = response else {
        metrics.outcome = "incomplete";
        return (None, metrics);
    };
    metrics.outcome = bounded_tool_error_outcome(response);
    if metrics.outcome != "success" {
        return (None, metrics);
    }
    let Some(data) = result
        .structured_content
        .as_ref()
        .and_then(|value| value.get("data"))
    else {
        return (None, metrics);
    };
    let status = match data.get("status").and_then(serde_json::Value::as_str) {
        Some("created") => Some("created"),
        Some("replayed") => Some("replayed"),
        _ => None,
    };
    metrics.record_status = status;
    metrics.session_id = data.get("session_id").and_then(serde_json::Value::as_i64);
    metrics.item_count = data
        .get("item_count")
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| usize::try_from(n).ok());
    metrics.attempt_count = data
        .get("attempt_count")
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| usize::try_from(n).ok());
    metrics.observation_count = data
        .get("observation_count")
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| usize::try_from(n).ok());
    metrics.new_weakness_count = data
        .get("new_weaknesses_created")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len);
    let applied = data
        .get("review_updates")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len);
    let decisions = data
        .get("review_decisions")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len);
    metrics.review_applied_count = applied;
    metrics.review_skipped_count = match (decisions, applied) {
        (Some(decisions), Some(applied)) => Some(decisions.saturating_sub(applied)),
        (Some(_), None) => None,
        _ => None,
    };
    (status, metrics)
}

#[tool_handler(router=self.tool_router)]
impl ServerHandler for LearningServer {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        let requested_name = request.name.to_string();
        let known = self.tool_router.has_route(&requested_name);
        let tool = if known {
            requested_name.clone()
        } else {
            "unknown".to_owned()
        };
        let arguments_json_bytes = request.arguments.as_ref().and_then(|arguments| {
            serde_json::to_vec(&serde_json::Value::Object(arguments.clone()))
                .ok()
                .map(|bytes| bytes.len())
        });
        let call_id = uuid::Uuid::new_v4().to_string();
        telemetry::emit_started(&self.telemetry, &call_id, &tool, arguments_json_bytes);
        let text_fields = recording_text_fields(&tool, request.arguments.as_ref());
        if !text_fields.is_empty()
            && let Ok(serialized) = serde_json::to_string(&text_fields)
        {
            telemetry::emit_text(&self.telemetry, &call_id, &tool, &serialized);
        }
        let started = Instant::now();
        let call_context = telemetry::CallContext {
            process_instance_id: self.telemetry.process_instance_id.clone(),
            call_id: call_id.clone().into(),
            tool: tool.clone().into(),
        };
        let result = telemetry::with_call_context(call_context, async {
            let tool_context = ToolCallContext::new(self, request, context);
            self.tool_router.call(tool_context).await
        })
        .await;
        let mut metrics = match &result {
            Ok(response) => recording_metrics(response).1,
            Err(_) => CompletionMetrics {
                outcome: if known {
                    "transport_error"
                } else {
                    "unknown_tool"
                },
                ..CompletionMetrics::default()
            },
        };
        if !known {
            metrics.outcome = "unknown_tool";
        }
        metrics.arguments_json_bytes = arguments_json_bytes;
        if let Ok(response) = &result {
            metrics.response_json_bytes = match response {
                rmcp::model::CallToolResponse::Complete(tool_response) => {
                    serde_json::to_vec(tool_response)
                        .ok()
                        .map(|bytes| bytes.len())
                }
                _ => None,
            };
            if metrics.outcome == "invalid_argument" || metrics.outcome == "unknown_reference" {
                if let rmcp::model::CallToolResponse::Complete(tool_response) = response {
                    if let Some(message) = tool_response
                        .content
                        .iter()
                        .find_map(|content| content.as_text().map(|text| text.text.as_str()))
                    {
                        telemetry::emit_validation_failed(
                            &self.telemetry,
                            &call_id,
                            &tool,
                            metrics.outcome,
                            message,
                        );
                    }
                }
            }
        }
        metrics.duration_ms = started.elapsed().as_millis() as u64;
        telemetry::emit_completed(&self.telemetry, &call_id, &tool, &metrics);
        result
    }

    fn get_info(&self) -> ServerInfo {
        let mut info=ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions("Use these tools only for the configured single-learner Spanish database. Normal tutoring: get_practice_brief, validate each communicative prompt before delivery, then record the completed session once with record_tutoring_session when its generic turns/attempts shape is sufficient. Reuse the brief's known weakness keys and policy version. Put separate turns and retries in that one request; preserve actual prompts, answers, hints, retry order, effort, exposure and timing facts without inference. Declare only genuinely new weaknesses in new_weaknesses; an incidental discovery stays a candidate when activation is omitted. Keep stylistic suggestions and accepted regional alternatives in findings, feedback or notes; they are not weakness observations. Do not call upserts, taxonomy edits, or confirmation reads before ordinary recording. Only propose supported reviews; zero schedule changes is valid. Inspect the recording response and retry uncertain writes with the identical request and idempotency key. Use record_practice_session for advanced canonical workflows, including activity runs, until compact support covers them. get_learning_context, get_recent_practice and get_review_queue are read-when-needed context tools; do not require all three before recording. Preference reads/updates are for inspection or learner-requested changes. get_taxonomy and get_data_status are occasional diagnostics. Spoken responses are stored as transcripts only; timing and hesitation data must be explicitly reported or externally measured. Never ask for project IDs, database names, tables, schemas, or SQL.");
        info.server_info = Implementation::new("aprendiendo-mcp", env!("CARGO_PKG_VERSION"))
            .with_title("Aprendiendo Español")
            .with_description("Project-scoped Spanish learning history and FSRS weakness tools");
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::MockStore, model::LearningContextRequest};
    #[tokio::test]
    async fn learning_context_uses_default_bound() {
        let server = LearningServer::new(std::sync::Arc::new(MockStore), None);
        let result = server
            .get_learning_context(Parameters(LearningContextRequest {
                recent_sessions: None,
                weakness_limit: None,
                detail: None,
                concept_keys: None,
                scheme_keys: None,
                collection_keys: None,
                target_types: None,
            }))
            .await
            .unwrap();
        assert_eq!(result.0.data["recent_sessions"], 5);
    }
    #[test]
    fn advertises_domain_tools() {
        let server = LearningServer::new(std::sync::Arc::new(MockStore), None);
        let names = server
            .tool_router
            .list_all()
            .into_iter()
            .map(|t| t.name.into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 13);
        assert!(names.contains(&"get_practice_brief".to_owned()));
    }

    #[test]
    fn recording_text_telemetry_is_allowlisted_and_excludes_identifiers() {
        let arguments = serde_json::json!({
            "idempotency_key": "secret-idempotency-sentinel",
            "task_ref": "private-task-ref",
            "turns": [{
                "prompt": "¿Qué hiciste ayer?",
                "attempts": [{"transcript": "Fui al mercado.", "findings": [{
                    "original": "al mercado", "suggestion": "al mercado local"
                }]}]
            }]
        });
        let fields = recording_text_fields("record_tutoring_session", arguments.as_object());
        let serialized = serde_json::to_string(&fields).unwrap();
        assert!(serialized.contains("¿Qué hiciste ayer?"));
        assert!(serialized.contains("Fui al mercado."));
        assert!(!serialized.contains("secret-idempotency-sentinel"));
        assert!(!serialized.contains("private-task-ref"));
        let unknown_metadata = serde_json::json!({"unexpected": {"text": "metadata sentinel"}});
        assert!(
            !serde_json::to_string(&recording_text_fields(
                "record_tutoring_session",
                unknown_metadata.as_object(),
            ))
            .unwrap()
            .contains("metadata sentinel")
        );
        assert!(recording_text_fields("get_recent_practice", arguments.as_object()).is_empty());
    }
}
