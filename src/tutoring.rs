//! The compact, normal-tutoring recording contract.
//!
//! Expansion is deliberately a pure traversal of the request. It assigns the
//! canonical numbers from array order and never consults the database, the
//! clock, or a weakness match. The resulting request goes through the same
//! validation, transaction and idempotency path as the public canonical tool.

use crate::{
    model::{
        AssessmentPhase, AttemptInput, AttemptReflectionInput, DrillType, EvidenceSource,
        EvidenceStrength, ExerciseTypeKey, FindingInput, FindingKind, FsrsRating, HintLevel,
        LearnerEffort, NewWeaknessInput, ObservationInput, ObservationOutcome, ObservationRole,
        ObservationSeverity, PracticeItemInput, RecordPracticeSessionRequest, ResponseMode,
        RetrievalMode, ReviewInput, TimingSource,
    },
    production::{
        AssessmentProvenance, AttemptEvidence, AttemptKind, CommunicativeOutcome, Exposure,
        Intervention, InterventionKind, ObservationEvidence, ProductionEvidence, PromptCueing,
        SessionOverride, TargetAccuracy, TargetOpportunity, TargetRealization, VariationAssessment,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutoringAttemptReference {
    #[schemars(range(min = 1))]
    pub turn: u16,
    #[schemars(range(min = 1))]
    pub attempt: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutoringObservationReference {
    #[schemars(range(min = 1))]
    pub turn: u16,
    #[schemars(range(min = 1))]
    pub attempt: u16,
    #[schemars(range(min = 1))]
    pub observation: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutoringAttemptEvidence {
    pub kind: AttemptKind,
    #[serde(alias = "original_attempt_ref")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_attempt: Option<TutoringAttemptReference>,
    #[serde(alias = "prompt_cueing")]
    pub cueing: PromptCueing,
    #[serde(alias = "prior_target_exposure")]
    pub exposure: Exposure,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure_source: Option<String>,
    #[serde(alias = "communicative_outcome")]
    pub outcome: CommunicativeOutcome,
    #[serde(alias = "assessment_provenance")]
    pub provenance: AssessmentProvenance,
    #[serde(alias = "scenario_tag")]
    pub scenario: String,
    #[serde(alias = "topic_tag")]
    pub topic: String,
    #[serde(alias = "initial_sentence_count")]
    #[schemars(range(min = 0, max = 100))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sentence_count: Option<u16>,
    /// Optional explicitly reported effort. Observation-level effort remains
    /// the rating input; this value is retained as a learner reflection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<LearnerEffort>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutoringObservationAssessment {
    pub target_realization: TargetRealization,
    pub target_accuracy: TargetAccuracy,
    pub provenance: AssessmentProvenance,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutoringObservationInput {
    pub weakness_key: String,
    pub outcome: ObservationOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<ObservationRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessment_phase: Option<AssessmentPhase>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint_level: Option<HintLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learner_effort: Option<LearnerEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_source: Option<EvidenceSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_strength: Option<EvidenceStrength>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<ObservationSeverity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub produced: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_span: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Present only when the tutor has the production-evidence facts needed
    /// for conservative review analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessment: Option<TutoringObservationAssessment>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutoringFindingInput {
    pub assessment_kind: FindingKind,
    pub original: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutoringInterventionInput {
    pub kind: InterventionKind,
    #[serde(alias = "actual_text")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 20))]
    pub target_weakness_keys: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutoringAttemptInput {
    pub transcript: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_mode: Option<ResponseMode>,
    #[schemars(range(min = 1, max = 7200))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_duration_seconds: Option<u32>,
    #[schemars(range(min = 1, max = 86400000))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_duration_milliseconds: Option<u64>,
    #[schemars(range(min = 1, max = 3600000))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_latency_milliseconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_source: Option<TimingSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<TutoringAttemptEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 300))]
    pub observations: Vec<TutoringObservationInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 50))]
    pub findings: Vec<TutoringFindingInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 20))]
    pub interventions_after: Vec<TutoringInterventionInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutoringTurnInput {
    pub drill_type: DrillType,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 20))]
    pub target_weakness_keys: Vec<String>,
    /// Optional communicative label for the production evidence envelope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communicative_function: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 20))]
    pub attempts: Vec<TutoringAttemptInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutoringVariationInput {
    #[serde(default)]
    #[schemars(length(min = 1, max = 300))]
    pub observation_refs: Vec<TutoringObservationReference>,
    pub rationale: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutoringReviewInput {
    #[serde(alias = "weakness_key")]
    pub target: String,
    pub rating: FsrsRating,
    pub retrieval_mode: RetrievalMode,
    pub evidence_strength: EvidenceStrength,
    #[serde(alias = "evidence_observations", alias = "evidence_observation_refs")]
    #[serde(default)]
    #[schemars(length(min = 1, max = 300))]
    pub observation_refs: Vec<TutoringObservationReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating_rationale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating_source: Option<crate::model::RatingSource>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub evidence: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variation: Option<TutoringVariationInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordTutoringSessionRequest {
    /// Required nonblank key. Exact retries expand to the same canonical
    /// request and replay the existing ledger response.
    #[schemars(length(min = 1))]
    pub idempotency_key: String,
    pub exercise_type_key: ExerciseTypeKey,
    #[schemars(regex(pattern = r"^\d{4}-\d{2}-\d{2}$"))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_date: Option<String>,
    #[schemars(regex(pattern = r"^\d{4}-\d{2}-\d{2}T"))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<String>,
    /// Stable caller-assigned reference for a repeated task or prompt family.
    /// The same value can be passed to get_recent_practice with limit=1.
    #[schemars(length(min = 1, max = 160))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_ref: Option<String>,
    #[schemars(length(min = 1))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[schemars(length(min = 1))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Required when any production evidence is supplied. Omit the evidence
    /// envelope for record-only practice; reviews then remain conservatively
    /// ineligible, as they do on the canonical endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_overrides: Option<SessionOverride>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 100))]
    pub new_weaknesses: Vec<NewWeaknessInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 100))]
    pub turns: Vec<TutoringTurnInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 100))]
    pub reviews: Vec<TutoringReviewInput>,
}

