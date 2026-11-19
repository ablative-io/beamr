//! The scheduler's `GroupLeaderFacility`.

use crate::scheduler::{ProcessSlot, ScheduledProcess, SharedState, lock_or_recover};
use crate::term::Term;
use std::sync::Arc;

/// Real `GroupLeaderFacility` backed by the scheduler's shared state.
pub(in crate::scheduler) struct SchedulerGroupLeaderFacility {
    pub(in crate::scheduler) shared: Arc<SharedState>,
}

impl crate::native::GroupLeaderFacility for SchedulerGroupLeaderFacility {
    fn set_group_leader(
        &self,
        pid: u64,
        leader: Term,
    ) -> Result<(), crate::native::group_leader::GroupLeaderError> {
        let Some(_admission) = self.shared.try_reserve_teardown_admission() else {
            return Err(crate::native::group_leader::GroupLeaderError::NoProc);
        };
        let Some(entry) = self.shared.process_bodies.get(&pid) else {
            return Err(crate::native::group_leader::GroupLeaderError::NoProc);
        };
        let mut slot = lock_or_recover(&entry);
        match &mut *slot {
            ProcessSlot::Present(ScheduledProcess(process)) => {
                process.set_group_leader(leader);
                Ok(())
            }
            ProcessSlot::Executing(metadata) => {
                metadata.group_leader = leader;
                Ok(())
            }
            ProcessSlot::Absent => Err(crate::native::group_leader::GroupLeaderError::NoProc),
        }
    }

    fn group_leader(
        &self,
        pid: u64,
    ) -> Result<Term, crate::native::group_leader::GroupLeaderError> {
        let Some(entry) = self.shared.process_bodies.get(&pid) else {
            return Err(crate::native::group_leader::GroupLeaderError::NoProc);
        };
        let slot = lock_or_recover(&entry);
        match &*slot {
            ProcessSlot::Present(ScheduledProcess(process)) => Ok(process.group_leader()),
            ProcessSlot::Executing(metadata) => Ok(metadata.group_leader),
            ProcessSlot::Absent => Err(crate::native::group_leader::GroupLeaderError::NoProc),
        }
    }
}
