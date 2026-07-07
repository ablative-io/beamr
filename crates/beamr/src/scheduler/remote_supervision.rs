//! Inbound remote-supervision appliers: application of wire
//! LINK/UNLINK/EXIT/EXIT2 controls to local processes, and the noconnection
//! backstop delivered on connection loss.
//!
//! Moved out of `supervision_integration.rs` (per-file line budget);
//! `supervision_integration` re-exports [`process_remote_exit_signal`] and
//! [`connection_down`] so the connection-event subscriber
//! (`scheduler/connection_lifecycle.rs`) and existing tests compile unchanged.
//!
//! Every applier here is a thin shim over the slot-protocol-correct paths
//! (R4): all decisions happen under the target's slot lock, the lock is
//! dropped before any wake or cleanup (C1), and Executing targets route
//! through `ProcessMetadata` deferral (`pending_exit_messages` /
//! exit tombstones) so death or trap delivery completes at store-back (C2).

use crate::distribution::control_link::LinkControlDelivery;
use crate::process::{ExitReason, ProcessStatus, RemotePid};
use crate::scheduler::process_slot::PendingExitSource;
use crate::supervision::link;

use super::execution::{cleanup_exited_process, wake_process};
use super::supervision_integration::{
    SchedulerDistributionSendFacility, establish_remote_link, remove_remote_link,
    shared_exit_tombstone,
};
use super::{ProcessSlot, ScheduledProcess, SharedState, dist_control_out, lock_or_recover};
use crate::atom::Atom;

/// How an inbound remote exit signal interacts with link state (ruling 1).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum RemoteExitKind {
    /// EXIT (op 3) and the noconnection backstop: consumes the remote link;
    /// an absent link makes the signal a no-op — the DC-4 exactly-once gate.
    LinkExit,
    /// EXIT2 (op 8, `exit/2`): exit-signal rules regardless of links; never
    /// touches link state.
    Direct,
}

/// Apply one remote exit signal to a local target.
///
/// Reached from the connection-event hub ([`connection_down`], once per
/// remote link on a dead node) and from the inbound wire EXIT/EXIT2 arms of
/// `dispatch_frame` via [`LinkControlDelivery`].
///
/// `LinkExit` signals are gated on actually removing a remote link from
/// `source_pid`: for each remote link exactly one of {wire EXIT(3),
/// noconnection} is applied (DC-4). `Direct` signals skip the link removal
/// entirely.
///
/// Executing targets (D9): `should_die` is computed FIRST, so an inbound
/// Kill to a trapping Executing target dies untrapped (C3); a dying target's
/// `trap_exit` is cleared for Kill (parity with `process_exit_signal`) and
/// its death is deferred through [`shared_exit_tombstone`] — tombstone + ETS
/// transfer + link-set tombstone now, full cleanup (cascade, DOWN, resources)
/// at store-back via `cleanup_if_tombstoned_after_store`.
pub(crate) fn process_remote_exit_signal(
    shared: &SharedState,
    source_pid: RemotePid,
    target_pid: u64,
    reason: ExitReason,
    kind: RemoteExitKind,
) {
    let Some(entry) = shared.process_bodies.get(&target_pid) else {
        return;
    };
    let mut slot = lock_or_recover(&entry);
    match &mut *slot {
        ProcessSlot::Present(ScheduledProcess(target)) => {
            if matches!(target.status(), ProcessStatus::Exited(_)) {
                return;
            }
            if kind == RemoteExitKind::LinkExit && !target.remove_remote_link(source_pid) {
                // No link from `source_pid`: the other delivery path already
                // consumed it (or an unlink crossed the exit). No-op (DC-4).
                return;
            }
            let should_die =
                reason == ExitReason::Kill || (reason != ExitReason::Normal && !target.trap_exit());
            if should_die {
                let propagated_reason = link::terminal_reason(reason);
                target.terminate(propagated_reason);
                drop(slot);
                drop(entry);
                cleanup_exited_process(shared, target_pid, propagated_reason);
            } else if target.trap_exit() {
                link::enqueue_remote_exit_message_pub(target, source_pid, reason);
                drop(slot);
                drop(entry);
                wake_process(shared, target_pid);
            }
        }
        ProcessSlot::Executing(metadata) => {
            if kind == RemoteExitKind::LinkExit && !metadata.remove_remote_link(source_pid) {
                return;
            }
            let should_die =
                reason == ExitReason::Kill || (reason != ExitReason::Normal && !metadata.trap_exit);
            if should_die {
                if reason == ExitReason::Kill {
                    metadata.trap_exit = false;
                }
                shared_exit_tombstone(shared, target_pid, link::terminal_reason(reason));
            } else if metadata.trap_exit {
                metadata
                    .pending_exit_messages
                    .push((PendingExitSource::Remote(source_pid), reason));
                drop(slot);
                drop(entry);
                wake_process(shared, target_pid);
            }
        }
        ProcessSlot::Absent => {}
    }
}

