use anyhow::{Result, anyhow};
use fsrs::{FSRS, MemoryState, current_retrievability};

pub const ALGORITHM: &str = "fsrs";
pub const ALGORITHM_VERSION: &str = "FSRS-6";
pub const DESIRED_RETENTION: f32 = 0.90;
pub const PARAMETERS_VERSION: i32 = 1;

#[derive(Debug, Clone, Copy)]
pub struct ScheduledState {
    pub memory: MemoryState,
    pub interval_days: i32,
}

pub fn default_parameters_json() -> String {
    serde_json::to_string(&fsrs::DEFAULT_PARAMETERS).expect("FSRS defaults serialize")
}

pub fn schedule(
    previous: Option<MemoryState>,
    elapsed_days: i64,
    rating: i32,
    desired_retention: f32,
    parameters: &[f32],
) -> Result<ScheduledState> {
    if !(1..=4).contains(&rating) || elapsed_days < 0 {
        return Err(anyhow!("invalid FSRS scheduling input"));
    }
    let elapsed_days = u32::try_from(elapsed_days).map_err(|_| anyhow!("elapsed days overflow"))?;
    if !(0.70..=0.97).contains(&desired_retention) {
        return Err(anyhow!("invalid desired retention"));
    }
    let fsrs = FSRS::new(parameters).map_err(|err| anyhow!("invalid FSRS parameters: {err:?}"))?;
    let next = fsrs
        .next_states(previous, desired_retention, elapsed_days)
        .map_err(|err| anyhow!("FSRS scheduling failed: {err:?}"))?;
    let state = match rating {
        1 => next.again,
        2 => next.hard,
        3 => next.good,
        4 => next.easy,
        _ => unreachable!(),
    };
    let interval_days = state.interval.round().max(1.0) as i32;
    Ok(ScheduledState {
        memory: state.memory,
        interval_days,
    })
}

pub fn retrievability(state: MemoryState, elapsed_days: i64, parameters: &[f32]) -> Result<f32> {
    if elapsed_days < 0 {
        return Err(anyhow!("elapsed days cannot be negative"));
    }
    let decay = parameters
        .get(20)
        .copied()
        .unwrap_or(fsrs::FSRS6_DEFAULT_DECAY);
    Ok(current_retrievability(state, elapsed_days as f32, decay))
}

pub fn memory_state(stability: f64, difficulty: f64) -> Result<MemoryState> {
    let state = MemoryState {
        stability: stability as f32,
        difficulty: difficulty as f32,
    };
    if !state.stability.is_finite()
        || state.stability <= 0.0
        || !state.difficulty.is_finite()
        || !(1.0..=10.0).contains(&state.difficulty)
    {
        return Err(anyhow!("invalid cached FSRS memory state"));
    }
    Ok(state)
}
