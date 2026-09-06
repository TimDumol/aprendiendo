use rmcp::{
    Json, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use tracing::error;

use crate::{
    activities,
    db::SharedStore,
    model::{
        DomainResult, LearningContextRequest, PracticeBriefRequest, RecentPracticeRequest,
        RecordPracticeSessionRequest, RecordPracticeSessionResponse, ReviewQueueRequest,
        TaxonomyRequest, UpsertConceptRequest, UpsertWeaknessRequest,
    },
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

#[derive(Clone)]
pub struct LearningServer {
    store: SharedStore,
    tool_router: ToolRouter<Self>,
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
            route.attr.input_schema = std::sync::Arc::new(expanded.as_object().unwrap().clone());
        }
        Self { store, tool_router }
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
                format!("invalid_argument: {message}")
            }
            crate::db::RecordPracticeError::UnknownReference(message) => {
                format!("unknown_reference: {message}")
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
}

#[tool_router]
impl LearningServer {
    /// Return durable learner preferences without loading historical sessions.
    #[tool(
        name = "get_practice_preferences",
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
        let n = request.limit.unwrap_or(10);
        if !(1..=50).contains(&n) {
            return Err("limit must be between 1 and 50".into());
        }
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
        description = "Atomically record one completed practice session (contract_version=2). Supply production_evidence with actual prompts linked through items, separate attempts, observation assessments and interventions. Valid evidence is saved even when reviews are skipped; inspect review_decisions. Unknown cueing cannot support review. Two materially varied independent observations and supported rating evidence are required. Use one exact canonical exercise_type_key and session-wide observation numbers. Retrying an identical request returns status=replayed; a changed request with the same idempotency_key conflicts.",
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
        name = "get_review_queue",
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
        annotations(
            title = "Create or update taxonomy concept",
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
        annotations(
            title = "Create or update weakness",
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

#[tool_handler(router=self.tool_router)]
impl ServerHandler for LearningServer {
    fn get_info(&self) -> ServerInfo {
        let mut info=ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions("Use these tools only for the configured single-learner Spanish database. Start with get_practice_brief and honor its effective preferences. Keep target wording private. Validate each communicative prompt with validate_practice_plan and perform the requested semantic review before delivering one turn; adapt follow-ups after the learner responds. Accept alternative wording: target not observed is not a failure. Finish the short initial written round before correction, quote the original with an indirect hint, then show original and correction after an unsuccessful retry. Record hints and retries separately; inspect review decisions. Never add drills merely to obtain a rating. ChatGPT generates and presents exercise language; record each learner-facing prompt or turn separately. Spoken responses are stored as transcripts only: never infer audio properties or timing from transcript text. Timing and hesitation data must be explicitly learner-reported or externally measured. Record deliberate FSRS reviews under the existing evidence rules. Never ask for project IDs, database names, tables, schemas, or SQL.");
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
        assert_eq!(names.len(), 12);
        assert!(names.contains(&"get_practice_brief".to_owned()));
    }
}