/// Deliver a `noconnection` exit signal to every local process remote-linked
/// to `node`. Reached from the connection-event hub:
/// `ConnectionEventHub::dispatch` →
/// `connection_lifecycle::handle_connection_event` → here, strictly after the
/// pg purge so a trap-exit handler receiving `{'EXIT', _, noconnection}` never
/// observes the dead node's members. Collect-then-apply: no slot lock is held
/// across an application. Each delivery is a [`RemoteExitKind::LinkExit`], so
/// a link whose wire EXIT already landed is not signalled twice (DC-4).
pub(crate) fn connection_down(shared: &SharedState, node: Atom) {
    let affected: Vec<(u64, RemotePid)> = shared
        .process_bodies
        .iter()
        .flat_map(|entry| {
            let pid = *entry.key();
            let slot = lock_or_recover(entry.value());
            match &*slot {
                ProcessSlot::Present(ScheduledProcess(process)) => process
                    .remote_links()
                    .iter()
                    .copied()
                    .filter(|remote| remote.node == node)
                    .map(|remote| (pid, remote))
                    .collect::<Vec<_>>(),
                ProcessSlot::Executing(metadata) => metadata
                    .remote_links
                    .iter()
                    .copied()
                    .filter(|remote| remote.node == node)
                    .map(|remote| (pid, remote))
                    .collect::<Vec<_>>(),
                ProcessSlot::Absent => Vec::new(),
            }
        })
        .collect();
    for (local_pid, remote_pid) in affected {
        process_remote_exit_signal(
            shared,
            remote_pid,
            local_pid,
            ExitReason::NoConnection,
            RemoteExitKind::LinkExit,
        );
    }
}

/// Apply an inbound wire LINK from remote `from` to local `to_pid`.
///
/// All decisions happen under the target's slot lock inside
/// [`establish_remote_link`] (no TOCTOU, ruling 3). A dead or absent target —
/// never a duplicate link, which is idempotent success (ruling 2) — answers
/// with a wire EXIT carrying `noproc` (or the real terminal reason when a
/// tombstone survives), so the remote linker does not hold a dangling
/// half-link forever. An Executing-but-tombstoned target links successfully
/// and self-heals at store-back: `cleanup_exited_process` → `propagate_exit`
/// sends the linker a wire EXIT with the true terminal reason.
pub(super) fn apply_inbound_link(shared: &SharedState, from: RemotePid, to_pid: u64) {
    if !establish_remote_link(shared, to_pid, from) {
        let reason = shared
            .exit_tombstones
            .get(&to_pid)
            .map(link::terminal_reason)
            .unwrap_or(ExitReason::NoProc);
        dist_control_out::send_exit_linked(shared, to_pid, from, reason);
    }
}

/// Inbound link-control sink: thin shims over the slot-protocol-correct
/// appliers above (R4). Wired into `dispatch_frame` by
/// `register_distribution_control_handler`.
impl LinkControlDelivery for SchedulerDistributionSendFacility {
    fn apply_link(&self, from: RemotePid, to_pid: u64) {
        apply_inbound_link(&self.shared, from, to_pid);
    }

    fn apply_unlink(&self, from: RemotePid, to_pid: u64) {
        // No reply on UNLINK (mirrors local unlink); an absent link is a no-op.
        let _ = remove_remote_link(&self.shared, to_pid, from);
    }

    fn apply_link_exit(&self, from: RemotePid, to_pid: u64, reason: ExitReason) {
        // Via the `supervision_integration` re-export deliberately: the shim
        // is the stable in-crate path for the moved appliers.
        super::supervision_integration::process_remote_exit_signal(
            &self.shared,
            from,
            to_pid,
            reason,
            RemoteExitKind::LinkExit,
        );
    }

    fn apply_exit2(&self, from: RemotePid, to_pid: u64, reason: ExitReason) {
        super::supervision_integration::process_remote_exit_signal(
            &self.shared,
            from,
            to_pid,
            reason,
            RemoteExitKind::Direct,
        );
    }
}
