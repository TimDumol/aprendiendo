use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LearningContextRequest {
    /// Number of recent sessions to include. Range: 1-20.
    pub recent_sessions: Option<u16>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecentPracticeRequest {
    /// Maximum number of sessions to return. Range: 1-50.
    pub limit: Option<u16>,
    /// Optional exercise type, topic, or weakness-category filter.
    pub skill: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewQueueRequest {
    /// Maximum number of active weaknesses to return. Range: 1-50.
    pub limit: Option<u16>,
    /// Optional weakness category filter.
    pub category: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ObservationOutcome {
    Incorrect,
    Correct,
    PromptedCorrect,
    Omitted,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationInput {
    /// Existing weakness key from the learning catalog.
    pub weakness_key: String,
    pub outcome: ObservationOutcome,
    /// What the learner produced, when useful for later review.
    pub produced: Option<String>,
    /// Corrected Spanish, when the outcome was not fully correct.
    pub correction: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttemptInput {
    /// Attempt number within the session; must be unique and at least 1.
    pub attempt_no: u16,
    /// Learner transcript or written answer.
    pub transcript: String,
    #[serde(default)]
    pub observations: Vec<ObservationInput>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordPracticeSessionRequest {
    /// Caller-generated key used to make retries idempotent.
    pub idempotency_key: String,
    /// Session date in YYYY-MM-DD form; defaults to the database's current date.
    pub session_date: Option<String>,
    /// Domain-specific type such as "4-3-2" or "production drill".
    pub exercise_type: String,
    pub topic: Option<String>,
    pub notes: Option<String>,
    #[serde(default)]
    pub attempts: Vec<AttemptInput>,
    /// Session-level observations that are not tied to one attempt.
    #[serde(default)]
    pub observations: Vec<ObservationInput>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpsertWeaknessRequest {
    /// Stable machine-readable weakness key.
    pub key: String,
    pub category: String,
    pub description: String,
    pub target_pattern: Option<String>,
    /// Whether the weakness should remain in the active review queue.
    pub active: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DomainResult {
    /// Domain-shaped JSON returned by the database adapter function.
    pub data: Value,
}
