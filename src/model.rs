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
    /// Canonical exercise type keys to include.
    #[schemars(length(max = 6))]
    pub exercise_type_keys: Option<Vec<ExerciseTypeKey>>,
    pub drill_types: Option<Vec<DrillType>>,
    pub weakness_keys: Option<Vec<String>>,
    pub categories: Option<Vec<String>>,
    pub concept_keys: Option<Vec<String>>,
    pub scheme_keys: Option<Vec<String>>,
    pub collection_keys: Option<Vec<String>>,
    pub target_types: Option<Vec<TargetType>>,
    pub activity_types: Option<Vec<ActivityType>>,
    pub response_modes: Option<Vec<ResponseMode>>,
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    pub detail: Option<DetailMode>,
    pub include_items: Option<bool>,
    pub include_attempts: Option<bool>,
    pub include_observations: Option<bool>,
    pub include_reviews: Option<bool>,
}

/// The canonical catalog of exercise types accepted by recorded sessions.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ExerciseTypeKey {
    #[serde(rename = "fluency_4_3_2")]
    Fluency432,
    ProductionDrill,
    TranslationDrill,
    DeleA2OralMicrodrill,
    AgreementDisagreementDrill,
    GuidedConversation,
}

impl ExerciseTypeKey {
    pub const ALL: [Self; 6] = [
        Self::Fluency432,
        Self::ProductionDrill,
        Self::TranslationDrill,
        Self::DeleA2OralMicrodrill,
        Self::AgreementDisagreementDrill,
        Self::GuidedConversation,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fluency432 => "fluency_4_3_2",
            Self::ProductionDrill => "production_drill",
            Self::TranslationDrill => "translation_drill",
            Self::DeleA2OralMicrodrill => "dele_a2_oral_microdrill",
            Self::AgreementDisagreementDrill => "agreement_disagreement_drill",
            Self::GuidedConversation => "guided_conversation",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fluency432 => "4-3-2",
            Self::ProductionDrill => "production drill",
            Self::TranslationDrill => "translation drill",
            Self::DeleA2OralMicrodrill => "DELE A2 oral microdrill",
            Self::AgreementDisagreementDrill => "agreement/disagreement drill",
            Self::GuidedConversation => "guided conversation",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "fluency_4_3_2" => Some(Self::Fluency432),
            "production_drill" => Some(Self::ProductionDrill),
            "translation_drill" => Some(Self::TranslationDrill),
            "dele_a2_oral_microdrill" => Some(Self::DeleA2OralMicrodrill),
            "agreement_disagreement_drill" => Some(Self::AgreementDisagreementDrill),
            "guided_conversation" => Some(Self::GuidedConversation),
            _ => None,
        }
    }
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
    /// Positive number unique across all observations in this request; reviews refer to it.
    #[schemars(range(min = 1))]
    pub observation_no: u16,
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ActivityType {
    SituationalResponse,
    PictureNarration,
    QuestionAnswerSprint,
    RetellReconstruction,
    CorrectiveConversation,
    SentenceTransformationSprint,
    Dictogloss,
    VoiceDiary,
    ReadCloseExplain,
    RolePlayComplications,
}

