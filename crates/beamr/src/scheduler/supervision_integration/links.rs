//! Links: adding, finding and removing a link in a process slot, and the
//! scheduler's `LinkFacility`.

use crate::native::links::{LinkError, LinkFacility};
use crate::scheduler::{ProcessSlot, ScheduledProcess, SharedState, lock_or_recover};
use std::sync::Arc;

pub(super) fn add_link_to_slot(shared: &SharedState, pid: u64, linked_pid: u64) -> bool {
    let Some(entry) = shared.process_bodies.get(&pid) else {
        return false;
    };
    let mut slot = lock_or_recover(&entry);
    match &mut *slot {
        ProcessSlot::Present(ScheduledProcess(process)) => {
            process.add_link(linked_pid);
            true
        }
        ProcessSlot::Executing(metadata) => {
            metadata.add_link(linked_pid, pid);
            true
        }
        ProcessSlot::Absent => false,
    }
}

fn slot_has_link(shared: &SharedState, pid: u64, linked_pid: u64) -> bool {
    let Some(entry) = shared.process_bodies.get(&pid) else {
        return false;
    };
    let slot = lock_or_recover(&entry);
    match &*slot {
        ProcessSlot::Present(ScheduledProcess(process)) => process.links().contains(&linked_pid),
        ProcessSlot::Executing(metadata) => metadata.has_link(linked_pid),
        ProcessSlot::Absent => false,
    }
}

fn remove_link_from_slot(shared: &SharedState, pid: u64, linked_pid: u64) {
    if let Some(entry) = shared.process_bodies.get(&pid) {
        let mut slot = lock_or_recover(&entry);
        match &mut *slot {
            ProcessSlot::Present(ScheduledProcess(process)) => {
                process.remove_link(linked_pid);
            }
            ProcessSlot::Executing(metadata) => metadata.remove_link(linked_pid),
            ProcessSlot::Absent => {}
        }
    }
}

/// Real `LinkFacility` backed by the scheduler's shared state.
pub(in crate::scheduler) struct SchedulerLinkFacility {
    pub(in crate::scheduler) shared: Arc<SharedState>,
}

impl LinkFacility for SchedulerLinkFacility {
    fn link(&self, caller_pid: u64, target_pid: u64) -> Result<(), LinkError> {
        // Teardown-admission (spec §4 step 3), held across the link mutation.
        let Some(_admission) = self.shared.try_reserve_teardown_admission() else {
            return Err(LinkError::NoProc);
        };
        if caller_pid == target_pid {
            return Ok(());
        }

        // Check if target is already dead.
        if self.shared.exit_tombstones.contains_key(&target_pid) {
            return Err(LinkError::NoProc);
        }

        // Check target exists in process table.
        if self.shared.process_table.get(target_pid).is_none() {
            return Err(LinkError::NoProc);
        }

        let already_linked = slot_has_link(&self.shared, caller_pid, target_pid);

        // Add link to caller.
        if !add_link_to_slot(&self.shared, caller_pid, target_pid) {
            return Err(LinkError::NoCaller);
        }

        // Add link to target.
        let target_linked = add_link_to_slot(&self.shared, target_pid, caller_pid);

        if !already_linked && target_linked {
            #[cfg(feature = "telemetry")]
            crate::telemetry::lifecycle::record_process_linked(caller_pid, target_pid);
        }

        Ok(())
    }

    fn unlink(&self, caller_pid: u64, target_pid: u64) -> Result<(), LinkError> {
        if caller_pid == target_pid {
            return Ok(());
        }

        remove_link_from_slot(&self.shared, caller_pid, target_pid);

        remove_link_from_slot(&self.shared, target_pid, caller_pid);

        Ok(())
    }

    fn set_trap_exit(&self, caller_pid: u64, value: bool) -> Result<bool, LinkError> {
        let Some(entry) = self.shared.process_bodies.get(&caller_pid) else {
            return Err(LinkError::NoCaller);
        };
        let mut slot = lock_or_recover(&entry);
        let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
            return Err(LinkError::NoCaller);
        };
        let old = process.trap_exit();
        process.set_trap_exit(value);
        Ok(old)
    }
}