#[derive(Debug, Clone)]
struct AttemptMeta {
    key: (usize, usize),
    item_no: u16,
    attempt_no: u16,
    original: Option<(usize, usize)>,
}

fn text(value: &str, path: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{path} must not be blank"));
    }
    if value.len() > max {
        return Err(format!("{path} must be at most {max} UTF-8 bytes"));
    }
    Ok(())
}

fn optional_text(value: Option<&str>, path: &str, max: usize) -> Result<(), String> {
    if let Some(value) = value {
        text(value, path, max)?;
    }
    Ok(())
}

fn checked_index(value: u16, length: usize, path: &str) -> Result<usize, String> {
    let index = usize::from(value);
    if value == 0 || index > length {
        return Err(format!("{path} references an out-of-range array element"));
    }
    Ok(index - 1)
}

fn resolve_observation(
    reference: &TutoringObservationReference,
    turns: &[TutoringTurnInput],
    observation_numbers: &HashMap<(usize, usize, usize), u16>,
    path: &str,
) -> Result<u16, String> {
    let turn = checked_index(reference.turn, turns.len(), &format!("{path}.turn"))?;
    let attempt = checked_index(
        reference.attempt,
        turns[turn].attempts.len(),
        &format!("{path}.attempt"),
    )?;
    let observation = checked_index(
        reference.observation,
        turns[turn].attempts[attempt].observations.len(),
        &format!("{path}.observation"),
    )?;
    observation_numbers
        .get(&(turn, attempt, observation))
        .copied()
        .ok_or_else(|| format!("{path} references an observation that is not available"))
}

