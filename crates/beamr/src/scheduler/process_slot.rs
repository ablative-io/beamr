use std::net::SocketAddr;
use std::sync::Arc;

use crate::ets::{EtsTableId, OwnedTerm};
use crate::io::resource::FdInner;
use crate::namespace::NamespaceId;
use crate::native::CapabilitySet;
use crate::process::{ExitReason, Monitor, Priority, RemotePid, SharedRelations};
use crate::term::Term;

use super::ScheduledProcess;

/// Source endpoint for a pending trapped EXIT message.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(super) enum PendingExitSource {
    /// Local immediate PID source.
    Local(u64),
    /// Remote boxed PID source.
    Remote(RemotePid),
}

pub(super) enum PendingMailboxMessage {
    /// A term whose storage already belongs to the target process.
    TargetOwned(Term),
    /// A host-owned term awaiting a checked copy into the target process heap.
    HostOwned {
        message: OwnedTerm,
        completion: std::sync::mpsc::Sender<Result<(), super::MailboxSendError>>,
    },
}

pub(super) struct PendingEtsTransferMessage {
    pub(super) table_id: EtsTableId,
    pub(super) from_pid: u64,
    pub(super) data: OwnedTerm,
}

/// A UDP datagram queued for delivery to a process currently executing on a scheduler thread.
pub struct UdpActiveMessage {
    pub fd: Arc<FdInner>,
    pub bytes: Vec<u8>,
    pub addr: SocketAddr,
}

/// A TCP data chunk queued for delivery to a process currently executing on a scheduler thread.
pub struct TcpActiveMessage {
    pub fd: Arc<FdInner>,
    pub bytes: Vec<u8>,
}

pub(super) struct ProcessMetadata {
    pub(super) namespace_id: NamespaceId,
    pub(super) capabilities: CapabilitySet,
    /// The running process's links and monitors, shared with it until one
    /// side changes them (see `process::relations`).
    pub(super) relations: SharedRelations,
    pub(super) trap_exit: bool,
    pub(super) priority: Priority,
    pub(super) current_mfa: Option<(crate::atom::Atom, crate::atom::Atom, u8)>,
    pub(super) heap_size: usize,
    pub(super) binary_heap_size: usize,
    pub(super) message_queue_len: usize,
    pub(super) group_leader: Term,
    pub(super) logical_clock: u64,
    pub(super) pending_exit_messages: Vec<(PendingExitSource, ExitReason)>,
    pub(super) pending_down_messages: Vec<(u64, u64, ExitReason)>,
    pub(super) pending_io_messages: Vec<PendingMailboxMessage>,
    pub(super) pending_distribution_payloads: Vec<Vec<u8>>,
    /// ETF payloads for local sends queued while the receiver is Executing.
    pub(super) pending_local_messages: Vec<Vec<u8>>,
    pub(super) pending_ets_transfer_messages: Vec<PendingEtsTransferMessage>,
    pub(super) pending_udp_messages: Vec<UdpActiveMessage>,
    pub(super) pending_tcp_messages: Vec<TcpActiveMessage>,
}

impl ProcessMetadata {
    pub(super) fn links(&self) -> &[u64] {
        self.relations.links.as_slice()
    }

    pub(super) fn has_link(&self, pid: u64) -> bool {
        self.relations.links.contains(&pid)
    }

    pub(super) fn add_link(&mut self, pid: u64, self_pid: u64) {
        if pid != self_pid && !self.relations.links.contains(&pid) {
            Arc::make_mut(&mut self.relations.links).insert(pid);
        }
    }

    pub(super) fn remove_link(&mut self, pid: u64) {
        if self.relations.links.contains(&pid) {
            Arc::make_mut(&mut self.relations.links).remove(&pid);
        }
    }

    pub(super) fn add_remote_link(&mut self, pid: RemotePid) {
        if !self.relations.remote_links.contains(&pid) {
            Arc::make_mut(&mut self.relations.remote_links).insert(pid);
        }
    }

    /// Remove a remote link entry; `true` iff an entry was actually removed
    /// (the DC-4 exactly-once gate for link exits, mirroring
    /// `Process::remove_remote_link`).
    pub(super) fn remove_remote_link(&mut self, pid: RemotePid) -> bool {
        self.relations.remote_links.contains(&pid)
            && Arc::make_mut(&mut self.relations.remote_links).remove(&pid)
    }

    /// Every remote link, oldest first, leaving none.
    pub(super) fn take_remote_links(&mut self) -> Vec<RemotePid> {
        let links = self.relations.remote_links.as_slice().to_vec();
        self.relations.remote_links = Arc::default();
        links
    }

    pub(super) fn add_monitor(&mut self, monitor: Monitor) {
        Arc::make_mut(&mut self.relations.monitors).insert(monitor);
    }

    pub(super) fn remove_monitor(&mut self, reference: u64) {
        if self.relations.monitors.contains(reference) {
            Arc::make_mut(&mut self.relations.monitors).remove(reference);
        }
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
