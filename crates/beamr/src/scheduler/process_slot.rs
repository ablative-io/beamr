use crate::namespace::NamespaceId;
use crate::process::ExitReason;

use super::ScheduledProcess;

pub(super) struct ProcessMetadata {
    pub(super) namespace_id: NamespaceId,
    pub(super) links: Vec<u64>,
    pub(super) trap_exit: bool,
    pub(super) pending_exit_messages: Vec<(u64, ExitReason)>,
}

impl ProcessMetadata {
    pub(super) fn add_link(&mut self, pid: u64, self_pid: u64) {
        if pid != self_pid && !self.links.contains(&pid) {
            self.links.push(pid);
        }
    }

    pub(super) fn remove_link(&mut self, pid: u64) {
        self.links.retain(|linked_pid| *linked_pid != pid);
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Default)]
pub(super) enum ProcessSlot {
    Present(ScheduledProcess),
    Executing(ProcessMetadata),
    #[default]
    Absent,
}
