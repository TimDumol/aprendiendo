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
    model::{
        DomainResult, LearningContextRequest, ObservationInput, RecentPracticeRequest,
        RecordPracticeSessionRequest, ReviewQueueRequest, UpsertWeaknessRequest,
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
        let security_schemes = match oauth_scope {
            Some(scope) => serde_json::json!([{"type": "oauth2", "scopes": [scope]}]),
            None => serde_json::json!([{"type": "noauth"}]),
        };
        for route in tool_router.map.values_mut() {
            let mut meta = rmcp::model::MetaObject::new();
            meta.insert("securitySchemes".into(), security_schemes.clone());
            route.attr.meta = Some(meta);
        }
        Self { store, tool_router }
    }

    fn validate_bounded_text(value: &str, field: &str, max: usize) -> Result<(), String> {
        if value.trim().is_empty() {
            return Err(format!("{field} must not be empty"));
        }
        if value.len() > max {
            return Err(format!("{field} must be at most {max} bytes"));
        }
        Ok(())
    }

    fn validate_optional_text(value: Option<&str>, field: &str, max: usize) -> Result<(), String> {
        if let Some(value) = value {
            Self::validate_bounded_text(value, field, max)?;
        }
        Ok(())
    }

    fn validate_observation(observation: &ObservationInput) -> Result<(), String> {
        Self::validate_bounded_text(&observation.weakness_key, "weakness_key", 160)?;
        Self::validate_optional_text(observation.produced.as_deref(), "produced", 2_000)?;
        Self::validate_optional_text(observation.correction.as_deref(), "correction", 2_000)?;
        Self::validate_optional_text(observation.notes.as_deref(), "observation.notes", 2_000)
    }

    fn tool_error(operation: &'static str, err: anyhow::Error) -> String {
        error!(operation, error = %err, "learning database operation failed");
        format!("{operation} failed; the server log contains the diagnostic detail")
    }

    fn domain(data: serde_json::Value) -> Json<DomainResult> {
        Json(DomainResult { data })
    }
}

#[tool_router]
impl LearningServer {
    /// Fetch active weaknesses with performance counts and a bounded recent-session summary.
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
        Parameters(request): Parameters<LearningContextRequest>,
    ) -> Result<Json<DomainResult>, String> {
        let recent = request.recent_sessions.unwrap_or(5);
        if !(1..=20).contains(&recent) {
            return Err("recent_sessions must be between 1 and 20".into());
        }
        self.store
            .learning_context(recent.into())
            .await
            .map(Self::domain)
            .map_err(|err| Self::tool_error("get_learning_context", err))
    }

    /// Fetch recent practice sessions with attempts and observations, optionally filtered by skill.
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
        let limit = request.limit.unwrap_or(10);
        if !(1..=50).contains(&limit) {
            return Err("limit must be between 1 and 50".into());
        }
        Self::validate_optional_text(request.skill.as_deref(), "skill", 120)?;
        self.store
            .recent_practice(limit.into(), request.skill.as_deref())
            .await
            .map(Self::domain)
            .map_err(|err| Self::tool_error("get_recent_practice", err))
    }

    /// Record one completed practice session atomically using existing weakness keys.
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
        Self::validate_bounded_text(&request.idempotency_key, "idempotency_key", 128)?;
        Self::validate_bounded_text(&request.exercise_type, "exercise_type", 120)?;
        Self::validate_optional_text(request.session_date.as_deref(), "session_date", 10)?;
        Self::validate_optional_text(request.topic.as_deref(), "topic", 500)?;
        Self::validate_optional_text(request.notes.as_deref(), "notes", 4_000)?;
        if request.attempts.len() > 100 || request.observations.len() > 100 {
            return Err(
                "a session may contain at most 100 attempts and 100 session observations".into(),
            );
        }

        let mut attempt_numbers = HashSet::new();
        let mut observation_count = request.observations.len();
        for attempt in &request.attempts {
            if attempt.attempt_no == 0 || !attempt_numbers.insert(attempt.attempt_no) {
                return Err("attempt_no values must be unique integers of at least 1".into());
            }
            Self::validate_bounded_text(&attempt.transcript, "attempt.transcript", 8_000)?;
            observation_count += attempt.observations.len();
            for observation in &attempt.observations {
                Self::validate_observation(observation)?;
            }
        }
        if observation_count > 300 {
            return Err("a session may contain at most 300 total observations".into());
        }
        for observation in &request.observations {
            Self::validate_observation(observation)?;
        }

        let payload = serde_json::to_value(&request).map_err(|err| err.to_string())?;
        if payload.to_string().len() > 65_536 {
            return Err("serialized practice session must be at most 64 KiB".into());
        }
        self.store
            .record_practice_json(payload)
            .await
            .map(Self::domain)
            .map_err(|err| Self::tool_error("record_practice_session", err))
    }

    /// Fetch active weaknesses prioritized by incorrect observations and error rate.
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
        let limit = request.limit.unwrap_or(20);
        if !(1..=50).contains(&limit) {
            return Err("limit must be between 1 and 50".into());
        }
        Self::validate_optional_text(request.category.as_deref(), "category", 80)?;
        self.store
            .review_queue(limit.into(), request.category.as_deref())
            .await
            .map(Self::domain)
            .map_err(|err| Self::tool_error("get_review_queue", err))
    }

    /// Create a weakness or update its category, description, target pattern, and active state.
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
        Self::validate_bounded_text(&request.key, "key", 160)?;
        Self::validate_bounded_text(&request.category, "category", 80)?;
        Self::validate_bounded_text(&request.description, "description", 1_000)?;
        Self::validate_optional_text(request.target_pattern.as_deref(), "target_pattern", 1_000)?;
        let patch = serde_json::to_value(&request).map_err(|err| err.to_string())?;
        self.store
            .upsert_weakness_json(patch)
            .await
            .map(Self::domain)
            .map_err(|err| Self::tool_error("upsert_weakness", err))
    }

    /// Check adapter version, record counts, and freshness without exposing database schema.
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
            .map_err(|err| Self::tool_error("get_data_status", err))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LearningServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Use these tools only for the configured single-learner Spanish database. Start with get_learning_context. Use weakness keys returned by context or review tools when recording observations; create a new key with upsert_weakness first. Record a session once with a stable idempotency_key. Never ask for project IDs, database names, tables, schemas, or SQL.",
            );
        info.server_info = Implementation::new("aprendiendo-mcp", env!("CARGO_PKG_VERSION"))
            .with_title("Aprendiendo Español")
            .with_description("Project-scoped Spanish learning history and weakness tools");
        info
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{db::MockStore, model::LearningContextRequest};

    use super::*;

    #[tokio::test]
    async fn learning_context_uses_default_bound() {
        let server = LearningServer::new(Arc::new(MockStore), None);
        let result = server
            .get_learning_context(Parameters(LearningContextRequest {
                recent_sessions: None,
            }))
            .await
            .expect("tool should succeed");
        assert_eq!(result.0.data["recent_sessions"], 5);
    }

    #[test]
    fn advertises_only_domain_tools() {
        let server = LearningServer::new(Arc::new(MockStore), None);
        let names = server
            .tool_router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 6);
        assert!(names.contains(&"upsert_weakness".to_owned()));
        assert!(
            !names
                .iter()
                .any(|name| name.contains("sql") || name.contains("schema"))
        );
    }
}
