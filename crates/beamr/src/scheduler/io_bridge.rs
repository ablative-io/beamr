//! Where the io subsystem meets the scheduler: submitting file ring
//! operations, and waking or messaging a process when io completes.

use super::process_slot::{PendingMailboxMessage, ProcessSlot};
use super::{SharedState, execution, lock_or_recover, suspension};
use crate::io::{IoOp, IoWakeTarget};
use crate::term::Term;

impl SharedState {
    /// Submit an operation to the file-IO ring, or `None` when it is `Disabled`
    /// (spec §3.3). The completion re-arm paths use this so a disabled ring
    /// stops re-arming instead of dispatching into a ring that was never built.
    pub(super) fn submit_file_ring_op(&self, op: IoOp) -> Option<u64> {
        self.file_io_ring
            .service()
            .map(|ring| ring.ring().submit(op))
    }
}

impl IoWakeTarget for SharedState {
    fn wake_with_io_result(&self, pid: u64, term: Term) {
        // Identity-resolved at publish time: the bridge completes the
        // host-await suspension the submitting native registered. A stale
        // completion (the await already timed out and re-entered) is
        // dropped instead of being applied blind.
        let Some(payload) = suspension::SuspensionResultPayload::host(term) else {
            return;
        };
        let _published = self.publish_suspension_result_current(
            pid,
            crate::process::SuspensionKind::HostAwait,
            payload,
        );
        execution::wake_process(self, pid);
    }

    fn send_io_message(&self, pid: u64, term: Term) {
        let Some(entry) = self.process_bodies.get(&pid) else {
            return;
        };
        let mut slot = lock_or_recover(&entry);
        if let ProcessSlot::Present(process) = &mut *slot {
            process.0.mailbox_mut().push_owned(term);
        } else if let ProcessSlot::Executing(metadata) = &mut *slot {
            metadata
                .pending_io_messages
                .push(PendingMailboxMessage::TargetOwned(term));
        }
        drop(slot);
        drop(entry);
        if pid == self.standard_io_pid {
            let mut wait_set = lock_or_recover(&self.wait_set);
            wait_set.woken.push((pid, 0));
            // W15: edge E in the same critical section.
            self.wake_edge_locked(&wait_set);
        } else {
            execution::wake_process(self, pid);
        }
    }
}
