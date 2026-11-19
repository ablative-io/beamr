//! Replay mode's recorded schedule: taking the next recorded slice for a
//! worker, and holding a slice to the reductions it recorded.

use crate::replay::RecordedSchedule;
use crate::scheduler::SharedState;

pub(super) fn take_replay_schedule(
    shared: &SharedState,
    pid: u64,
    scheduler_index: usize,
) -> Option<RecordedSchedule> {
    let replay_driver = shared.replay_driver.as_ref()?;
    let mut guard = match replay_driver.lock() {
        Ok(guard) => guard,
        Err(error) => error.into_inner(),
    };
    let recorded = match guard.next_schedule(scheduler_index) {
        Ok(recorded) => recorded,
        Err(error) => {
            shared.exit_errors.insert(pid, error.into());
            return None;
        }
    };
    if recorded.pid == pid {
        Some(recorded)
    } else {
        shared.exit_errors.insert(
            pid,
            crate::error::ExecError::ReplayMismatch(format!(
                "schedule pid mismatch: expected pid {}, recorded pid {}",
                pid, recorded.pid
            )),
        );
        None
    }
}

pub(super) fn validate_replay_schedule_reductions(
    shared: &SharedState,
    recorded_schedule: Option<RecordedSchedule>,
    reductions: u32,
) -> Result<(), ()> {
    let Some(recorded_schedule) = recorded_schedule else {
        return Ok(());
    };
    let Some(replay_driver) = shared.replay_driver.as_ref() else {
        return Ok(());
    };
    let guard = match replay_driver.lock() {
        Ok(guard) => guard,
        Err(error) => error.into_inner(),
    };
    guard
        .validate_schedule_reductions(recorded_schedule, reductions)
        .map_err(|error| {
            shared
                .exit_errors
                .insert(recorded_schedule.pid, error.into());
        })
}