impl ActivityType {
    pub const ALL: [Self; 10] = [
        Self::SituationalResponse,
        Self::PictureNarration,
        Self::QuestionAnswerSprint,
        Self::RetellReconstruction,
        Self::CorrectiveConversation,
        Self::SentenceTransformationSprint,
        Self::Dictogloss,
        Self::VoiceDiary,
        Self::ReadCloseExplain,
        Self::RolePlayComplications,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SituationalResponse => "situational_response",
            Self::PictureNarration => "picture_narration",
            Self::QuestionAnswerSprint => "question_answer_sprint",
            Self::RetellReconstruction => "retell_reconstruction",
            Self::CorrectiveConversation => "corrective_conversation",
            Self::SentenceTransformationSprint => "sentence_transformation_sprint",
            Self::Dictogloss => "dictogloss",
            Self::VoiceDiary => "voice_diary",
            Self::ReadCloseExplain => "read_close_explain",
            Self::RolePlayComplications => "role_play_complications",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityInteractionMode {
    SingleResponse,
    Sprint,
    MultiTurn,
}

impl ActivityInteractionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SingleResponse => "single_response",
            Self::Sprint => "sprint",
            Self::MultiTurn => "multi_turn",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityItemPhase {
    Initial,
    FollowUp,
    Complication,
}

impl ActivityItemPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::FollowUp => "follow_up",
            Self::Complication => "complication",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StimulusKind {
    Situation,
    SourceText,
    ImageDescription,
    ImageSequenceDescription,
    ArticleReference,
    MediaTranscript,
    Complication,
}

impl StimulusKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Situation => "situation",
            Self::SourceText => "source_text",
            Self::ImageDescription => "image_description",
            Self::ImageSequenceDescription => "image_sequence_description",
            Self::ArticleReference => "article_reference",
            Self::MediaTranscript => "media_transcript",
            Self::Complication => "complication",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StimulusDeliveryMode {
    Read,
    Viewed,
    HeardReported,
    Conversation,
}

impl StimulusDeliveryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Viewed => "viewed",
            Self::HeardReported => "heard_reported",
            Self::Conversation => "conversation",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseMode {
    Typed,
    SpokenTranscript,
}

impl ResponseMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Typed => "typed",
            Self::SpokenTranscript => "spoken_transcript",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimingSource {
    LearnerReported,
    ExternalTimer,
    ClientMeasured,
}

impl TimingSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LearnerReported => "learner_reported",
            Self::ExternalTimer => "external_timer",
            Self::ClientMeasured => "client_measured",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionSource {
    Learner,
    Assistant,
}

impl ReflectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Learner => "learner",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionKind {
    HesitationReported,
    SimplificationReported,
    RetrievalGapReported,
    SelfCorrectionReported,
    CircumlocutionReported,
    General,
}

impl ReflectionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HesitationReported => "hesitation_reported",
            Self::SimplificationReported => "simplification_reported",
            Self::RetrievalGapReported => "retrieval_gap_reported",
            Self::SelfCorrectionReported => "self_correction_reported",
            Self::CircumlocutionReported => "circumlocution_reported",
            Self::General => "general",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivityConfigInput {
    #[schemars(range(min = 1, max = 3600))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preparation_seconds: Option<u32>,
    #[schemars(range(min = 1, max = 3600))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_seconds: Option<u32>,
    #[schemars(range(min = 1, max = 20))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question_count: Option<u16>,
    #[schemars(range(min = 1, max = 10))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure_count: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_hidden_before_response: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unpredictable_followups: Option<bool>,
    #[schemars(range(min = 0, max = 10))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complication_count: Option<u8>,
}

impl ActivityConfigInput {
    pub fn is_empty(&self) -> bool {
        self.preparation_seconds.is_none()
            && self.response_seconds.is_none()
            && self.question_count.is_none()
            && self.exposure_count.is_none()
            && self.source_hidden_before_response.is_none()
            && self.unpredictable_followups.is_none()
            && self.complication_count.is_none()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivityStimulusInput {
    #[schemars(range(min = 1))]
    pub stimulus_no: u16,
    pub kind: StimulusKind,
    pub delivery_mode: StimulusDeliveryMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivityRunInput {
    #[schemars(range(min = 1))]
    pub run_no: u16,
    pub activity_type: ActivityType,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 7200))]
    pub planned_duration_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 86400000))]
    pub actual_duration_milliseconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_source: Option<TimingSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<ActivityConfigInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 20))]
    pub stimuli: Vec<ActivityStimulusInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttemptReflectionInput {
    #[schemars(range(min = 1))]
    pub reflection_no: u16,
    pub source: ReflectionSource,
    pub kind: ReflectionKind,
    pub note: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttemptInput {
    #[schemars(range(min = 1))]
    pub attempt_no: u16,
    #[schemars(range(min = 1))]
    pub practice_item_no: Option<u16>,
    pub transcript: String,
    #[schemars(range(min = 1, max = 7200))]
    pub target_duration_seconds: Option<u32>,
    #[schemars(range(min = 1, max = 86400000))]
    pub actual_duration_milliseconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_mode: Option<ResponseMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 3600000))]
    pub response_latency_milliseconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_source: Option<TimingSource>,
    #[serde(default)]
    #[schemars(length(max = 300))]
    pub observations: Vec<ObservationInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 20))]
    pub reflections: Vec<AttemptReflectionInput>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PracticeItemInput {
    #[schemars(range(min = 1))]
    pub item_no: u16,
    pub drill_type: DrillType,
    pub prompt: String,
    pub response: Option<String>,
    pub corrected_response: Option<String>,
    pub reference_answer: Option<String>,
    pub feedback: Option<String>,
    pub outcome: Option<PracticeItemOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub activity_run_no: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_phase: Option<ActivityItemPhase>,
    #[serde(default)]
    #[schemars(length(max = 20))]
    pub target_weakness_keys: Vec<String>,
    #[serde(default)]
    #[schemars(length(max = 300))]
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
    /// Non-empty observation numbers from this request used as review evidence.
    #[schemars(length(min = 1, max = 300), inner(range(min = 1)))]
    #[serde(default)]
    pub evidence_observation_nos: Vec<u16>,
    pub rating_rationale: Option<String>,
    #[serde(default = "default_rating_source")]
    pub rating_source: RatingSource,
    /// JSON object containing review evidence; serialized size is limited at runtime.
    #[serde(default)]
    pub evidence: serde_json::Map<String, Value>,
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
    pub activity_type: Option<ActivityType>,
    pub planned_duration_seconds: Option<u32>,
    pub activity_config: Option<ActivityConfigInput>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    Created,
    Replayed,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReviewUpdate {
    pub weakness_key: String,
    pub rating: String,
    pub retrievability_before: Option<f64>,
    pub stability_before: Option<f64>,
    pub stability_after: f64,
    pub difficulty_before: Option<f64>,
    pub difficulty_after: f64,
    pub scheduled_interval_days: i32,
    pub due_learning_day: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RecordPracticeSessionResponse {
    pub session_id: i64,
    pub status: RecordStatus,
    pub exercise_type_key: ExerciseTypeKey,
    pub exercise_type_label: String,
    pub item_count: usize,
    pub attempt_count: usize,
    pub observation_count: usize,
    pub activity_run_count: usize,
    pub stimulus_count: usize,
    pub reflection_count: usize,
    pub new_weaknesses_created: Vec<String>,
    pub review_updates: Vec<ReviewUpdate>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordPracticeSessionRequest {
    /// Required nonblank key. Identical canonical requests replay; max 128 UTF-8 bytes.
    #[schemars(length(min = 1))]
    pub idempotency_key: String,
    /// YYYY-MM-DD; omitted values derive from reviewed_at or the current learning day.
    #[schemars(regex(pattern = r"^\d{4}-\d{2}-\d{2}$"))]
    pub session_date: Option<String>,
    /// RFC 3339 timestamp used to derive the learning day when supplied.
    #[schemars(regex(pattern = r"^\d{4}-\d{2}-\d{2}T"))]
    pub reviewed_at: Option<String>,
    /// One of the six canonical exercise type keys.
    pub exercise_type_key: ExerciseTypeKey,
    /// Optional nonblank topic; max 500 UTF-8 bytes.
    #[schemars(length(min = 1))]
    pub topic: Option<String>,
    /// Optional nonblank notes; max 4000 UTF-8 bytes.
    #[schemars(length(min = 1))]
    pub notes: Option<String>,
    #[schemars(length(max = 100))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub new_weaknesses: Vec<NewWeaknessInput>,
    #[schemars(length(max = 100))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<PracticeItemInput>,
    #[schemars(length(max = 100))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<AttemptInput>,
    #[schemars(length(max = 300))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observations: Vec<ObservationInput>,
    #[schemars(length(max = 100))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviews: Vec<ReviewInput>,
    #[schemars(length(max = 10))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity_runs: Vec<ActivityRunInput>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DomainResult<T = Value> {
    pub data: T,
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::schema_for;
    use serde_json::json;

    #[test]
    fn exercise_type_catalog_round_trips_in_schema_order() {
        let expected = [
            ("fluency_4_3_2", "4-3-2"),
            ("production_drill", "production drill"),
            ("translation_drill", "translation drill"),
            ("dele_a2_oral_microdrill", "DELE A2 oral microdrill"),
            (
                "agreement_disagreement_drill",
                "agreement/disagreement drill",
            ),
            ("guided_conversation", "guided conversation"),
        ];
        let actual = ExerciseTypeKey::ALL
            .into_iter()
            .map(|key| {
                let encoded = serde_json::to_string(&key).unwrap();
                let decoded: ExerciseTypeKey = serde_json::from_str(&encoded).unwrap();
                assert_eq!(decoded, key);
                (key.as_str(), key.label())
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn record_request_rejects_legacy_fields_and_non_object_evidence() {
        let base = json!({
            "idempotency_key": "model-test",
            "exercise_type_key": "translation_drill"
        });
        let mut legacy = base.clone();
        legacy["exercise_type"] = json!("translation_drill");
        assert!(serde_json::from_value::<RecordPracticeSessionRequest>(legacy).is_err());

        let evidence = json!({
            "idempotency_key": "model-test",
            "exercise_type_key": "translation_drill",
            "reviews": [{
                "weakness_key": "x",
                "rating": "good",
                "retrieval_mode": "controlled_production",
                "evidence_strength": "controlled_production",
                "evidence": []
            }]
        });
        assert!(serde_json::from_value::<RecordPracticeSessionRequest>(evidence).is_err());
    }

    #[test]
    fn observation_number_is_required_and_positive_in_schema() {
        let missing = json!({
            "idempotency_key": "model-test",
            "exercise_type_key": "translation_drill",
            "observations": [{"weakness_key": "x", "outcome": "correct"}]
        });
        assert!(serde_json::from_value::<RecordPracticeSessionRequest>(missing).is_err());
        let zero = json!({
            "idempotency_key": "model-test",
            "exercise_type_key": "translation_drill",
            "observations": [{"observation_no": 0, "weakness_key": "x", "outcome": "correct"}]
        });
        let parsed: RecordPracticeSessionRequest = serde_json::from_value(zero).unwrap();
        assert_eq!(parsed.observations[0].observation_no, 0);

        let schema = serde_json::to_value(schema_for!(ObservationInput)).unwrap();
        assert_eq!(schema["properties"]["observation_no"]["minimum"], 1);
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "observation_no")
        );
    }

    #[test]
    fn record_status_schema_has_only_created_and_replayed() {
        let schema = serde_json::to_value(schema_for!(RecordStatus)).unwrap();
        let values = schema["enum"].as_array().unwrap();
        assert_eq!(values, &[json!("created"), json!("replayed")]);
    }
}