fn compact_observation(value: &TutoringObservationInput, observation_no: u16) -> ObservationInput {
    ObservationInput {
        observation_no,
        weakness_key: value.weakness_key.clone(),
        outcome: value.outcome,
        role: value.role.unwrap_or(ObservationRole::Incidental),
        assessment_phase: value.assessment_phase,
        hint_level: value.hint_level,
        learner_effort: value.learner_effort,
        evidence_source: value.evidence_source,
        evidence_strength: value.evidence_strength,
        severity: value.severity,
        produced: value.produced.clone(),
        correction: value.correction.clone(),
        error_span: value.error_span.clone(),
        notes: value.notes.clone(),
    }
}

fn finding(value: &TutoringFindingInput, item_no: u16, attempt_no: u16) -> FindingInput {
    FindingInput {
        practice_item_no: Some(item_no),
        attempt_no: Some(attempt_no),
        assessment_kind: value.assessment_kind,
        original: value.original.clone(),
        suggestion: value.suggestion.clone(),
        note: value.note.clone(),
    }
}

fn feedback_for_turn(turn: &TutoringTurnInput) -> Result<Option<String>, String> {
    let mut parts = Vec::new();
    for attempt in &turn.attempts {
        for intervention in &attempt.interventions_after {
            text(&intervention.text, "interventions_after.text", 4_000)?;
            parts.push(intervention.text.clone());
        }
    }
    if parts.is_empty() {
        Ok(None)
    } else {
        let feedback = parts.join("\n");
        text(&feedback, "expanded item feedback", 4_000)?;
        Ok(Some(feedback))
    }
}

