use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LearningContextRequest {
    /// Number of recent sessions to include. Range: 1-20.
    pub recent_sessions: Option<u16>,
    /// Maximum number of active weaknesses to include. Range: 1-50.
    pub weakness_limit: Option<u16>,
    /// Output detail. Defaults to summary.
    pub detail: Option<DetailMode>,
    pub concept_keys: Option<Vec<String>>,
    pub scheme_keys: Option<Vec<String>>,
    pub collection_keys: Option<Vec<String>>,
    pub target_types: Option<Vec<TargetType>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaxonomyRequest {
    pub scheme_keys: Option<Vec<String>>,
    pub root_concept_keys: Option<Vec<String>>,
    pub include_descendants: Option<bool>,
    pub depth: Option<u8>,
    pub include_targets: Option<bool>,
    pub include_collections: Option<bool>,
    pub limit: Option<u16>,
    pub compact: Option<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DetailMode {
    Summary,
    Full,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecentPracticeRequest {
    pub limit: Option<u16>,
    pub skill: Option<String>,
    pub exercise_types: Option<Vec<String>>,
    pub drill_types: Option<Vec<DrillType>>,
    pub weakness_keys: Option<Vec<String>>,
    pub categories: Option<Vec<String>>,
    pub concept_keys: Option<Vec<String>>,
    pub scheme_keys: Option<Vec<String>>,
    pub collection_keys: Option<Vec<String>>,
    pub target_types: Option<Vec<TargetType>>,
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    pub detail: Option<DetailMode>,
    pub include_items: Option<bool>,
    pub include_attempts: Option<bool>,
    pub include_observations: Option<bool>,
    pub include_reviews: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewQueueRequest {
    pub limit: Option<u16>,
    pub category: Option<String>,
    pub as_of: Option<String>,
    pub include_upcoming: Option<bool>,
    pub concept_keys: Option<Vec<String>>,
    pub scheme_keys: Option<Vec<String>>,
    pub collection_keys: Option<Vec<String>>,
    pub target_types: Option<Vec<TargetType>>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationOutcome {
    Incorrect,
    PartiallyCorrect,
    Correct,
    PromptedCorrect,
    Omitted,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationRole {
    Targeted,
    Incidental,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStrength {
    Recognition,
    CuedProduction,
    ControlledProduction,
    SpontaneousProduction,
}

impl EvidenceStrength {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recognition => "recognition",
            Self::CuedProduction => "cued_production",
            Self::ControlledProduction => "controlled_production",
            Self::SpontaneousProduction => "spontaneous_production",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSeverity {
    Minor,
    MeaningAffecting,
    Blocking,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentPhase {
    Historical,
    ColdRetrieval,
    GuidedPractice,
    ImmediateRetry,
    Transfer,
    Incidental,
}

impl AssessmentPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Historical => "historical",
            Self::ColdRetrieval => "cold_retrieval",
            Self::GuidedPractice => "guided_practice",
            Self::ImmediateRetry => "immediate_retry",
            Self::Transfer => "transfer",
            Self::Incidental => "incidental",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HintLevel {
    None,
    Indirect,
    Direct,
    AnswerShown,
}

impl HintLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Indirect => "indirect",
            Self::Direct => "direct",
            Self::AnswerShown => "answer_shown",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LearnerEffort {
    Effortless,
    SomeEffort,
    SubstantialEffort,
    Unknown,
}

impl LearnerEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Effortless => "effortless",
            Self::SomeEffort => "some_effort",
            Self::SubstantialEffort => "substantial_effort",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    Learner,
    Assistant,
    Legacy,
}

impl EvidenceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Learner => "learner",
            Self::Assistant => "assistant",
            Self::Legacy => "legacy",
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationInput {
    pub observation_no: Option<u16>,
    pub weakness_key: String,
    pub outcome: ObservationOutcome,
    #[serde(default = "default_incidental")]
    pub role: ObservationRole,
    pub assessment_phase: Option<AssessmentPhase>,
    pub hint_level: Option<HintLevel>,
    pub learner_effort: Option<LearnerEffort>,
    pub evidence_source: Option<EvidenceSource>,
    pub evidence_strength: Option<EvidenceStrength>,
    pub severity: Option<ObservationSeverity>,
    pub produced: Option<String>,
    pub correction: Option<String>,
    pub error_span: Option<String>,
    pub notes: Option<String>,
}

fn default_incidental() -> ObservationRole {
    ObservationRole::Incidental
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DrillType {
    Translation,
    SituationalResponse,
    SentenceTransformation,
    QuestionAnswer,
    SentenceCompletion,
    SentenceCombining,
    ErrorCorrection,
    MinimalPairChoice,
    DialogueCompletion,
    MicroStory,
    Retell,
    #[serde(rename = "fluency_4_3_2")]
    Fluency432,
}

impl DrillType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Translation => "translation",
            Self::SituationalResponse => "situational_response",
            Self::SentenceTransformation => "sentence_transformation",
            Self::QuestionAnswer => "question_answer",
            Self::SentenceCompletion => "sentence_completion",
            Self::SentenceCombining => "sentence_combining",
            Self::ErrorCorrection => "error_correction",
            Self::MinimalPairChoice => "minimal_pair_choice",
            Self::DialogueCompletion => "dialogue_completion",
            Self::MicroStory => "micro_story",
            Self::Retell => "retell",
            Self::Fluency432 => "fluency_4_3_2",
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttemptInput {
    pub attempt_no: u16,
    pub practice_item_no: Option<u16>,
    pub transcript: String,
    pub target_duration_seconds: Option<u32>,
    pub actual_duration_milliseconds: Option<u64>,
    #[serde(default)]
    pub observations: Vec<ObservationInput>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PracticeItemInput {
    pub item_no: u16,
    pub drill_type: DrillType,
    pub prompt: String,
    pub response: Option<String>,
    pub corrected_response: Option<String>,
    pub reference_answer: Option<String>,
    pub feedback: Option<String>,
    pub outcome: Option<PracticeItemOutcome>,
    #[serde(default)]
    pub target_weakness_keys: Vec<String>,
    #[serde(default)]
    pub observations: Vec<ObservationInput>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PracticeItemOutcome {
    Incorrect,
    PartiallyCorrect,
    Correct,
    Omitted,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewWeaknessInput {
    pub key: String,
    pub category: String,
    pub description: String,
    pub target_pattern: Option<String>,
    pub recommended_drill_types: Option<Vec<DrillType>>,
    pub active: Option<bool>,
    pub target_type: Option<TargetType>,
    pub primary_concept_key: Option<String>,
    #[serde(default)]
    pub concept_links: Vec<ConceptLinkInput>,
    #[serde(default)]
    pub target_relations: Vec<TargetRelationInput>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpsertWeaknessRequest {
    pub key: String,
    pub category: String,
    pub description: String,
    pub target_pattern: Option<String>,
    pub recommended_drill_types: Option<Vec<DrillType>>,
    pub active: Option<bool>,
    pub target_type: Option<TargetType>,
    pub primary_concept_key: Option<String>,
    #[serde(default)]
    pub concept_links: Vec<ConceptLinkInput>,
    #[serde(default)]
    pub target_relations: Vec<TargetRelationInput>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetType {
    GrammaticalConstruction,
    LexicalItem,
    LexicalChunk,
    FormMeaningContrast,
    Pronunciation,
    Orthography,
    DiscourseStrategy,
    SociopragmaticChoice,
}

impl TargetType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GrammaticalConstruction => "grammatical_construction",
            Self::LexicalItem => "lexical_item",
            Self::LexicalChunk => "lexical_chunk",
            Self::FormMeaningContrast => "form_meaning_contrast",
            Self::Pronunciation => "pronunciation",
            Self::Orthography => "orthography",
            Self::DiscourseStrategy => "discourse_strategy",
            Self::SociopragmaticChoice => "sociopragmatic_choice",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetStatus {
    Candidate,
    Active,
    Suspended,
    Retired,
    Merged,
}

impl TargetStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Retired => "retired",
            Self::Merged => "merged",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConceptLinkRole {
    Form,
    Meaning,
    Function,
    Context,
    ErrorSource,
    Curriculum,
}

impl ConceptLinkRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Form => "form",
            Self::Meaning => "meaning",
            Self::Function => "function",
            Self::Context => "context",
            Self::ErrorSource => "error_source",
            Self::Curriculum => "curriculum",
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct ConceptLinkInput {
    pub concept_key: String,
    pub role: ConceptLinkRole,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TargetRelationInput {
    pub other_weakness_key: String,
    pub predicate: String,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct ConceptEdgeInput {
    pub other_concept_key: String,
    pub predicate: String,
    pub provenance: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpsertConceptRequest {
    pub key: String,
    pub scheme_key: String,
    pub label: String,
    pub definition: Option<String>,
    pub source_uri: Option<String>,
    pub broader_concept_keys: Option<Vec<String>>,
    #[serde(default)]
    pub edges: Vec<ConceptEdgeInput>,
    pub active: Option<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FsrsRating {
    Again,
    Hard,
    Good,
    Easy,
}

impl FsrsRating {
    pub fn number(self) -> i32 {
        match self {
            Self::Again => 1,
            Self::Hard => 2,
            Self::Good => 3,
            Self::Easy => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalMode {
    Recognition,
    CuedProduction,
    ControlledProduction,
    SpontaneousProduction,
}

impl RetrievalMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recognition => "recognition",
            Self::CuedProduction => "cued_production",
            Self::ControlledProduction => "controlled_production",
            Self::SpontaneousProduction => "spontaneous_production",
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewInput {
    pub weakness_key: String,
    pub rating: FsrsRating,
    pub retrieval_mode: RetrievalMode,
    pub evidence_strength: EvidenceStrength,
    #[serde(default)]
    pub evidence_observation_nos: Vec<u16>,
    pub rating_rationale: Option<String>,
    #[serde(default = "default_rating_source")]
    pub rating_source: RatingSource,
    #[serde(default)]
    pub evidence: Value,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RatingSource {
    Learner,
    AssistantSuggestedConfirmed,
    AssistantSuggested,
}

impl RatingSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Learner => "learner",
            Self::AssistantSuggestedConfirmed => "assistant_suggested_confirmed",
            Self::AssistantSuggested => "assistant_suggested",
        }
    }
}

fn default_rating_source() -> RatingSource {
    RatingSource::AssistantSuggested
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PracticeObjective {
    ReviewDue,
    Targeted,
    Explore,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DrillMix {
    Auto,
    ProductionFocused,
    TranslationOnly,
    RecognitionToProduction,
    Fluency,
    Custom,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PracticeBriefRequest {
    pub count: Option<u16>,
    pub objective: Option<PracticeObjective>,
    pub level: Option<String>,
    pub drill_mix: Option<DrillMix>,
    pub allowed_drill_types: Option<Vec<DrillType>>,
    pub weakness_keys: Option<Vec<String>>,
    pub categories: Option<Vec<String>>,
    pub concept_keys: Option<Vec<String>>,
    pub scheme_keys: Option<Vec<String>>,
    pub collection_keys: Option<Vec<String>>,
    pub target_types: Option<Vec<TargetType>>,
    pub include_upcoming: Option<bool>,
    pub recent_prompts_per_weakness: Option<u16>,
    pub as_of: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordPracticeSessionRequest {
    pub idempotency_key: String,
    pub session_date: Option<String>,
    pub reviewed_at: Option<String>,
    pub exercise_type: String,
    pub exercise_type_key: Option<String>,
    pub topic: Option<String>,
    pub notes: Option<String>,
    #[serde(default)]
    pub new_weaknesses: Vec<NewWeaknessInput>,
    #[serde(default)]
    pub items: Vec<PracticeItemInput>,
    #[serde(default)]
    pub attempts: Vec<AttemptInput>,
    #[serde(default)]
    pub observations: Vec<ObservationInput>,
    #[serde(default)]
    pub reviews: Vec<ReviewInput>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DomainResult {
    pub data: Value,
}
