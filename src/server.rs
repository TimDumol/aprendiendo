use std::collections::HashSet;

use rmcp::{
    Json, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use tracing::error;

use crate::{
    db::SharedStore,
    evidence,
    model::{
        DomainResult, LearningContextRequest, ObservationInput, PracticeBriefRequest,
        RecentPracticeRequest, RecordPracticeSessionRequest, ReviewQueueRequest, TaxonomyRequest,
        UpsertConceptRequest, UpsertWeaknessRequest,
    },
};

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
    fn observation(o: &ObservationInput) -> Result<(), String> {
        if o.observation_no.is_none() {
            return Err("observation_no is required and must be unique across the session".into());
        }
        Self::text(&o.weakness_key, "weakness_key", 160)?;
        Self::optional(o.produced.as_deref(), "produced", 2_000)?;
        Self::optional(o.correction.as_deref(), "correction", 2_000)?;
        Self::optional(o.error_span.as_deref(), "error_span", 1_000)?;
        Self::optional(o.notes.as_deref(), "observation.notes", 2_000)
    }
    fn operation_error(operation: &'static str, err: anyhow::Error) -> String {
        error!(operation,error=%err,"learning database operation failed");
        format!("{operation} failed; the server log contains the diagnostic detail")
    }
    fn domain(data: serde_json::Value) -> Json<DomainResult> {
        Json(DomainResult { data })
    }
}

#[tool_router]
impl LearningServer {
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
        if !(1..=20).contains(&n) {
            return Err("recent_sessions must be between 1 and 20".into());
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
        Self::list(request.exercise_types.as_ref(), "exercise_types", 12)?;
        Self::list(request.drill_types.as_ref(), "drill_types", 12)?;
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
        for v in request.exercise_types.iter().flatten() {
            Self::text(v, "exercise_type", 120)?;
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
        annotations(
            title = "Record practice session",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn record_practice_session(
        &self,
        Parameters(request): Parameters<RecordPracticeSessionRequest>,
    ) -> Result<Json<DomainResult>, String> {
        Self::text(&request.idempotency_key, "idempotency_key", 128)?;
        Self::text(&request.exercise_type, "exercise_type", 120)?;
        Self::optional(request.session_date.as_deref(), "session_date", 10)?;
        Self::optional(request.reviewed_at.as_deref(), "reviewed_at", 64)?;
        Self::optional(request.topic.as_deref(), "topic", 500)?;
        Self::optional(request.notes.as_deref(), "notes", 4_000)?;
        if request.items.len() > 100
            || request.attempts.len() > 100
            || request.new_weaknesses.len() > 100
            || request.reviews.len() > 100
        {
            return Err(
                "a session may contain at most 100 items, attempts, weaknesses, and reviews".into(),
            );
        }
        let mut item_n = HashSet::new();
        let mut attempts_n = HashSet::new();
        let mut observations = request.observations.len();
        for w in &request.new_weaknesses {
            Self::text(&w.key, "new_weakness.key", 160)?;
            Self::text(&w.category, "new_weakness.category", 80)?;
            Self::text(&w.description, "new_weakness.description", 1_000)?;
            Self::optional(
                w.target_pattern.as_deref(),
                "new_weakness.target_pattern",
                1_000,
            )?;
            Self::list(
                w.recommended_drill_types.as_ref(),
                "recommended_drill_types",
                12,
            )?;
            Self::list(Some(&w.concept_links), "concept_links", 20)?;
            Self::list(Some(&w.target_relations), "target_relations", 20)?;
            for link in &w.concept_links {
                Self::text(&link.concept_key, "concept_link.concept_key", 160)?;
            }
            for relation in &w.target_relations {
                Self::text(
                    &relation.other_weakness_key,
                    "target_relation.other_weakness_key",
                    160,
                )?;
                Self::text(&relation.predicate, "target_relation.predicate", 40)?;
                Self::optional(relation.notes.as_deref(), "target_relation.notes", 2_000)?;
            }
        }
        for item in &request.items {
            if item.item_no == 0 || !item_n.insert(item.item_no) {
                return Err("item_no values must be unique integers of at least 1".into());
            }
            Self::text(&item.prompt, "item.prompt", 4_000)?;
            Self::optional(item.response.as_deref(), "item.response", 8_000)?;
            Self::optional(
                item.corrected_response.as_deref(),
                "item.corrected_response",
                8_000,
            )?;
            Self::optional(
                item.reference_answer.as_deref(),
                "item.reference_answer",
                8_000,
            )?;
            Self::optional(item.feedback.as_deref(), "item.feedback", 4_000)?;
            Self::list(Some(&item.target_weakness_keys), "target_weakness_keys", 20)?;
            for key in &item.target_weakness_keys {
                Self::text(key, "target_weakness_key", 160)?;
            }
            observations += item.observations.len();
            for o in &item.observations {
                Self::observation(o)?;
            }
        }
        for a in &request.attempts {
            if a.attempt_no == 0 || !attempts_n.insert(a.attempt_no) {
                return Err("attempt_no values must be unique integers of at least 1".into());
            }
            if a.practice_item_no.is_some_and(|n| !item_n.contains(&n)) {
                return Err("attempt references an unknown practice item".into());
            }
            Self::text(&a.transcript, "attempt.transcript", 8_000)?;
            if a.target_duration_seconds.is_some_and(|n| n == 0) {
                return Err("target_duration_seconds must be positive".into());
            }
            observations += a.observations.len();
            for o in &a.observations {
                Self::observation(o)?;
            }
        }
        for o in &request.observations {
            Self::observation(o)?;
        }
        evidence::validate_session_observation_numbers(&request).map_err(|e| e.to_string())?;
        if observations > 300 {
            return Err("a session may contain at most 300 observations".into());
        }
        for r in &request.reviews {
            Self::text(&r.weakness_key, "review.weakness_key", 160)?;
            if !r.evidence.is_null() && !r.evidence.is_object() {
                return Err("review evidence must be a JSON object".into());
            }
            if serde_json::to_string(&r.evidence)
                .map(|s| s.len() > 4_096)
                .unwrap_or(true)
            {
                return Err("review evidence must be at most 4096 bytes".into());
            }
        }
        let payload = serde_json::to_value(&request).map_err(|e| e.to_string())?;
        if serde_json::to_vec(&payload)
            .map(|x| x.len() > 65_536)
            .unwrap_or(true)
        {
            return Err("serialized practice session must be at most 64 KiB".into());
        }
        self.store
            .record_practice_json(payload)
            .await
            .map(Self::domain)
            .map_err(|e| Self::operation_error("record_practice_session", e))
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
        let mut info=ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions("Use these tools only for the configured single-learner Spanish database. Start with get_learning_context or get_practice_brief. Let ChatGPT generate exercise language, record each item separately, and supply one explicit FSRS rating per deliberately reviewed weakness. Never ask for project IDs, database names, tables, schemas, or SQL.");
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
        assert_eq!(names.len(), 9);
        assert!(names.contains(&"get_practice_brief".to_owned()));
    }
}
