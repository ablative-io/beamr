//! Reading a process's state by pid: its trap_exit, links, nativeness,
//! namespace and process_info.

use super::process_slot::{ProcessMetadata, ProcessSlot};
use super::{Scheduler, SharedState, lock_or_recover};
use crate::namespace::NamespaceId;
use crate::native::{ProcessInfoItem, ProcessInfoStatus, ProcessInfoValue, ProcessMonitorInfo};
use crate::process::registry::ProcessTable;
use crate::process::{Process, ProcessStatus};

impl Scheduler {
    #[must_use]
    pub fn trap_exit(&self, pid: u64) -> Option<bool> {
        process_trap_exit(&self.shared, pid)
    }
    #[must_use]
    pub fn is_linked(&self, left: u64, right: u64) -> bool {
        process_links_contain(&self.shared, left, right)
            && process_links_contain(&self.shared, right, left)
    }
    /// Whether the process is native (carries a Rust handler).
    ///
    /// `Some(true)` for a parked native process, `Some(false)` for a parked
    /// bytecode process, and `None` when the process is absent (its body has
    /// been removed by `cleanup_exited_process`) or currently mid-slice (its
    /// `Process` is checked out, so native-ness is not observable from the
    /// metadata shadow). A `None` after an expected exit therefore confirms
    /// the body was removed from `process_bodies`.
    #[must_use]
    pub fn is_native(&self, pid: u64) -> Option<bool> {
        process_is_native(&self.shared, pid)
    }

    #[must_use]
    pub fn process_namespace(&self, pid: u64) -> Option<NamespaceId> {
        process_namespace(&self.shared, pid)
    }
    #[must_use]
    pub fn process_table(&self) -> &ProcessTable {
        &self.shared.process_table
    }
}
fn process_namespace(shared: &SharedState, pid: u64) -> Option<NamespaceId> {
    let entry = shared.process_bodies.get(&pid)?;
    match &*lock_or_recover(&entry) {
        ProcessSlot::Present(scheduled) => Some(scheduled.0.namespace_id()),
        ProcessSlot::Executing(metadata) => Some(metadata.namespace_id),
        ProcessSlot::Absent => None,
    }
}
fn process_trap_exit(shared: &SharedState, pid: u64) -> Option<bool> {
    let entry = shared.process_bodies.get(&pid)?;
    match &*lock_or_recover(&entry) {
        ProcessSlot::Present(scheduled) => Some(scheduled.0.trap_exit()),
        ProcessSlot::Executing(metadata) => Some(metadata.trap_exit),
        ProcessSlot::Absent => None,
    }
}
fn process_is_native(shared: &SharedState, pid: u64) -> Option<bool> {
    let entry = shared.process_bodies.get(&pid)?;
    match &*lock_or_recover(&entry) {
        ProcessSlot::Present(scheduled) => Some(scheduled.0.is_native()),
        // Mid-slice the `Process` is checked out; native-ness lives on it, not
        // on the metadata shadow. Absent means the body is being swapped.
        ProcessSlot::Executing(_) | ProcessSlot::Absent => None,
    }
}
pub(super) fn process_links_contain(shared: &SharedState, pid: u64, linked_pid: u64) -> bool {
    let Some(entry) = shared.process_bodies.get(&pid) else {
        return false;
    };
    match &*lock_or_recover(&entry) {
        ProcessSlot::Present(scheduled) => scheduled.0.links().contains(&linked_pid),
        ProcessSlot::Executing(metadata) => metadata.has_link(linked_pid),
        ProcessSlot::Absent => false,
    }
}

impl SharedState {
    pub(super) fn process_info(&self, pid: u64, item: ProcessInfoItem) -> Option<ProcessInfoValue> {
        self.process_table.get(pid)?;
        let entry = self.process_bodies.get(&pid)?;
        match &*lock_or_recover(&entry) {
            ProcessSlot::Present(scheduled) => process_info_from_process(&scheduled.0, item),
            ProcessSlot::Executing(metadata) => process_info_from_metadata(metadata, item),
            ProcessSlot::Absent => None,
        }
    }
}

fn process_info_from_process(process: &Process, item: ProcessInfoItem) -> Option<ProcessInfoValue> {
    if matches!(process.status(), ProcessStatus::Exited(_)) {
        return None;
    }
    Some(match item {
        ProcessInfoItem::CurrentFunction => {
            ProcessInfoValue::CurrentFunction(process.current_mfa())
        }
        ProcessInfoItem::HeapSize => ProcessInfoValue::HeapSize(process.heap().total_used()),
        ProcessInfoItem::MessageQueueLen => {
            ProcessInfoValue::MessageQueueLen(process.mailbox().message_count())
        }
        ProcessInfoItem::RegisteredName => ProcessInfoValue::RegisteredName(None),
        ProcessInfoItem::Status => ProcessInfoValue::Status(status_from_process(process.status())?),
        ProcessInfoItem::TrapExit => ProcessInfoValue::TrapExit(process.trap_exit()),
        ProcessInfoItem::Priority => ProcessInfoValue::Priority(process.priority()),
        ProcessInfoItem::Links => ProcessInfoValue::Links(process.links().to_vec()),
        ProcessInfoItem::Monitors => ProcessInfoValue::Monitors(
            process
                .monitors()
                .iter()
                .map(|monitor| ProcessMonitorInfo {
                    watcher: monitor.watcher(),
                    target: monitor.target(),
                })
                .collect(),
        ),
    })
}

fn process_info_from_metadata(
    metadata: &ProcessMetadata,
    item: ProcessInfoItem,
) -> Option<ProcessInfoValue> {
    Some(match item {
        ProcessInfoItem::CurrentFunction => ProcessInfoValue::CurrentFunction(metadata.current_mfa),
        ProcessInfoItem::HeapSize => ProcessInfoValue::HeapSize(metadata.heap_size),
        ProcessInfoItem::MessageQueueLen => {
            ProcessInfoValue::MessageQueueLen(metadata.message_queue_len)
        }
        ProcessInfoItem::RegisteredName => ProcessInfoValue::RegisteredName(None),
        ProcessInfoItem::Status => ProcessInfoValue::Status(ProcessInfoStatus::Running),
        ProcessInfoItem::TrapExit => ProcessInfoValue::TrapExit(metadata.trap_exit),
        ProcessInfoItem::Priority => ProcessInfoValue::Priority(metadata.priority),
        ProcessInfoItem::Links => ProcessInfoValue::Links(metadata.links().to_vec()),
        ProcessInfoItem::Monitors => ProcessInfoValue::Monitors(
            metadata
                .relations
                .monitors
                .as_vec()
                .iter()
                .map(|monitor| ProcessMonitorInfo {
                    watcher: monitor.watcher(),
                    target: monitor.target(),
                })
                .collect(),
        ),
    })
}

fn status_from_process(status: ProcessStatus) -> Option<ProcessInfoStatus> {
    match status {
        ProcessStatus::New | ProcessStatus::Running | ProcessStatus::Yielded => {
            Some(ProcessInfoStatus::Running)
        }
        ProcessStatus::Waiting => Some(ProcessInfoStatus::Waiting),
        ProcessStatus::Suspended => Some(ProcessInfoStatus::Suspended),
        ProcessStatus::Exited(_) => None,
    }
}
