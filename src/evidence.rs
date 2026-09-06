use crate::model::{
    AssessmentPhase, ObservationInput, ObservationRole, RecordPracticeSessionRequest,
};
use anyhow::{Result, bail};
use std::collections::HashSet;

pub fn normalized_phase(observation: &ObservationInput) -> Result<AssessmentPhase> {
    let phase = match observation.assessment_phase {
        Some(phase) => phase,
        None => match observation.role {
            ObservationRole::Incidental => AssessmentPhase::Incidental,
            ObservationRole::Targeted => {
                bail!("targeted observations must declare an assessment_phase")
            }
        },
    };
    match (observation.role, phase) {
        (ObservationRole::Incidental, AssessmentPhase::Incidental)
        | (ObservationRole::Targeted, AssessmentPhase::ColdRetrieval)
        | (ObservationRole::Targeted, AssessmentPhase::GuidedPractice)
        | (ObservationRole::Targeted, AssessmentPhase::ImmediateRetry)
        | (ObservationRole::Targeted, AssessmentPhase::Transfer) => {}
        (ObservationRole::Incidental, _) => {
            bail!("incidental observations must use assessment_phase='incidental'")
        }
        (ObservationRole::Targeted, AssessmentPhase::Historical) => {
            bail!("historical is reserved for migrated observations")
        }
        (ObservationRole::Targeted, AssessmentPhase::Incidental) => {
            bail!("targeted observations cannot use assessment_phase='incidental'")
        }
    }
    if phase != AssessmentPhase::Incidental && observation.evidence_strength.is_none() {
        bail!("deliberate observations require evidence_strength");
    }
    if observation.hint_level == Some(crate::model::HintLevel::AnswerShown)
        && observation.outcome == crate::model::ObservationOutcome::Correct
    {
        bail!("answer_shown cannot be recorded as correct; use prompted_correct");
    }
    Ok(phase)
}

pub fn validate_session_observation_numbers(request: &RecordPracticeSessionRequest) -> Result<()> {
    let mut numbers = HashSet::new();
    for observation in &request.observations {
        validate_one(observation, &mut numbers)?;
    }
    for attempt in &request.attempts {
        for observation in &attempt.observations {
            validate_one(observation, &mut numbers)?;
        }
    }
    for item in &request.items {
        for observation in &item.observations {
            validate_one(observation, &mut numbers)?;
        }
    }
    Ok(())
}

fn validate_one(observation: &ObservationInput, numbers: &mut HashSet<u16>) -> Result<()> {
    let number = observation.observation_no;
    if number == 0 || !numbers.insert(number) {
        bail!("observation_no values must be unique integers of at least 1 across the session");
    }
    normalized_phase(observation)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EvidenceStrength, HintLevel, ObservationOutcome};

    fn base() -> ObservationInput {
        ObservationInput {
            observation_no: 1,
            weakness_key: "x".into(),
            outcome: ObservationOutcome::Incorrect,
            role: ObservationRole::Targeted,
            assessment_phase: Some(AssessmentPhase::ColdRetrieval),
            hint_level: Some(HintLevel::None),
            learner_effort: None,
            evidence_source: None,
            evidence_strength: Some(EvidenceStrength::ControlledProduction),
            severity: None,
            produced: None,
            correction: None,
            error_span: None,
            notes: None,
        }
    }

    #[test]
    fn targeted_requires_phase_and_strength() {
        let mut x = base();
        x.evidence_strength = None;
        assert!(normalized_phase(&x).is_err());
        x.assessment_phase = None;
        x.evidence_strength = Some(EvidenceStrength::ControlledProduction);
        assert!(normalized_phase(&x).is_err());
    }
}
