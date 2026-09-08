//! Durable policy and conservative rule checks. Linguistic assessment remains tutor judgment.
use crate::model::{DrillType, PracticeBriefRequest};
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PracticeMode {
    Spontaneous,
    Controlled,
    Mixed,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PromptPolicy {
    pub allow_required_constructions: bool,
    pub allow_sentence_starters: bool,
    pub allow_model_answer_before_attempt: bool,
    pub prefer_actual_recent_experiences: bool,
    pub one_turn_at_a_time: bool,
    pub adaptive_followups: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WrittenRoundBudget {
    pub minimum: u16,
    pub maximum: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CorrectionPolicy {
    pub after_round: bool,
    pub allow_communication_breakdown_exception: bool,
    pub self_correction_attempts_before_model: u8,
    pub quote_original_on_hint: bool,
    pub show_original_with_model_correction: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub default_practice_mode: PracticeMode,
    pub excluded_drill_types: Vec<DrillType>,
    pub prompt_policy: PromptPolicy,
    /// Sentence budget for initial written responses only. Retries are separate.
    pub written_round_budget: WrittenRoundBudget,
    pub correction_policy: CorrectionPolicy,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreferencePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_practice_mode: Option<PracticeMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_drill_types: Option<Vec<DrillType>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_policy: Option<PromptPolicyPatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written_round_budget: Option<WrittenRoundBudgetPatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correction_policy: Option<CorrectionPolicyPatch>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PromptPolicyPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_required_constructions: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_sentence_starters: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_model_answer_before_attempt: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefer_actual_recent_experiences: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_turn_at_a_time: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptive_followups: Option<bool>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WrittenRoundBudgetPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<u16>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CorrectionPolicyPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_round: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_communication_breakdown_exception: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_correction_attempts_before_model: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote_original_on_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_original_with_model_correction: Option<bool>,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdatePreferencesRequest {
    pub patch: PreferencePatch,
    pub expected_version: u64,
    pub source: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionOverride {
    pub patch: PreferencePatch,
    pub source: String,
}

pub const DDL: &str = "CREATE TABLE IF NOT EXISTS practice_preferences (id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL,preferences_json TEXT NOT NULL,updated_at TEXT NOT NULL,source TEXT NOT NULL); CREATE TABLE IF NOT EXISTS production_sessions(session_id INTEGER PRIMARY KEY REFERENCES sessions(id), policy_json TEXT NOT NULL, evidence_json TEXT NOT NULL, audit_json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS unobserved_targets(session_id INTEGER NOT NULL REFERENCES sessions(id),observation_no INTEGER NOT NULL,PRIMARY KEY(session_id,observation_no));";

pub fn defaults() -> Preferences {
    Preferences {
        default_practice_mode: PracticeMode::Mixed,
        excluded_drill_types: Vec::new(),
        prompt_policy: PromptPolicy {
            allow_required_constructions: true,
            allow_sentence_starters: true,
            allow_model_answer_before_attempt: true,
            prefer_actual_recent_experiences: false,
            one_turn_at_a_time: true,
            adaptive_followups: true,
        },
        written_round_budget: WrittenRoundBudget {
            minimum: 3,
            maximum: 5,
        },
        correction_policy: CorrectionPolicy {
            after_round: true,
            allow_communication_breakdown_exception: true,
            self_correction_attempts_before_model: 1,
            quote_original_on_hint: true,
            show_original_with_model_correction: true,
        },
    }
}
pub fn approved() -> Preferences {
    let mut p = defaults();
    p.default_practice_mode = PracticeMode::Spontaneous;
    p.excluded_drill_types = vec![
        DrillType::Translation,
        DrillType::SentenceTransformation,
        DrillType::SentenceCompletion,
        DrillType::SentenceCombining,
        DrillType::MinimalPairChoice,
        DrillType::ErrorCorrection,
        DrillType::DialogueCompletion,
    ];
    p.prompt_policy.allow_required_constructions = false;
    p.prompt_policy.allow_sentence_starters = false;
    p.prompt_policy.allow_model_answer_before_attempt = false;
    p.prompt_policy.prefer_actual_recent_experiences = true;
    p
}
fn merge(base: &mut Value, patch: Value) {
    if let (Some(b), Some(p)) = (base.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            if v.is_object() {
                merge(b.entry(k.clone()).or_insert(json!({})), v.clone());
            } else {
                b.insert(k.clone(), v.clone());
            }
        }
    }
}
fn patched(p: Preferences, patch: &PreferencePatch) -> Result<Preferences> {
    let mut v = serde_json::to_value(p)?;
    merge(&mut v, serde_json::to_value(patch)?);
    let p: Preferences = serde_json::from_value(v)?;
    if p.written_round_budget.minimum == 0
        || p.written_round_budget.maximum < p.written_round_budget.minimum
        || p.written_round_budget.maximum > 50
    {
        bail!("invalid written initial sentence budget")
    }
    if p.correction_policy.self_correction_attempts_before_model > 5 {
        bail!("at most five self-correction attempts")
    }
    Ok(p)
}
pub fn get(c: &Connection) -> Result<Value> {
    let row:Option<(u64,String,String,String)>=c.query_row("SELECT version,preferences_json,updated_at,source FROM practice_preferences WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    Ok(match row {
        Some((v, p, t, s)) => {
            json!({"version":v,"preferences":serde_json::from_str::<Value>(&p)?,"updated_at":t,"source":s})
        }
        None => {
            json!({"version":0,"preferences":defaults(),"updated_at":null,"source":"system defaults"})
        }
    })
}
pub fn update(c: &Connection, x: UpdatePreferencesRequest) -> Result<Value> {
    if x.source.trim().is_empty() || x.source.len() > 1000 {
        bail!("source must contain 1–1000 bytes describing learner intent")
    }
    let current = get(c)?;
    if current["version"] != json!(x.expected_version) {
        bail!("preference_version_conflict")
    }
    let p = patched(
        serde_json::from_value(current["preferences"].clone())?,
        &x.patch,
    )?;
    let changed=c.execute("INSERT INTO practice_preferences(id,version,preferences_json,updated_at,source) VALUES(1,?1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?3) ON CONFLICT(id) DO UPDATE SET version=excluded.version,preferences_json=excluded.preferences_json,updated_at=excluded.updated_at,source=excluded.source WHERE practice_preferences.version=?4",params![x.expected_version+1,serde_json::to_string(&p)?,x.source,x.expected_version])?;
    if changed != 1 {
        bail!("preference_version_conflict")
    }
    get(c)
}
/// Explicit deployment operation; never infer policy from historical notes or seed other databases.
pub fn seed(c: &Connection) -> Result<Value> {
    if get(c)?["version"] != 0 {
        return get(c);
    }
    let patch: PreferencePatch = serde_json::from_value(serde_json::to_value(approved())?)?;
    update(
        c,
        UpdatePreferencesRequest {
            patch,
            expected_version: 0,
            source:
                "Learner-approved SPONTANEOUS_PRODUCTION.md, 2026-09-06; explicit deployment seed"
                    .into(),
        },
    )
}
pub fn resolve(
    c: &Connection,
    mode: Option<PracticeMode>,
    over: Option<&SessionOverride>,
) -> Result<Value> {
    let saved = get(c)?;
    let mut p: Preferences = serde_json::from_value(saved["preferences"].clone())?;
    if let Some(o) = over {
        if o.source.trim().is_empty() || o.source.len() > 1000 {
            bail!("session override requires a concise learner intent source")
        }
        p = patched(p, &o.patch)?;
    }
    let mut conflicts = vec![];
    if let Some(m) = mode {
        if m != p.default_practice_mode {
            conflicts.push("practice_mode conflicts with preferences; supply session_overrides with learner intent");
        }
    }
    Ok(
        json!({"effective_preferences":p,"preference_version":saved["version"],"session_overrides":over,"conflicts":conflicts}),
    )
}
pub fn constrain(c: &Connection, r: &mut PracticeBriefRequest) -> Result<Value> {
    let mut resolved = resolve(c, r.practice_mode, r.session_overrides.as_ref())?;
    let p: Preferences = serde_json::from_value(resolved["effective_preferences"].clone())?;
    let spontaneous = p.default_practice_mode == PracticeMode::Spontaneous;
    let permitted = |d: &DrillType| {
        !p.excluded_drill_types.contains(d)
            && (!spontaneous
                || matches!(
                    d,
                    DrillType::SituationalResponse
                        | DrillType::QuestionAnswer
                        | DrillType::MicroStory
                        | DrillType::Retell
                ))
    };
    let Some(conflicts) = resolved.get_mut("conflicts").and_then(Value::as_array_mut) else {
        bail!("resolved practice policy is missing conflicts")
    };
    if r.allowed_drill_types
        .as_ref()
        .is_some_and(|ds| ds.iter().any(|d| !permitted(d)))
    {
        conflicts.push(json!("allowed_drill_types contains an excluded initial format; use an explicit session override"));
    }
    if spontaneous
        && matches!(
            r.drill_mix,
            Some(
                crate::model::DrillMix::TranslationOnly
                    | crate::model::DrillMix::RecognitionToProduction
                    | crate::model::DrillMix::Fluency
            )
        )
    {
        conflicts.push(json!("drill_mix conflicts with spontaneous mode"));
    }
    if spontaneous
        && r.activity_type.is_some_and(|a| {
            !matches!(
                a,
                crate::model::ActivityType::SituationalResponse
                    | crate::model::ActivityType::PictureNarration
                    | crate::model::ActivityType::RolePlayComplications
                    | crate::model::ActivityType::QuestionAnswerSprint
                    | crate::model::ActivityType::VoiceDiary
                    | crate::model::ActivityType::CorrectiveConversation
            )
        })
    {
        conflicts.push(json!("activity requires controlled or repeated source production; override mode for this session"));
    }
    let all = [
        DrillType::Translation,
        DrillType::SituationalResponse,
        DrillType::SentenceTransformation,
        DrillType::QuestionAnswer,
        DrillType::SentenceCompletion,
        DrillType::SentenceCombining,
        DrillType::ErrorCorrection,
        DrillType::MinimalPairChoice,
        DrillType::DialogueCompletion,
        DrillType::MicroStory,
        DrillType::Retell,
        DrillType::Fluency432,
    ];
    if spontaneous || !p.excluded_drill_types.is_empty() {
        r.allowed_drill_types = Some(
            r.allowed_drill_types
                .clone()
                .unwrap_or(all.to_vec())
                .into_iter()
                .filter(permitted)
                .collect(),
        );
    }
    if spontaneous {
        r.count = Some(r.count.unwrap_or(3).min(p.written_round_budget.maximum));
    }
    Ok(resolved)
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetOpportunity {
    pub weakness_key: String,
    pub communicative_function: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposedTurn {
    pub turn_no: u16,
    pub prompt: String,
    pub drill_type: DrillType,
    #[serde(default)]
    pub target_weakness_keys: Vec<String>,
    pub sentence_starter: Option<String>,
    pub model_answer: Option<String>,
    pub topic_tag: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidatePlanRequest {
    pub policy_version: u64,
    pub session_overrides: Option<SessionOverride>,
    pub turns: Vec<ProposedTurn>,
    pub target_opportunities: Vec<TargetOpportunity>,
    #[serde(default)]
    pub preceding_context: Vec<String>,
    pub planned_initial_sentences: Option<u16>,
}
pub fn prompt_findings(prompt: &str, targets: &[String]) -> Vec<Value> {
    let lower = prompt.to_lowercase();
    let mut findings = vec![];
    for marker in [
        "exprésalo con",
        "expresalo con",
        "usando ",
        "utiliza ",
        "usa la estructura",
        "une las",
        "completa la frase",
        "transforma ",
        "rellena ",
        "___",
    ] {
        if let Some(i) = lower.find(marker) {
            findings.push(json!({"code":"explicit_construction_or_completion","severity":"error","evidence_span":&lower[i..i+marker.len()],"reason":"Instruction supplies a construction or requests manipulation/completion.","method":"rule"}));
        }
    }
    let llevar = targets
        .iter()
        .any(|k| k.contains("llevar") || k.contains("duration_llevar"));
    let para = targets.iter().any(|k| k.contains("para_que"));
    for marker in [
        "llevas", "llevo", "lleva ", "llevamos", "llevar +", "para que",
    ] {
        if ((llevar && marker != "para que") || (para && marker == "para que"))
            && lower.contains(marker)
        {
            findings.push(json!({"code":"target_construction_supplied","severity":"error","evidence_span":marker,"reason":"Prompt supplies the construction under assessment; ordinary isolated function-word overlap is not checked.","method":"rule"}));
        }
    }
    findings
}
pub fn validate(c: &Connection, r: ValidatePlanRequest) -> Result<Value> {
    let policy = resolve(c, None, r.session_overrides.as_ref())?;
    if policy["preference_version"] != r.policy_version {
        bail!("preference_version_conflict")
    }
    let p: Preferences = serde_json::from_value(policy["effective_preferences"].clone())?;
    if r.turns.is_empty() || r.turns.len() > 20 || r.preceding_context.len() > 20 {
        bail!("validate between one and twenty delivered/proposed turns or context entries")
    }
    for opportunity in &r.target_opportunities {
        if !c.query_row(
            "SELECT EXISTS(SELECT 1 FROM weaknesses WHERE key=?1)",
            [&opportunity.weakness_key],
            |row| row.get::<_, bool>(0),
        )? {
            bail!("invalid target opportunity reference")
        }
    }
    let mut checks = vec![];
    let mut numbers = std::collections::HashSet::new();
    for t in &r.turns {
        if t.turn_no == 0 || !numbers.insert(t.turn_no) || t.prompt.trim().is_empty() {
            bail!("invalid turn number or empty prompt")
        }
        for key in &t.target_weakness_keys {
            if !r
                .target_opportunities
                .iter()
                .any(|x| &x.weakness_key == key)
            {
                bail!("invalid target opportunity reference")
            }
        }
        if p.default_practice_mode == PracticeMode::Spontaneous {
            for mut f in prompt_findings(&t.prompt, &t.target_weakness_keys) {
                f["turn_no"] = json!(t.turn_no);
                checks.push(f);
            }
        }
        for (bad, code) in [
            (
                p.excluded_drill_types.contains(&t.drill_type),
                "excluded_format",
            ),
            (
                t.sentence_starter.is_some() && !p.prompt_policy.allow_sentence_starters,
                "sentence_starter",
            ),
            (
                t.model_answer.is_some() && !p.prompt_policy.allow_model_answer_before_attempt,
                "model_answer_before_attempt",
            ),
            (
                r.preceding_context.iter().any(|s| s == &t.prompt),
                "repeated_prompt",
            ),
        ] {
            if bad {
                checks.push(json!({"code":code,"turn_no":t.turn_no,"severity":"error","evidence_span":t.prompt,"reason":"Proposed turn conflicts with effective policy or repeats preceding wording.","method":"rule"}));
            }
        }
    }
    if r.planned_initial_sentences
        .is_some_and(|n| n < p.written_round_budget.minimum || n > p.written_round_budget.maximum)
    {
        checks.push(json!({"code":"impossible_budget","severity":"error","method":"rule"}));
    }
    Ok(
        json!({"status":if checks.is_empty(){"structural_checks_passed"}else{"revision_required"},"policy_version":r.policy_version,"validator_version":1,"checks":checks,"semantic_review":"required","semantic_review_checklist":["disguised transformation or nearly supplied answer","valid alternative wording accepted","unnecessary invention burden","scenario repetition","single acceptable answer"],"limits":"Rules do not prove independent cognition or verify what was delivered. Validate adaptive follow-ups individually."}),
    )
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttemptKind {
    Initial,
    SelfCorrection,
    FreshTransfer,
    LaterReview,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptCueing {
    NoneDetected,
    Indirect,
    TargetFormSupplied,
    ModelSupplied,
    Unknown,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Exposure {
    NoneKnown,
    RelevantWording,
    Model,
    Unknown,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommunicativeOutcome {
    Successful,
    PartiallySuccessful,
    Unsuccessful,
    NotAssessed,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetRealization {
    Used,
    Attempted,
    NotObserved,
    Ambiguous,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetAccuracy {
    Correct,
    PartiallyCorrect,
    Incorrect,
    NotAssessable,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceKind {
    TutorJudgment,
    LearnerReport,
    DeterministicRule,
    LegacyImport,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssessmentProvenance {
    pub kind: ProvenanceKind,
    pub evidence: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttemptEvidence {
    pub attempt_no: u16,
    pub attempt_kind: AttemptKind,
    pub original_attempt_no: Option<u16>,
    pub prompt_cueing: PromptCueing,
    pub prior_target_exposure: Exposure,
    pub exposure_source: Option<String>,
    pub communicative_outcome: CommunicativeOutcome,
    pub assessment_provenance: AssessmentProvenance,
    /// Tutor-assessed communicative situation, used with variation rationale; different nouns alone do not establish variation.
    pub scenario_tag: String,
    pub topic_tag: String,
    /// Explicit tutor count; no punctuation-based inference from transcripts.
    pub initial_sentence_count: Option<u16>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationEvidence {
    pub observation_no: u16,
    pub attempt_no: u16,
    pub target_realization: TargetRealization,
    pub target_accuracy: TargetAccuracy,
    pub assessment_provenance: AssessmentProvenance,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InterventionKind {
    IndirectHint,
    TargetForm,
    ModelCorrection,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Intervention {
    pub intervention_no: u16,
    pub kind: InterventionKind,
    pub text: String,
    pub target_weakness_keys: Vec<String>,
    pub after_attempt_no: u16,
    pub before_attempt_no: Option<u16>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariationAssessment {
    pub weakness_key: String,
    pub observation_nos: Vec<u16>,
    pub rationale: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductionEvidence {
    pub policy_version: u64,
    pub session_overrides: Option<SessionOverride>,
    pub target_opportunities: Vec<TargetOpportunity>,
    pub attempts: Vec<AttemptEvidence>,
    pub observations: Vec<ObservationEvidence>,
    pub interventions: Vec<Intervention>,
    pub variation_assessments: Vec<VariationAssessment>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ReviewDecision {
    pub weakness_key: String,
    pub eligible: bool,
    pub reason_codes: Vec<String>,
    pub supporting_observation_nos: Vec<u16>,
    pub excluded_observation_nos: Vec<u16>,
    pub policy_version: u64,
    pub applied_rating: Option<String>,
}
fn observations(
    r: &crate::model::RecordPracticeSessionRequest,
) -> Vec<&crate::model::ObservationInput> {
    r.observations
        .iter()
        .chain(r.items.iter().flat_map(|i| i.observations.iter()))
        .chain(r.attempts.iter().flat_map(|a| a.observations.iter()))
        .collect()
}
pub fn validate_evidence(r: &crate::model::RecordPracticeSessionRequest) -> Result<()> {
    let Some(e) = &r.production_evidence else {
        return Ok(());
    };
    let obs = observations(r);
    let mut seen = std::collections::HashSet::new();
    for a in &e.attempts {
        if !seen.insert(a.attempt_no) {
            bail!("duplicate attempt evidence")
        }
        let attempt = r
            .attempts
            .iter()
            .find(|v| v.attempt_no == a.attempt_no)
            .ok_or_else(|| anyhow::anyhow!("invalid attempt reference"))?;
        if attempt.transcript.trim().is_empty()
            || !r
                .items
                .iter()
                .any(|i| Some(i.item_no) == attempt.practice_item_no)
        {
            bail!("each production attempt requires an actual item/prompt and learner transcript")
        }
        if a.assessment_provenance.evidence.trim().is_empty() {
            bail!("assessment provenance requires supporting evidence")
        }
        if let Some(original) = a.original_attempt_no {
            if original >= a.attempt_no || !r.attempts.iter().any(|v| v.attempt_no == original) {
                bail!("invalid original attempt reference/order")
            }
        }
        if a.attempt_kind == AttemptKind::SelfCorrection && a.original_attempt_no.is_none() {
            bail!("self-correction requires original_attempt_no")
        }
        if a.attempt_kind == AttemptKind::Initial && a.original_attempt_no.is_some() {
            bail!("initial attempt cannot be a retry")
        }
    }
    if seen.len() != r.attempts.len() {
        bail!("all attempts require production evidence")
    }
    seen.clear();
    for o in &e.observations {
        if !seen.insert(o.observation_no) {
            bail!("duplicate observation evidence")
        }
        let actual = obs
            .iter()
            .find(|a| a.observation_no == o.observation_no)
            .ok_or_else(|| anyhow::anyhow!("invalid observation reference"))?;
        let a = r
            .attempts
            .iter()
            .find(|a| a.attempt_no == o.attempt_no)
            .ok_or_else(|| anyhow::anyhow!("invalid observation attempt reference"))?;
        if !a
            .observations
            .iter()
            .any(|v| v.observation_no == o.observation_no)
        {
            bail!("production observations must be nested under the supporting attempt")
        }
        if matches!(
            o.target_realization,
            TargetRealization::NotObserved | TargetRealization::Ambiguous
        ) && o.target_accuracy != TargetAccuracy::NotAssessable
        {
            bail!("unobserved/ambiguous targets cannot have assessed accuracy")
        }
        if o.assessment_provenance.evidence.trim().is_empty() {
            bail!("observation provenance requires evidence")
        }
        if matches!(
            o.target_realization,
            TargetRealization::Used | TargetRealization::Attempted
        ) {
            let consistent = match o.target_accuracy {
                TargetAccuracy::Correct => matches!(
                    actual.outcome,
                    crate::model::ObservationOutcome::Correct
                        | crate::model::ObservationOutcome::PromptedCorrect
                ),
                TargetAccuracy::Incorrect => {
                    actual.outcome == crate::model::ObservationOutcome::Incorrect
                }
                TargetAccuracy::PartiallyCorrect => {
                    actual.outcome == crate::model::ObservationOutcome::PartiallyCorrect
                }
                TargetAccuracy::NotAssessable => true,
            };
            if !consistent {
                bail!("target accuracy conflicts with observation outcome")
            }
        } else if actual.outcome != crate::model::ObservationOutcome::Omitted {
            bail!("not observed or ambiguous target must use omitted outcome")
        }
    }
    if seen.len() != obs.len() {
        bail!("all observations require production evidence and attempt linkage")
    }
    seen.clear();
    for i in &e.interventions {
        if i.intervention_no == 0
            || !seen.insert(i.intervention_no)
            || i.text.trim().is_empty()
            || !r
                .attempts
                .iter()
                .any(|a| a.attempt_no == i.after_attempt_no)
            || i.before_attempt_no.is_some_and(|n| {
                n <= i.after_attempt_no || !r.attempts.iter().any(|a| a.attempt_no == n)
            })
        {
            bail!("invalid intervention reference, text or ordering")
        }
    }
    for a in &e.attempts {
        if a.attempt_kind == AttemptKind::SelfCorrection
            && !e.interventions.iter().any(|i| {
                a.original_attempt_no
                    .is_some_and(|n| n <= i.after_attempt_no)
                    && i.before_attempt_no == Some(a.attempt_no)
            })
        {
            bail!("self-correction requires the actual intervention and its timing")
        }
    }
    for v in &e.variation_assessments {
        if v.rationale.trim().is_empty()
            || v.observation_nos.iter().any(|n| {
                !obs.iter()
                    .any(|o| o.observation_no == *n && o.weakness_key == v.weakness_key)
            })
        {
            bail!("invalid variation assessment")
        }
    }
    Ok(())
}
fn attempt_class(
    r: &crate::model::RecordPracticeSessionRequest,
    e: &ProductionEvidence,
    a: &AttemptEvidence,
) -> (&'static str, Vec<Value>) {
    let Some(attempt) = r.attempts.iter().find(|x| x.attempt_no == a.attempt_no) else {
        return (
            "unknown",
            vec![json!({
                "code": "invalid_attempt_reference",
                "attempt_no": a.attempt_no
            })],
        );
    };
    let Some(item) = r
        .items
        .iter()
        .find(|x| Some(x.item_no) == attempt.practice_item_no)
    else {
        return (
            "unknown",
            vec![json!({
                "code": "invalid_practice_item_reference",
                "attempt_no": a.attempt_no
            })],
        );
    };
    let targets = item
        .target_weakness_keys
        .iter()
        .cloned()
        .chain(attempt.observations.iter().map(|o| o.weakness_key.clone()))
        .collect::<Vec<_>>();
    let findings = prompt_findings(&item.prompt, &targets);
    let exposed = e.interventions.iter().any(|i| {
        i.after_attempt_no < a.attempt_no
            && (i.target_weakness_keys.is_empty()
                || i.target_weakness_keys.iter().any(|k| targets.contains(k)))
    });
    let hinted = attempt.observations.iter().any(|o| {
        o.hint_level
            .is_some_and(|h| h != crate::model::HintLevel::None)
    });
    let class = if hinted
        || a.attempt_kind == AttemptKind::SelfCorrection
        || exposed
        || matches!(
            a.prior_target_exposure,
            Exposure::Model | Exposure::RelevantWording
        ) {
        "assisted"
    } else if !findings.is_empty()
        || matches!(
            a.prompt_cueing,
            PromptCueing::Indirect | PromptCueing::TargetFormSupplied | PromptCueing::ModelSupplied
        )
        || matches!(
            item.drill_type,
            DrillType::Translation
                | DrillType::SentenceTransformation
                | DrillType::SentenceCompletion
                | DrillType::SentenceCombining
                | DrillType::ErrorCorrection
                | DrillType::MinimalPairChoice
                | DrillType::DialogueCompletion
        )
    {
        "cued_controlled"
    } else if a.prompt_cueing == PromptCueing::Unknown
        || a.prior_target_exposure == Exposure::Unknown
    {
        "unknown"
    } else {
        "independent"
    };
    (class, findings)
}
pub fn review_decision(
    r: &crate::model::RecordPracticeSessionRequest,
    review: &crate::model::ReviewInput,
    version: u64,
) -> ReviewDecision {
    let mut d = ReviewDecision {
        weakness_key: review.weakness_key.clone(),
        eligible: false,
        reason_codes: vec![],
        supporting_observation_nos: vec![],
        excluded_observation_nos: vec![],
        policy_version: version,
        applied_rating: None,
    };
    let Some(e) = &r.production_evidence else {
        d.reason_codes.push("unknown_assistance".into());
        d.excluded_observation_nos = review.evidence_observation_nos.clone();
        return d;
    };
    let obs = observations(r);
    let mut failure = false;
    let mut scenarios = std::collections::HashSet::new();
    // Inspect every observation of this target: a proposal cannot omit an independent failure.
    for o in e.observations.iter().filter(|o| {
        obs.iter()
            .any(|v| v.observation_no == o.observation_no && v.weakness_key == review.weakness_key)
    }) {
        let Some(a) = e.attempts.iter().find(|a| a.attempt_no == o.attempt_no) else {
            d.reason_codes.push("invalid_attempt_reference".into());
            continue;
        };
        let (class, findings) = attempt_class(r, e, a);
        let Some(actual) = obs.iter().find(|v| v.observation_no == o.observation_no) else {
            d.reason_codes.push("invalid_observation_reference".into());
            continue;
        };
        let reason = if matches!(
            o.target_realization,
            TargetRealization::NotObserved | TargetRealization::Ambiguous
        ) {
            Some("target_not_observed")
        } else if !findings.is_empty() {
            Some("prompt_supplied_target")
        } else if a.attempt_kind == AttemptKind::SelfCorrection {
            Some("assisted_retry")
        } else if class == "unknown" || actual.hint_level.is_none() {
            Some("unknown_assistance")
        } else if class != "independent" || actual.hint_level != Some(crate::model::HintLevel::None)
        {
            Some("assisted_or_cued")
        } else if !matches!(
            actual.assessment_phase,
            Some(
                crate::model::AssessmentPhase::ColdRetrieval
                    | crate::model::AssessmentPhase::Transfer
            )
        ) {
            Some("not_independent_assessment_phase")
        } else if o.target_accuracy == TargetAccuracy::NotAssessable {
            Some("insufficient_rating_evidence")
        } else {
            None
        };
        if let Some(reason) = reason {
            if review.evidence_observation_nos.contains(&o.observation_no) {
                d.excluded_observation_nos.push(o.observation_no);
                d.reason_codes.push(reason.into());
            }
        } else {
            failure |= matches!(
                o.target_accuracy,
                TargetAccuracy::Incorrect | TargetAccuracy::PartiallyCorrect
            );
            if matches!(
                o.target_accuracy,
                TargetAccuracy::Incorrect | TargetAccuracy::PartiallyCorrect
            ) && !review.evidence_observation_nos.contains(&o.observation_no)
            {
                d.reason_codes.push("independent_failure_not_cited".into());
            }
            if review.evidence_observation_nos.contains(&o.observation_no) {
                d.supporting_observation_nos.push(o.observation_no);
                scenarios.insert(a.scenario_tag.clone());
            }
        }
    }
    if review.rating != crate::model::FsrsRating::Again
        && obs
            .iter()
            .filter(|o| d.supporting_observation_nos.contains(&o.observation_no))
            .any(|o| {
                o.learner_effort.is_none()
                    || o.learner_effort == Some(crate::model::LearnerEffort::Unknown)
            })
    {
        d.reason_codes.push("insufficient_rating_evidence".into());
    }
    if d.supporting_observation_nos.len() < 2
        || scenarios.len() < 2
        || !e.variation_assessments.iter().any(|v| {
            v.weakness_key == review.weakness_key
                && d.supporting_observation_nos
                    .iter()
                    .all(|n| v.observation_nos.contains(n))
        })
    {
        d.reason_codes.push("insufficient_varied_evidence".into());
    }
    if failure && review.rating != crate::model::FsrsRating::Again {
        d.reason_codes
            .push("independent_failure_requires_again".into());
    }
    if !failure && review.rating == crate::model::FsrsRating::Again {
        d.reason_codes.push("rating_conflicts_with_success".into());
    }
    if matches!(
        review.rating,
        crate::model::FsrsRating::Good
            | crate::model::FsrsRating::Hard
            | crate::model::FsrsRating::Easy
    ) && review
        .rating_rationale
        .as_ref()
        .is_none_or(|s| s.trim().is_empty())
    {
        d.reason_codes.push("insufficient_rating_evidence".into());
    }
    d.reason_codes.sort();
    d.reason_codes.dedup();
    // Excluded assistance is retained but does not invalidate sufficient independent evidence.
    d.eligible = d.supporting_observation_nos.len() >= 2
        && !d.reason_codes.iter().any(|s| {
            matches!(
                s.as_str(),
                "insufficient_varied_evidence"
                    | "independent_failure_requires_again"
                    | "independent_failure_not_cited"
                    | "rating_conflicts_with_success"
                    | "insufficient_rating_evidence"
            )
        });
    d
}
pub fn audit(
    r: &crate::model::RecordPracticeSessionRequest,
    policy: &Value,
    decisions: &[ReviewDecision],
) -> Value {
    let Some(e) = &r.production_evidence else {
        return json!({"classification":"unknown","reason":"legacy or missing production metadata","initial_turn_denominator":null,"recorded_attempts":r.attempts.len(),"review_decisions":decisions});
    };
    let mut counts = std::collections::BTreeMap::from([
        ("independent", 0),
        ("cued_controlled", 0),
        ("assisted", 0),
        ("unknown", 0),
    ]);
    let mut turns = vec![];
    let mut violations = vec![];
    let mut sentences = 0u32;
    for a in &e.attempts {
        let (class, findings) = attempt_class(r, e, a);
        let initial = a.attempt_kind == AttemptKind::Initial;
        if initial {
            *counts.entry(class).or_insert(0) += 1;
            sentences += u32::from(a.initial_sentence_count.unwrap_or(0));
        }
        if policy["effective_preferences"]["default_practice_mode"] == "spontaneous"
            || policy["effective_preferences"]["prompt_policy"]["allow_required_constructions"]
                == false
        {
            violations.extend(findings);
        }
        let Some(attempt) = r.attempts.iter().find(|v| v.attempt_no == a.attempt_no) else {
            violations.push(json!({
                "code": "invalid_attempt_reference",
                "attempt_no": a.attempt_no
            }));
            continue;
        };
        let Some(item) = r
            .items
            .iter()
            .find(|i| Some(i.item_no) == attempt.practice_item_no)
        else {
            violations.push(json!({
                "code": "invalid_practice_item_reference",
                "attempt_no": a.attempt_no
            }));
            continue;
        };
        if initial
            && policy["effective_preferences"]["excluded_drill_types"]
                .as_array()
                .is_some_and(|ds| ds.contains(&json!(item.drill_type)))
        {
            violations.push(json!({"code":"excluded_format","item_no":item.item_no}));
        }
        turns.push(json!({"attempt_no":a.attempt_no,"attempt_kind":a.attempt_kind,"observed_evidence_class":class,"communicative_outcome":a.communicative_outcome,"topic_tag":a.topic_tag,"scenario_tag":a.scenario_tag,"prompt_excerpt":item.prompt.chars().take(180).collect::<String>()}));
    }
    if sentences
        > policy["effective_preferences"]["written_round_budget"]["maximum"]
            .as_u64()
            .unwrap_or(5) as u32
    {
        violations.push(json!({"code":"initial_output_budget_exceeded"}));
    }
    json!({"initial_turn_denominator":counts.values().sum::<i32>(),"initial_turn_counts":counts,"initial_sentence_count_reported":sentences,"sentence_count_complete":e.attempts.iter().filter(|a|a.attempt_kind==AttemptKind::Initial).all(|a|a.initial_sentence_count.is_some()),"turns":turns,"policy_violations":violations,"target_opportunities":e.target_opportunities,"target_observations":e.observations,"review_decisions":decisions,"validator_version":1,"repetition_limits":"Compare topic/scenario tags and prompt excerpts; these are approximate, not semantic duplicate detection.","classification_limits":"Independent is operational tutor-supported evidence, not measured cognition or oral fluency. Unknowns remain in the denominator."})
}
pub fn session_audit(c: &Connection, sid: i64) -> Result<Value> {
    let row:Option<(String,String,String)>=c.query_row("SELECT policy_json,evidence_json,audit_json FROM production_sessions WHERE session_id=?1",[sid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    Ok(match row {
        Some((p, e, a)) => {
            json!({"policy":serde_json::from_str::<Value>(&p)?,"recorded_evidence":serde_json::from_str::<Value>(&e)?,"audit":serde_json::from_str::<Value>(&a)?})
        }
        None => {
            json!({"audit":{"classification":"unknown","reason":"historical record; no reconstructed attempts or cueing","initial_turn_denominator":null}})
        }
    })
}

/// Function labels are communicative opportunities; grammatical descriptions stay private.
pub fn communicative_function(c: &Connection, weakness_id: i64) -> Result<String> {
    let labels=c.prepare("SELECT c.label FROM weakness_concepts wc JOIN concepts c ON c.id=wc.concept_id WHERE wc.weakness_id=?1 AND wc.role='function' ORDER BY c.key")?.query_map([weakness_id],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(if labels.is_empty() {
        "Share a relevant experience, intention, or explanation in your own words".into()
    } else {
        labels.join("; ")
    })
}
pub fn repetition_summary(sessions: &[Value]) -> Value {
    let mut topics = std::collections::BTreeMap::<String, u64>::new();
    let mut scenarios = std::collections::BTreeMap::<String, u64>::new();
    let mut tagged = 0u64;
    let mut unknown_sessions = 0u64;
    for session in sessions {
        if let Some(turns) = session["production_audit"]["turns"].as_array() {
            for turn in turns.iter().filter(|t| t["attempt_kind"] == "initial") {
                tagged += 1;
                if let Some(tag) = turn["topic_tag"].as_str().filter(|s| !s.is_empty()) {
                    *topics.entry(tag.into()).or_default() += 1;
                }
                if let Some(tag) = turn["scenario_tag"].as_str().filter(|s| !s.is_empty()) {
                    *scenarios.entry(tag.into()).or_default() += 1;
                }
            }
        } else {
            unknown_sessions += 1;
        }
    }
    json!({"scope":"returned sessions, initial turns only; retries excluded","returned_session_count":sessions.len(),"initial_turns_with_metadata":tagged,"sessions_with_unknown_initial_turns":unknown_sessions,"topic_tag_counts":topics,"scenario_tag_counts":scenarios,"limits":"Exact tag counts are approximate repetition indicators, not semantic similarity. Unknown historical turns are not silently counted as novel."})
}