/// Expand one compact request into the existing canonical recorder contract.
pub fn expand(
    request: RecordTutoringSessionRequest,
) -> Result<RecordPracticeSessionRequest, String> {
    let RecordTutoringSessionRequest {
        idempotency_key,
        exercise_type_key,
        session_date,
        reviewed_at,
        task_ref,
        topic,
        notes,
        policy_version,
        session_overrides,
        new_weaknesses,
        turns,
        reviews,
    } = request;

    text(&idempotency_key, "idempotency_key", 128)?;
    optional_text(session_date.as_deref(), "session_date", 10)?;
    optional_text(reviewed_at.as_deref(), "reviewed_at", 128)?;
    optional_text(task_ref.as_deref(), "task_ref", 160)?;
    optional_text(topic.as_deref(), "topic", 500)?;
    optional_text(notes.as_deref(), "notes", 4_000)?;
    if turns.len() > 100 {
        return Err("turns may contain at most 100 values".into());
    }
    if new_weaknesses.len() > 100 {
        return Err("new_weaknesses may contain at most 100 values".into());
    }
    if reviews.len() > 100 {
        return Err("reviews may contain at most 100 values".into());
    }

    let mut metas = HashMap::new();
    let mut attempt_no = 0u16;
    let mut observations_total = 0usize;
    let mut findings_total = 0usize;
    let mut interventions_total = 0usize;
    for (turn_index, turn) in turns.iter().enumerate() {
        let item_no = u16::try_from(turn_index + 1)
            .map_err(|_| "turn index exceeds canonical item number range".to_owned())?;
        text(&turn.prompt, &format!("turns[{turn_index}].prompt"), 4_000)?;
        if turn.target_weakness_keys.len() > 20 {
            return Err(format!(
                "turns[{turn_index}].target_weakness_keys may contain at most 20 values"
            ));
        }
        for (key_index, key) in turn.target_weakness_keys.iter().enumerate() {
            text(
                key,
                &format!("turns[{turn_index}].target_weakness_keys[{key_index}]"),
                160,
            )?;
        }
        optional_text(
            turn.communicative_function.as_deref(),
            &format!("turns[{turn_index}].communicative_function"),
            1_000,
        )?;
        if turn.attempts.len() > 20 {
            return Err(format!(
                "turns[{turn_index}].attempts may contain at most 20 values"
            ));
        }
        for (attempt_index, attempt) in turn.attempts.iter().enumerate() {
            attempt_no = attempt_no
                .checked_add(1)
                .ok_or_else(|| "expanded attempt numbers exceed u16 range".to_owned())?;
            if attempt_no > 100 {
                return Err("a session may contain at most 100 attempts".into());
            }
            text(
                &attempt.transcript,
                &format!("turns[{turn_index}].attempts[{attempt_index}].transcript"),
                8_000,
            )?;
            if attempt.actual_duration_milliseconds.is_some() && attempt.timing_source.is_none() {
                return Err(format!(
                    "turns[{turn_index}].attempts[{attempt_index}].timing_source is required with actual_duration_milliseconds"
                ));
            }
            if attempt.response_latency_milliseconds.is_some() && attempt.timing_source.is_none() {
                return Err(format!(
                    "turns[{turn_index}].attempts[{attempt_index}].timing_source is required with response_latency_milliseconds"
                ));
            }
            observations_total += attempt.observations.len();
            if observations_total > 300 {
                return Err("a session may contain at most 300 observations".into());
            }
            for (observation_index, observation) in attempt.observations.iter().enumerate() {
                text(
                    &observation.weakness_key,
                    &format!(
                        "turns[{turn_index}].attempts[{attempt_index}].observations[{observation_index}].weakness_key"
                    ),
                    160,
                )?;
                optional_text(
                    observation.produced.as_deref(),
                    &format!(
                        "turns[{turn_index}].attempts[{attempt_index}].observations[{observation_index}].produced"
                    ),
                    2_000,
                )?;
                optional_text(
                    observation.correction.as_deref(),
                    &format!(
                        "turns[{turn_index}].attempts[{attempt_index}].observations[{observation_index}].correction"
                    ),
                    2_000,
                )?;
                optional_text(
                    observation.error_span.as_deref(),
                    &format!(
                        "turns[{turn_index}].attempts[{attempt_index}].observations[{observation_index}].error_span"
                    ),
                    1_000,
                )?;
                optional_text(
                    observation.notes.as_deref(),
                    &format!(
                        "turns[{turn_index}].attempts[{attempt_index}].observations[{observation_index}].notes"
                    ),
                    2_000,
                )?;
            }
            for (finding_index, feedback) in attempt.findings.iter().enumerate() {
                findings_total += 1;
                if findings_total > 300 {
                    return Err("a session may contain at most 300 findings".into());
                }
                text(
                    &feedback.original,
                    &format!(
                        "turns[{turn_index}].attempts[{attempt_index}].findings[{finding_index}].original"
                    ),
                    4_000,
                )?;
                optional_text(
                    feedback.suggestion.as_deref(),
                    &format!(
                        "turns[{turn_index}].attempts[{attempt_index}].findings[{finding_index}].suggestion"
                    ),
                    4_000,
                )?;
                optional_text(
                    feedback.note.as_deref(),
                    &format!(
                        "turns[{turn_index}].attempts[{attempt_index}].findings[{finding_index}].note"
                    ),
                    2_000,
                )?;
            }
            if attempt.interventions_after.len() > 20 {
                return Err(format!(
                    "turns[{turn_index}].attempts[{attempt_index}].interventions_after may contain at most 20 values"
                ));
            }
            interventions_total += attempt.interventions_after.len();
            if interventions_total > 300 {
                return Err("a session may contain at most 300 interventions".into());
            }
            let original = attempt
                .evidence
                .as_ref()
                .and_then(|e| e.original_attempt.as_ref())
                .map(|reference| {
                    let turn = checked_index(
                        reference.turn,
                        turns.len(),
                        &format!(
                            "turns[{turn_index}].attempts[{attempt_index}].evidence.original_attempt.turn"
                        ),
                    )?;
                    let attempt = checked_index(
                        reference.attempt,
                        turns[turn].attempts.len(),
                        &format!(
                            "turns[{turn_index}].attempts[{attempt_index}].evidence.original_attempt.attempt"
                        ),
                    )?;
                    Ok::<(usize, usize), String>((turn, attempt))
                })
                .transpose()?;
            metas.insert(
                (turn_index, attempt_index),
                AttemptMeta {
                    key: (turn_index, attempt_index),
                    item_no,
                    attempt_no,
                    original,
                },
            );
        }
    }

    let mut observation_numbers = HashMap::new();
    let mut next_observation_no = 0u16;
    for (turn_index, turn) in turns.iter().enumerate() {
        for (attempt_index, attempt) in turn.attempts.iter().enumerate() {
            for observation_index in 0..attempt.observations.len() {
                next_observation_no = next_observation_no
                    .checked_add(1)
                    .ok_or_else(|| "expanded observation numbers exceed u16 range".to_owned())?;
                observation_numbers.insert(
                    (turn_index, attempt_index, observation_index),
                    next_observation_no,
                );
            }
        }
    }

    let production_present = turns
        .iter()
        .flat_map(|turn| turn.attempts.iter())
        .any(|attempt| attempt.evidence.is_some())
        || turns
            .iter()
            .flat_map(|turn| turn.attempts.iter())
            .flat_map(|attempt| attempt.observations.iter())
            .any(|observation| observation.assessment.is_some())
        || reviews.iter().any(|review| review.variation.is_some());

    if production_present && policy_version.is_none() {
        return Err("policy_version is required when production evidence is supplied".into());
    }

    let mut canonical_items = Vec::with_capacity(turns.len());
    for (turn_index, turn) in turns.iter().enumerate() {
        canonical_items.push(PracticeItemInput {
            item_no: u16::try_from(turn_index + 1)
                .map_err(|_| "turn index exceeds canonical item number range".to_owned())?,
            drill_type: turn.drill_type,
            prompt: turn.prompt.clone(),
            response: None,
            corrected_response: None,
            reference_answer: None,
            feedback: if production_present {
                None
            } else {
                feedback_for_turn(turn)?
            },
            outcome: None,
            activity_run_no: None,
            item_phase: None,
            target_weakness_keys: turn.target_weakness_keys.clone(),
            observations: Vec::new(),
        });
    }

    let mut canonical_attempts = Vec::with_capacity(usize::from(attempt_no));
    let mut canonical_findings = Vec::new();
    let mut production_attempts = Vec::new();
    let mut production_observations = Vec::new();
    let mut production_interventions = Vec::new();
    let mut intervention_no = 0u16;

    for (turn_index, turn) in turns.iter().enumerate() {
        for (attempt_index, attempt) in turn.attempts.iter().enumerate() {
            let meta = metas
                .get(&(turn_index, attempt_index))
                .ok_or_else(|| {
                    format!(
                        "turns[{turn_index}].attempts[{attempt_index}] is missing attempt metadata"
                    )
                })?;
            let mut canonical_observations = Vec::with_capacity(attempt.observations.len());
            let mut reflections = Vec::new();
            if let Some(evidence) = &attempt.evidence
                && let Some(effort) = evidence.effort
            {
                reflections.push(AttemptReflectionInput {
                    reflection_no: 1,
                    source: crate::model::ReflectionSource::Learner,
                    kind: crate::model::ReflectionKind::General,
                    note: format!("Reported effort: {}", effort.as_str()),
                });
            }
            for (observation_index, observation) in attempt.observations.iter().enumerate() {
                let observation_no =
                    observation_numbers[&(turn_index, attempt_index, observation_index)];
                canonical_observations.push(compact_observation(observation, observation_no));
                if production_present {
                    let assessment = observation.assessment.as_ref().ok_or_else(|| {
                        format!(
                            "turns[{turn_index}].attempts[{attempt_index}].observations[{observation_index}].assessment is required when production evidence is supplied"
                        )
                    })?;
                    production_observations.push(ObservationEvidence {
                        observation_no,
                        attempt_no: meta.attempt_no,
                        target_realization: assessment.target_realization,
                        target_accuracy: assessment.target_accuracy,
                        assessment_provenance: assessment.provenance.clone(),
                    });
                }
            }
            for (finding_index, feedback) in attempt.findings.iter().enumerate() {
                let _ = finding_index;
                canonical_findings.push(finding(feedback, meta.item_no, meta.attempt_no));
            }
            for intervention in &attempt.interventions_after {
                text(&intervention.text, "interventions_after.text", 4_000)?;
                intervention_no = intervention_no
                    .checked_add(1)
                    .ok_or_else(|| "expanded intervention numbers exceed u16 range".to_owned())?;
                if production_present {
                    let next_same_turn = (attempt_index + 1 < turn.attempts.len())
                        .then(|| metas[&(turn_index, attempt_index + 1)].attempt_no);
                    let referenced_retry = metas
                        .values()
                        .filter(|candidate| candidate.original == Some(meta.key))
                        .map(|candidate| candidate.attempt_no)
                        .min();
                    let before_attempt_no = next_same_turn.or(referenced_retry);
                    production_interventions.push(Intervention {
                        intervention_no,
                        kind: intervention.kind,
                        text: intervention.text.clone(),
                        target_weakness_keys: intervention.target_weakness_keys.clone(),
                        after_attempt_no: meta.attempt_no,
                        before_attempt_no,
                    });
                }
            }
            canonical_attempts.push(AttemptInput {
                attempt_no: meta.attempt_no,
                practice_item_no: Some(meta.item_no),
                transcript: attempt.transcript.clone(),
                target_duration_seconds: attempt.target_duration_seconds,
                actual_duration_milliseconds: attempt.actual_duration_milliseconds,
                response_mode: attempt.response_mode,
                response_latency_milliseconds: attempt.response_latency_milliseconds,
                timing_source: attempt.timing_source,
                observations: canonical_observations,
                reflections,
            });
            if production_present {
                let evidence = attempt.evidence.as_ref().ok_or_else(|| {
                    format!(
                        "turns[{turn_index}].attempts[{attempt_index}].evidence is required when production evidence is supplied"
                    )
                })?;
                let original_attempt_no = if let Some(key) = meta.original {
                    Some(
                        metas
                            .get(&key)
                            .ok_or_else(|| {
                                format!(
                                    "turns[{turn_index}].attempts[{attempt_index}].evidence.original_attempt refers to a missing attempt"
                                )
                            })?
                            .attempt_no,
                    )
                } else {
                    None
                };
                if evidence.kind == AttemptKind::Initial && original_attempt_no.is_some() {
                    return Err(format!(
                        "turns[{turn_index}].attempts[{attempt_index}].evidence.original_attempt is not valid for an initial attempt"
                    ));
                }
                if evidence.kind == AttemptKind::SelfCorrection && original_attempt_no.is_none() {
                    return Err(format!(
                        "turns[{turn_index}].attempts[{attempt_index}].evidence.original_attempt is required for self_correction"
                    ));
                }
                if let Some(original) = meta.original {
                    let original_no = metas
                        .get(&original)
                        .ok_or_else(|| {
                            format!(
                                "turns[{turn_index}].attempts[{attempt_index}].evidence.original_attempt refers to a missing attempt"
                            )
                        })?
                        .attempt_no;
                    if original_no >= meta.attempt_no {
                        return Err(format!(
                            "turns[{turn_index}].attempts[{attempt_index}].evidence.original_attempt must refer to an earlier array element"
                        ));
                    }
                }
                text(
                    &evidence.scenario,
                    &format!("turns[{turn_index}].attempts[{attempt_index}].evidence.scenario"),
                    160,
                )?;
                text(
                    &evidence.topic,
                    &format!("turns[{turn_index}].attempts[{attempt_index}].evidence.topic"),
                    160,
                )?;
                optional_text(
                    evidence.exposure_source.as_deref(),
                    &format!(
                        "turns[{turn_index}].attempts[{attempt_index}].evidence.exposure_source"
                    ),
                    500,
                )?;
                production_attempts.push(AttemptEvidence {
                    attempt_no: meta.attempt_no,
                    attempt_kind: evidence.kind,
                    original_attempt_no,
                    prompt_cueing: evidence.cueing,
                    prior_target_exposure: evidence.exposure,
                    exposure_source: evidence.exposure_source.clone(),
                    communicative_outcome: evidence.outcome,
                    assessment_provenance: evidence.provenance.clone(),
                    scenario_tag: evidence.scenario.clone(),
                    topic_tag: evidence.topic.clone(),
                    initial_sentence_count: evidence.sentence_count,
                });
            }
        }
    }

    let mut canonical_reviews = Vec::with_capacity(reviews.len());
    let mut production_variation = Vec::new();
    for (review_index, review) in reviews.iter().enumerate() {
        text(
            &review.target,
            &format!("reviews[{review_index}].target"),
            160,
        )?;
        let evidence_observation_nos = review
            .observation_refs
            .iter()
            .enumerate()
            .map(|(index, reference)| {
                resolve_observation(
                    reference,
                    &turns,
                    &observation_numbers,
                    &format!("reviews[{review_index}].observation_refs[{index}]"),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        canonical_reviews.push(ReviewInput {
            weakness_key: review.target.clone(),
            rating: review.rating,
            retrieval_mode: review.retrieval_mode,
            evidence_strength: review.evidence_strength,
            evidence_observation_nos: evidence_observation_nos.clone(),
            rating_rationale: review.rating_rationale.clone(),
            rating_source: review
                .rating_source
                .unwrap_or(crate::model::RatingSource::AssistantSuggested),
            evidence: review.evidence.clone(),
        });
        if production_present {
            if let Some(variation) = &review.variation {
                if variation.observation_refs.is_empty() {
                    return Err(format!(
                        "reviews[{review_index}].variation.observation_refs must not be empty"
                    ));
                }
                let variation_observations = variation
                    .observation_refs
                    .iter()
                    .enumerate()
                    .map(|(index, reference)| {
                        resolve_observation(
                            reference,
                            &turns,
                            &observation_numbers,
                            &format!("reviews[{review_index}].variation.observation_refs[{index}]"),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                text(
                    &variation.rationale,
                    &format!("reviews[{review_index}].variation.rationale"),
                    2_000,
                )?;
                production_variation.push(VariationAssessment {
                    weakness_key: review.target.clone(),
                    observation_nos: variation_observations,
                    rationale: variation.rationale.clone(),
                });
            }
        }
    }

    let production_evidence = if production_present {
        let policy_version = policy_version.ok_or_else(|| {
            "policy_version is required when production evidence is supplied".to_owned()
        })?;
        let mut target_opportunities = Vec::new();
        for turn in &turns {
            if let Some(function) = &turn.communicative_function {
                for weakness_key in &turn.target_weakness_keys {
                    target_opportunities.push(TargetOpportunity {
                        weakness_key: weakness_key.clone(),
                        communicative_function: function.clone(),
                    });
                }
            }
        }
        Some(ProductionEvidence {
            policy_version,
            session_overrides,
            target_opportunities,
            attempts: production_attempts,
            observations: production_observations,
            interventions: production_interventions,
            variation_assessments: production_variation,
        })
    } else {
        None
    };

    Ok(RecordPracticeSessionRequest {
        production_evidence,
        idempotency_key,
        session_date,
        reviewed_at,
        task_ref,
        exercise_type_key,
        topic,
        notes,
        new_weaknesses,
        items: canonical_items,
        attempts: canonical_attempts,
        observations: Vec::new(),
        findings: canonical_findings,
        reviews: canonical_reviews,
        activity_runs: Vec::new(),
    })
}
