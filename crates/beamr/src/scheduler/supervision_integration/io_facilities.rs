//! The scheduler's io facilities: io messages, file io, and active TCP reads.

use crate::io::{CompletionRing, IoOp};
use crate::native::io_message::IoMessageFacility;
use crate::native::{FileIoCompletion, FileIoContinuation, FileIoFacility};
use crate::scheduler::execution::wake_process;
use crate::scheduler::{ProcessSlot, SharedState, lock_or_recover};
use crate::term::Term;
use std::sync::Arc;

pub(super) struct SchedulerIoMessageFacility {
    pub(super) shared: Arc<SharedState>,
}

impl IoMessageFacility for SchedulerIoMessageFacility {
    fn send_message(&self, sender_pid: u64, target_pid: u64, message: Term) -> bool {
        let _ = sender_pid;
        // Teardown-admission (spec §4 step 3): no delivery into a scheduler
        // that is being (or has been) torn down; held across the delivery so
        // an admitted send lands before shutdown returns.
        let Some(_admission) = self.shared.try_reserve_teardown_admission() else {
            return false;
        };
        let Some(entry) = self.shared.process_bodies.get(&target_pid) else {
            return false;
        };
        let mut slot = lock_or_recover(&entry);
        match &mut *slot {
            ProcessSlot::Present(process) => {
                process.0.mailbox_mut().push_owned(message);
            }
            ProcessSlot::Executing(metadata) => {
                metadata.pending_io_messages.push(
                    crate::scheduler::PendingMailboxMessage::TargetOwned(message),
                );
            }
            ProcessSlot::Absent => return false,
        }
        drop(slot);
        drop(entry);
        wake_process(&self.shared, target_pid);
        true
    }
}

pub(super) struct SchedulerFileIoFacility {
    pub(super) shared: Arc<SharedState>,
    /// The file ring captured at build time. The facility is constructed only
    /// when the file ring is present (spec §3.3), so this is the live Owned or
    /// Shared ring — never a Disabled slot.
    pub(super) ring: Arc<dyn CompletionRing>,
}

impl FileIoFacility for SchedulerFileIoFacility {
    fn submit_file_io(&self, pid: u64, op: IoOp, continuation: FileIoContinuation) -> u64 {
        let op_id = self.ring.submit(op);
        self.track_submitted_file_io(pid, op_id, continuation);
        op_id
    }

    fn track_submitted_file_io(&self, pid: u64, op_id: u64, continuation: FileIoContinuation) {
        if let Some((_, completion)) = self.shared.file_io_orphans.remove(&op_id) {
            self.shared.file_io_results.insert(
                pid,
                FileIoCompletion {
                    op_id,
                    continuation,
                    completion,
                },
            );
            crate::scheduler::execution::wake_process(&self.shared, pid);
        } else {
            self.shared
                .file_io_pending
                .insert(op_id, (pid, continuation));
        }
    }

    fn take_file_io_completion(&self, pid: u64) -> Option<FileIoCompletion> {
        self.shared
            .file_io_results
            .remove(&pid)
            .map(|(_, result)| result)
    }

    fn cancel_pending_file_io_for_pid(&self, pid: u64) {
        let op_ids: Vec<u64> = self
            .shared
            .file_io_pending
            .iter()
            .filter_map(|entry| (entry.value().0 == pid).then_some(*entry.key()))
            .collect();
        for op_id in op_ids {
            if self.shared.file_io_pending.remove(&op_id).is_some() {
                self.shared.file_io_canceled.insert(op_id);
            }
        }
        self.shared.file_io_results.remove(&pid);
    }

    fn ring(&self) -> &dyn CompletionRing {
        self.ring.as_ref()
    }
}

pub(super) struct SchedulerTcpIoFacility {
    pub(super) shared: Arc<SharedState>,
}

impl crate::native::TcpIoFacility for SchedulerTcpIoFacility {
    fn submit_active_tcp_read(
        &self,
        socket: Arc<crate::io::resource::FdInner>,
        buf_len: usize,
    ) -> Option<u64> {
        let op_id = self.shared.submit_file_ring_op(IoOp::Read {
            fd: socket.fd(),
            buf_len,
            offset: u64::MAX,
        })?;
        self.shared.file_io_pending.insert(
            op_id,
            (
                socket.controlling_process(),
                crate::native::FileIoContinuation::TcpActiveRecv { fd: socket },
            ),
        );
        Some(op_id)
    }
}
