use crate::model::{
    ActivityConfigInput, ActivityInteractionMode, ActivityItemPhase, ActivityType, DrillType,
    RecordPracticeSessionRequest, ReflectionKind, ReflectionSource, StimulusDeliveryMode,
    StimulusKind,
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct ActivitySpec {
    pub activity_type: ActivityType,
    pub label: &'static str,
    pub interaction_mode: ActivityInteractionMode,
    pub default_count: usize,
    pub compatible_drills: &'static [DrillType],
    pub default_config: ActivityConfigInput,
    pub default_planned_duration_seconds: Option<u32>,
    pub intended_evidence_strength: &'static str,
    pub stimulus_requirements: &'static [StimulusRequirementSpec],
}

#[derive(Debug, Clone, Copy)]
pub struct StimulusRequirementSpec {
    pub alternatives: &'static [StimulusAlternativeSpec],
}

#[derive(Debug, Clone, Copy)]
pub struct StimulusAlternativeSpec {
    pub kind: StimulusKind,
    pub allowed_delivery_modes: &'static [StimulusDeliveryMode],
    pub content_text_required: bool,
    pub source_uri_allowed: bool,
}

const SITUATIONAL: &[DrillType] = &[DrillType::SituationalResponse];
const PICTURE: &[DrillType] = &[DrillType::MicroStory, DrillType::Retell];
const QA: &[DrillType] = &[DrillType::QuestionAnswer];
const RETELL: &[DrillType] = &[DrillType::Retell];
const CONVERSATION: &[DrillType] = &[
    DrillType::QuestionAnswer,
    DrillType::SituationalResponse,
    DrillType::DialogueCompletion,
];
const TRANSFORMATION: &[DrillType] = &[DrillType::SentenceTransformation];
const ROLE_PLAY: &[DrillType] = &[
    DrillType::SituationalResponse,
    DrillType::DialogueCompletion,
    DrillType::QuestionAnswer,
];
const READ: &[StimulusDeliveryMode] = &[StimulusDeliveryMode::Read];
const VIEWED: &[StimulusDeliveryMode] = &[StimulusDeliveryMode::Viewed];
const HEARD_OR_VIEWED: &[StimulusDeliveryMode] = &[
    StimulusDeliveryMode::HeardReported,
    StimulusDeliveryMode::Viewed,
];
const READ_OR_CONVERSATION: &[StimulusDeliveryMode] = &[
    StimulusDeliveryMode::Read,
    StimulusDeliveryMode::Conversation,
];
const NO_STIMULUS_REQUIREMENTS: &[StimulusRequirementSpec] = &[];
const PICTURE_REQUIREMENTS: &[StimulusRequirementSpec] = &[StimulusRequirementSpec {
    alternatives: &[
        StimulusAlternativeSpec {
            kind: StimulusKind::ImageDescription,
            allowed_delivery_modes: VIEWED,
            content_text_required: true,
            source_uri_allowed: false,
        },
        StimulusAlternativeSpec {
            kind: StimulusKind::ImageSequenceDescription,
            allowed_delivery_modes: VIEWED,
            content_text_required: true,
            source_uri_allowed: false,
        },
    ],
}];
const RETELL_REQUIREMENTS: &[StimulusRequirementSpec] = &[StimulusRequirementSpec {
    alternatives: &[
        StimulusAlternativeSpec {
            kind: StimulusKind::SourceText,
            allowed_delivery_modes: READ,
            content_text_required: true,
            source_uri_allowed: false,
        },
        StimulusAlternativeSpec {
            kind: StimulusKind::MediaTranscript,
            allowed_delivery_modes: HEARD_OR_VIEWED,
            content_text_required: true,
            source_uri_allowed: false,
        },
    ],
}];
const READ_CLOSE_REQUIREMENTS: &[StimulusRequirementSpec] = &[StimulusRequirementSpec {
    alternatives: &[
        StimulusAlternativeSpec {
            kind: StimulusKind::SourceText,
            allowed_delivery_modes: READ,
            content_text_required: true,
            source_uri_allowed: false,
        },
        StimulusAlternativeSpec {
            kind: StimulusKind::ArticleReference,
            allowed_delivery_modes: READ,
            content_text_required: false,
            source_uri_allowed: true,
        },
    ],
}];
const ROLE_PLAY_REQUIREMENTS: &[StimulusRequirementSpec] = &[StimulusRequirementSpec {
    alternatives: &[StimulusAlternativeSpec {
        kind: StimulusKind::Situation,
        allowed_delivery_modes: READ_OR_CONVERSATION,
        content_text_required: true,
        source_uri_allowed: false,
    }],
}];

fn config(
    preparation_seconds: Option<u32>,
    response_seconds: Option<u32>,
    question_count: Option<u16>,
    exposure_count: Option<u8>,
    source_hidden_before_response: Option<bool>,
    unpredictable_followups: Option<bool>,
    complication_count: Option<u8>,
) -> ActivityConfigInput {
    ActivityConfigInput {
        preparation_seconds,
        response_seconds,
        question_count,
        exposure_count,
        source_hidden_before_response,
        unpredictable_followups,
        complication_count,
    }
}

pub fn activity_catalog() -> Vec<ActivitySpec> {
    ActivityType::ALL.into_iter().map(activity_spec).collect()
}

pub fn activity_spec(activity_type: ActivityType) -> ActivitySpec {
    match activity_type {
        ActivityType::SituationalResponse => ActivitySpec {
            activity_type,
            label: "Situational response",
            interaction_mode: ActivityInteractionMode::Sprint,
            default_count: 4,
            compatible_drills: SITUATIONAL,
            default_config: config(Some(3), Some(45), None, None, None, None, None),
            default_planned_duration_seconds: None,
            intended_evidence_strength: "spontaneous_production",
            stimulus_requirements: NO_STIMULUS_REQUIREMENTS,
        },
        ActivityType::PictureNarration => ActivitySpec {
            activity_type,
            label: "Picture/comic narration",
            interaction_mode: ActivityInteractionMode::SingleResponse,
            default_count: 1,
            compatible_drills: PICTURE,
            default_config: config(Some(30), Some(120), None, None, None, None, None),
            default_planned_duration_seconds: None,
            intended_evidence_strength: "spontaneous_production",
            stimulus_requirements: PICTURE_REQUIREMENTS,
        },
        ActivityType::QuestionAnswerSprint => ActivitySpec {
            activity_type,
            label: "Question-answer sprint",
            interaction_mode: ActivityInteractionMode::Sprint,
            default_count: 6,
            compatible_drills: QA,
            default_config: config(Some(3), Some(30), Some(6), None, None, None, None),
            default_planned_duration_seconds: None,
            intended_evidence_strength: "spontaneous_production",
            stimulus_requirements: NO_STIMULUS_REQUIREMENTS,
        },
        ActivityType::RetellReconstruction => ActivitySpec {
            activity_type,
            label: "Retell/reconstruction",
            interaction_mode: ActivityInteractionMode::SingleResponse,
            default_count: 1,
            compatible_drills: RETELL,
            default_config: config(None, Some(120), None, Some(1), Some(true), None, None),
            default_planned_duration_seconds: None,
            intended_evidence_strength: "spontaneous_production",
            stimulus_requirements: RETELL_REQUIREMENTS,
        },
        ActivityType::CorrectiveConversation => ActivitySpec {
            activity_type,
            label: "Conversation with corrective feedback",
            interaction_mode: ActivityInteractionMode::MultiTurn,
            default_count: 6,
            compatible_drills: CONVERSATION,
            default_config: config(None, None, None, None, None, Some(true), None),
            default_planned_duration_seconds: Some(1_200),
            intended_evidence_strength: "spontaneous_production",
            stimulus_requirements: NO_STIMULUS_REQUIREMENTS,
        },
        ActivityType::SentenceTransformationSprint => ActivitySpec {
            activity_type,
            label: "Sentence-transformation sprint",
            interaction_mode: ActivityInteractionMode::Sprint,
            default_count: 6,
            compatible_drills: TRANSFORMATION,
            default_config: config(None, None, None, None, None, None, None),
            default_planned_duration_seconds: None,
            intended_evidence_strength: "controlled_production",
            stimulus_requirements: NO_STIMULUS_REQUIREMENTS,
        },
        ActivityType::Dictogloss => ActivitySpec {
            activity_type,
            label: "Dictogloss",
            interaction_mode: ActivityInteractionMode::SingleResponse,
            default_count: 1,
            compatible_drills: RETELL,
            default_config: config(None, Some(180), None, Some(2), Some(true), None, None),
            default_planned_duration_seconds: None,
            intended_evidence_strength: "controlled_production",
            stimulus_requirements: RETELL_REQUIREMENTS,
        },
        ActivityType::VoiceDiary => ActivitySpec {
            activity_type,
            label: "Voice diary",
            interaction_mode: ActivityInteractionMode::SingleResponse,
            default_count: 1,
            compatible_drills: &[DrillType::SituationalResponse, DrillType::MicroStory],
            default_config: config(None, Some(180), None, None, None, None, None),
            default_planned_duration_seconds: None,
            intended_evidence_strength: "spontaneous_production",
            stimulus_requirements: NO_STIMULUS_REQUIREMENTS,
        },
        ActivityType::ReadCloseExplain => ActivitySpec {
            activity_type,
            label: "Read, close, explain",
            interaction_mode: ActivityInteractionMode::SingleResponse,
            default_count: 1,
            compatible_drills: RETELL,
            default_config: config(None, Some(120), None, None, Some(true), None, None),
            default_planned_duration_seconds: None,
            intended_evidence_strength: "spontaneous_production",
            stimulus_requirements: READ_CLOSE_REQUIREMENTS,
        },
        ActivityType::RolePlayComplications => ActivitySpec {
            activity_type,
            label: "Role-play with complications",
            interaction_mode: ActivityInteractionMode::MultiTurn,
            default_count: 4,
            compatible_drills: ROLE_PLAY,
            default_config: config(None, None, None, None, None, Some(true), Some(1)),
            default_planned_duration_seconds: None,
            intended_evidence_strength: "spontaneous_production",
            stimulus_requirements: ROLE_PLAY_REQUIREMENTS,
        },
    }
}

pub fn is_single_response(activity_type: ActivityType) -> bool {
    matches!(
        activity_type,
        ActivityType::PictureNarration
            | ActivityType::RetellReconstruction
            | ActivityType::Dictogloss
            | ActivityType::VoiceDiary
            | ActivityType::ReadCloseExplain
    )
}

pub fn default_config_value(activity_type: ActivityType) -> Result<Value> {
    Ok(serde_json::to_value(
        activity_spec(activity_type).default_config,
    )?)
}

pub fn merged_config(
    activity_type: ActivityType,
    override_config: Option<&ActivityConfigInput>,
) -> Result<ActivityConfigInput> {
    if let Some(overrides) = override_config {
        validate_config_fields(activity_type, overrides)?;
        if overrides.is_empty() {
            bail!("activity config must contain at least one field when supplied")
        }
    }
    let defaults = activity_spec(activity_type).default_config;
    let Some(overrides) = override_config else {
        return Ok(defaults);
    };
    Ok(ActivityConfigInput {
        preparation_seconds: overrides
            .preparation_seconds
            .or(defaults.preparation_seconds),
        response_seconds: overrides.response_seconds.or(defaults.response_seconds),
        question_count: overrides.question_count.or(defaults.question_count),
        exposure_count: overrides.exposure_count.or(defaults.exposure_count),
        source_hidden_before_response: overrides
            .source_hidden_before_response
            .or(defaults.source_hidden_before_response),
        unpredictable_followups: overrides
            .unpredictable_followups
            .or(defaults.unpredictable_followups),
        complication_count: overrides.complication_count.or(defaults.complication_count),
    })
}

pub fn validate_config_fields(
    activity_type: ActivityType,
    config: &ActivityConfigInput,
) -> Result<()> {
    let accepts = |field: &str| -> bool {
        match field {
            "preparation_seconds" => matches!(
                activity_type,
                ActivityType::SituationalResponse
                    | ActivityType::PictureNarration
                    | ActivityType::QuestionAnswerSprint
            ),
            "response_seconds" => matches!(
                activity_type,
                ActivityType::SituationalResponse
                    | ActivityType::PictureNarration
                    | ActivityType::QuestionAnswerSprint
                    | ActivityType::RetellReconstruction
                    | ActivityType::Dictogloss
                    | ActivityType::VoiceDiary
                    | ActivityType::ReadCloseExplain
            ),
            "question_count" => matches!(activity_type, ActivityType::QuestionAnswerSprint),
            "exposure_count" => matches!(
                activity_type,
                ActivityType::RetellReconstruction | ActivityType::Dictogloss
            ),
            "source_hidden_before_response" => matches!(
                activity_type,
                ActivityType::RetellReconstruction
                    | ActivityType::Dictogloss
                    | ActivityType::ReadCloseExplain
            ),
            "unpredictable_followups" => matches!(
                activity_type,
                ActivityType::CorrectiveConversation | ActivityType::RolePlayComplications
            ),
            "complication_count" => {
                matches!(activity_type, ActivityType::RolePlayComplications)
            }
            _ => false,
        }
    };
    let fields = [
        ("preparation_seconds", config.preparation_seconds.is_some()),
        ("response_seconds", config.response_seconds.is_some()),
        ("question_count", config.question_count.is_some()),
        ("exposure_count", config.exposure_count.is_some()),
        (
            "source_hidden_before_response",
            config.source_hidden_before_response.is_some(),
        ),
        (
            "unpredictable_followups",
            config.unpredictable_followups.is_some(),
        ),
        ("complication_count", config.complication_count.is_some()),
    ];
    for (field, present) in fields {
        if present && !accepts(field) {
            bail!(
                "activity config field {field} does not apply to {}",
                activity_type.as_str()
            )
        }
    }
    if config
        .source_hidden_before_response
        .is_some_and(|hidden| !hidden)
        && matches!(
            activity_type,
            ActivityType::Dictogloss | ActivityType::ReadCloseExplain
        )
    {
        bail!(
            "{} requires source_hidden_before_response=true",
            activity_type.as_str()
        )
    }
    Ok(())
}

pub fn validate_config_bounds(config: &ActivityConfigInput) -> Result<()> {
    if config
        .preparation_seconds
        .is_some_and(|value| !(1..=3_600).contains(&value))
    {
        bail!("preparation_seconds must be between 1 and 3600")
    }
    if config
        .response_seconds
        .is_some_and(|value| !(1..=3_600).contains(&value))
    {
        bail!("response_seconds must be between 1 and 3600")
    }
    if config
        .question_count
        .is_some_and(|value| !(1..=20).contains(&value))
    {
        bail!("question_count must be between 1 and 20")
    }
    if config
        .exposure_count
        .is_some_and(|value| !(1..=10).contains(&value))
    {
        bail!("exposure_count must be between 1 and 10")
    }
    if config.complication_count.is_some_and(|value| value > 10) {
        bail!("complication_count must be between 0 and 10")
    }
    Ok(())
}

pub fn validate_stimulus(stimulus: &crate::model::ActivityStimulusInput) -> Result<()> {
    let has_text = stimulus
        .content_text
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let has_uri = stimulus
        .source_uri
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let allowed = match stimulus.kind {
        StimulusKind::Situation => &[
            StimulusDeliveryMode::Read,
            StimulusDeliveryMode::Conversation,
        ][..],
        StimulusKind::SourceText | StimulusKind::ArticleReference => {
            &[StimulusDeliveryMode::Read][..]
        }
        StimulusKind::ImageDescription | StimulusKind::ImageSequenceDescription => {
            &[StimulusDeliveryMode::Viewed][..]
        }
        StimulusKind::MediaTranscript => &[
            StimulusDeliveryMode::HeardReported,
            StimulusDeliveryMode::Viewed,
        ][..],
        StimulusKind::Complication => &[StimulusDeliveryMode::Conversation][..],
    };
    if !allowed.contains(&stimulus.delivery_mode) {
        bail!(
            "stimulus kind {} cannot use delivery mode {}",
            stimulus.kind.as_str(),
            stimulus.delivery_mode.as_str()
        )
    }
    if !has_text && !has_uri {
        bail!("stimulus must contain non-empty content_text or source_uri")
    }
    if stimulus.kind != StimulusKind::ArticleReference && has_uri {
        bail!("source_uri is only allowed for article_reference stimuli")
    }
    if matches!(
        stimulus.kind,
        StimulusKind::Situation
            | StimulusKind::SourceText
            | StimulusKind::ImageDescription
            | StimulusKind::ImageSequenceDescription
            | StimulusKind::MediaTranscript
            | StimulusKind::Complication
    ) && !has_text
    {
        bail!("{} stimuli require content_text", stimulus.kind.as_str())
    }
    Ok(())
}

pub fn stimulus_requirements(activity_type: ActivityType) -> Vec<Value> {
    let alt = |spec: StimulusAlternativeSpec| {
        json!({
            "kind": spec.kind,
            "allowed_delivery_modes": spec.allowed_delivery_modes,
            "content_text_required": spec.content_text_required,
            "source_uri_allowed": spec.source_uri_allowed,
        })
    };
    activity_spec(activity_type)
        .stimulus_requirements
        .iter()
        .map(|requirement| {
            json!({
                "minimum_count": 1,
                "alternatives": requirement
                    .alternatives
                    .iter()
                    .copied()
                    .map(alt)
                    .collect::<Vec<_>>(),
            })
        })
        .collect()
}

pub fn phase_for_item(
    activity_type: ActivityType,
    item_index: usize,
    item_count: usize,
    complication_count: usize,
) -> ActivityItemPhase {
    match activity_type {
        ActivityType::CorrectiveConversation if item_index == 0 => ActivityItemPhase::Initial,
        ActivityType::CorrectiveConversation => ActivityItemPhase::FollowUp,
        ActivityType::RolePlayComplications if item_index == 0 => ActivityItemPhase::Initial,
        ActivityType::RolePlayComplications
            if complication_count > 0 && item_index >= item_count - complication_count =>
        {
            ActivityItemPhase::Complication
        }
        ActivityType::RolePlayComplications => ActivityItemPhase::FollowUp,
        _ => ActivityItemPhase::Initial,
    }
}

fn activity_stimuli_satisfy_requirements(
    activity_type: ActivityType,
    stimuli: &[crate::model::ActivityStimulusInput],
) -> bool {
    activity_spec(activity_type)
        .stimulus_requirements
        .iter()
        .all(|requirement| {
            requirement.alternatives.iter().any(|alternative| {
                stimuli.iter().any(|stimulus| {
                    stimulus.kind == alternative.kind
                        && alternative
                            .allowed_delivery_modes
                            .contains(&stimulus.delivery_mode)
                        && (!alternative.content_text_required
                            || stimulus
                                .content_text
                                .as_deref()
                                .is_some_and(|text| !text.trim().is_empty()))
                        && (alternative.source_uri_allowed || stimulus.source_uri.is_none())
                })
            })
        })
}

pub fn validate_activity_recording(request: &RecordPracticeSessionRequest) -> Result<()> {
    if request.activity_runs.len() > 10 {
        bail!("a session may contain at most 10 activity runs")
    }
    let activity_aware = !request.activity_runs.is_empty();
    let mut runs = HashMap::new();
    for run in &request.activity_runs {
        if run.run_no == 0 || runs.insert(run.run_no, run.activity_type).is_some() {
            bail!("run_no values must be unique integers of at least 1")
        }
        if run
            .planned_duration_seconds
            .is_some_and(|value| !(1..=7_200).contains(&value))
        {
            bail!("planned_duration_seconds must be between 1 and 7200")
        }
        if run
            .actual_duration_milliseconds
            .is_some_and(|value| !(1..=86_400_000).contains(&value))
        {
            bail!("activity actual_duration_milliseconds must be between 1 and 86400000")
        }
        if run.actual_duration_milliseconds.is_some() && run.timing_source.is_none() {
            bail!("activity run actual duration requires timing_source")
        }
        if run.actual_duration_milliseconds.is_none() && run.timing_source.is_some() {
            bail!("activity run timing_source requires actual duration")
        }
        if let Some(config) = &run.config {
            validate_config_fields(run.activity_type, config)?;
            validate_config_bounds(config)?;
            if config.is_empty() {
                bail!("activity config must contain at least one field when supplied")
            }
        }
        if run
            .notes
            .as_deref()
            .is_some_and(|value| value.len() > 4_000)
        {
            bail!("activity run notes must be at most 4000 bytes")
        }
        if run
            .notes
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            bail!("activity run notes must not be empty")
        }
        let mut stimuli = HashSet::new();
        if run.stimuli.len() > 20 {
            bail!("an activity run may contain at most 20 stimuli")
        }
        for stimulus in &run.stimuli {
            if stimulus.stimulus_no == 0 || !stimuli.insert(stimulus.stimulus_no) {
                bail!("stimulus_no values must be unique integers of at least 1")
            }
            if stimulus
                .content_text
                .as_deref()
                .is_some_and(|value| value.len() > 8_000)
            {
                bail!("stimulus content_text must be at most 8000 bytes")
            }
            if stimulus
                .source_uri
                .as_deref()
                .is_some_and(|value| value.len() > 2_000)
            {
                bail!("stimulus source_uri must be at most 2000 bytes")
            }
            if stimulus
                .content_text
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            {
                bail!("stimulus content_text must not be empty")
            }
            if stimulus
                .source_uri
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            {
                bail!("stimulus source_uri must not be empty")
            }
            validate_stimulus(stimulus)?;
        }
    }

    let mut item_runs = HashMap::new();
    let mut item_numbers = HashSet::new();
    for item in &request.items {
        item_numbers.insert(item.item_no);
        if activity_aware {
            let run_no = item.activity_run_no.ok_or_else(|| {
                anyhow::anyhow!("every activity-aware item requires activity_run_no")
            })?;
            let activity_type = runs
                .get(&run_no)
                .copied()
                .ok_or_else(|| anyhow::anyhow!("item references unknown activity run {run_no}"))?;
            if item.target_weakness_keys.len() != 1 {
                bail!("each activity item must target exactly one weakness")
            }
            if !activity_spec(activity_type)
                .compatible_drills
                .contains(&item.drill_type)
            {
                bail!(
                    "drill type {} is incompatible with activity {}",
                    item.drill_type.as_str(),
                    activity_type.as_str()
                )
            }
            let phase = item.item_phase.unwrap_or(ActivityItemPhase::Initial);
            if phase == ActivityItemPhase::Complication
                && !matches!(
                    activity_type,
                    ActivityType::RolePlayComplications | ActivityType::CorrectiveConversation
                )
            {
                bail!("complication item phase is not valid for this activity")
            }
            item_runs.insert(item.item_no, (run_no, phase));
        } else if item.activity_run_no.is_some() || item.item_phase.is_some() {
            bail!("activity item fields require declared activity_runs")
        }
    }
    if activity_aware {
        for attempt in &request.attempts {
            let item_no = attempt.practice_item_no.ok_or_else(|| {
                anyhow::anyhow!("activity-aware attempts require practice_item_no")
            })?;
            if !item_numbers.contains(&item_no) {
                bail!("attempt references unknown practice item {item_no}")
            }
        }
        for run in &request.activity_runs {
            let linked = item_runs
                .values()
                .filter(|(run_no, _)| *run_no == run.run_no)
                .collect::<Vec<_>>();
            if is_single_response(run.activity_type) && linked.len() != 1 {
                bail!(
                    "{} requires exactly one practice item",
                    run.activity_type.as_str()
                )
            }
            if !is_single_response(run.activity_type) && linked.is_empty() {
                bail!(
                    "{} requires at least one practice item",
                    run.activity_type.as_str()
                )
            }
            if !activity_stimuli_satisfy_requirements(run.activity_type, &run.stimuli) {
                bail!(
                    "{} does not contain a stimulus satisfying its declared requirements",
                    run.activity_type.as_str()
                )
            }
            match run.activity_type {
                ActivityType::Dictogloss => {
                    let config = run
                        .config
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("dictogloss requires activity config"))?;
                    if config.exposure_count.is_none() {
                        bail!("dictogloss requires config.exposure_count")
                    }
                    if config.source_hidden_before_response != Some(true) {
                        bail!("dictogloss requires source_hidden_before_response=true")
                    }
                }
                ActivityType::ReadCloseExplain
                    if run
                        .config
                        .as_ref()
                        .and_then(|config| config.source_hidden_before_response)
                        != Some(true) =>
                {
                    bail!("read_close_explain requires source_hidden_before_response=true")
                }
                _ => {}
            }
            if let Some(config) = &run.config {
                if let Some(question_count) = config.question_count
                    && usize::from(question_count) != linked.len()
                {
                    bail!("question_count must equal linked item count")
                }
                if let Some(complication_count) = config.complication_count {
                    let actual = linked
                        .iter()
                        .filter(|(_, phase)| *phase == ActivityItemPhase::Complication)
                        .count();
                    if usize::from(complication_count) != actual {
                        bail!("complication_count must equal complication item count")
                    }
                    if usize::from(complication_count) >= linked.len() {
                        bail!("complication_count must be smaller than item count")
                    }
                }
            }
        }
    }

    for attempt in &request.attempts {
        if attempt
            .response_latency_milliseconds
            .is_some_and(|value| !(1..=3_600_000).contains(&value))
        {
            bail!("response_latency_milliseconds must be between 1 and 3600000")
        }
        if attempt.response_latency_milliseconds.is_some() && attempt.timing_source.is_none() {
            bail!("response latency requires timing_source")
        }
        if attempt.actual_duration_milliseconds.is_some()
            && attempt.timing_source.is_none()
            && activity_aware
        {
            bail!("activity-aware attempt actual duration requires timing_source")
        }
        if attempt
            .actual_duration_milliseconds
            .is_some_and(|value| !(1..=86_400_000).contains(&value))
        {
            bail!("actual_duration_milliseconds must be between 1 and 86400000")
        }
        if attempt.timing_source.is_some()
            && attempt.actual_duration_milliseconds.is_none()
            && attempt.response_latency_milliseconds.is_none()
        {
            bail!("attempt timing_source requires a duration or response latency")
        }
        if attempt.reflections.len() > 20 {
            bail!("an attempt may contain at most 20 reflections")
        }
        let mut reflection_numbers = HashSet::new();
        for reflection in &attempt.reflections {
            if reflection.reflection_no == 0 || !reflection_numbers.insert(reflection.reflection_no)
            {
                bail!("reflection_no values must be unique integers of at least 1")
            }
            if reflection.note.trim().is_empty() {
                bail!("reflection.note must not be empty")
            }
            if reflection.note.len() > 2_000 {
                bail!("reflection.note must be at most 2000 bytes")
            }
            if reflection.source == ReflectionSource::Assistant
                && reflection.kind != ReflectionKind::General
            {
                bail!("assistant reflections may only use kind='general'")
            }
            if reflection.kind != ReflectionKind::General
                && reflection.source != ReflectionSource::Learner
            {
                bail!("reported reflections require source='learner'")
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_exhaustive_and_stable() {
        let catalog = activity_catalog();
        assert_eq!(catalog.len(), ActivityType::ALL.len());
        for activity in ActivityType::ALL {
            assert_eq!(
                catalog
                    .iter()
                    .filter(|s| s.activity_type == activity)
                    .count(),
                1
            );
            assert_eq!(activity_spec(activity).activity_type, activity);
        }
    }

    #[test]
    fn reported_media_and_visual_stimuli_require_text() {
        let stimulus = crate::model::ActivityStimulusInput {
            stimulus_no: 1,
            kind: StimulusKind::MediaTranscript,
            delivery_mode: StimulusDeliveryMode::HeardReported,
            content_text: None,
            source_uri: Some("https://example.test/audio".to_owned()),
        };
        assert!(validate_stimulus(&stimulus).is_err());
    }
}
