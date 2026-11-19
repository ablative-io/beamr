//! Remote links: registering the distribution control handler, recording
//! and removing a remote link on a local process, and the scheduler's
//! `DistributionControlFacility`.

use crate::distribution::remote_link::{DistributionControlFacility, RemoteLinkError};
use crate::process::{ExitReason, ProcessStatus, RemotePid};
use crate::scheduler::{
    ProcessSlot, ScheduledProcess, SharedState, dist_control_out, lock_or_recover,
};
use std::sync::Arc;

use super::distribution_send::SchedulerDistributionSendFacility;

pub(in crate::scheduler) fn register_distribution_control_handler(shared: &Arc<SharedState>) {
    // Capture a `Weak`, NOT an `Arc`: the handler is stored inside
    // `ConnectionManagerInner.control_frame_handler`, and `SharedState` owns that
    // `ConnectionManager`. A strong capture would form the permanent cycle
    // `SharedState -> distribution_connections -> control_frame_handler ->
    // Arc<SharedState>`, leaking every scheduler's `SharedState` forever. Mirror
    // the connection-down hook in `scheduler/mod.rs` (the pg-purge closure): hold
    // a `Weak` and `upgrade()` per inbound frame, dropping the frame if the
    // scheduler has already gone.
    // No manager to register on when distribution is Disabled (spec §3.6):
    // inbound control frames cannot arrive without a connection.
    let Some(dist) = shared.distribution() else {
        return;
    };
    let shared_for_handler = Arc::downgrade(shared);
    dist.connections()
        .register_control_frame_handler_with_origin(move |origin, control, payload| {
            if control.is_empty() && payload.is_empty() {
                // Keepalive: the all-zero 8-byte header reaches the handler as
                // an empty control + payload. It is liveness traffic (already
                // consumed by the read loop's inbound-activity refresh), not a
                // control frame — decoding it would fail and count every
                // healthy heartbeat as a dropped "dispatch" frame in telemetry.
                return;
            }
            let Some(shared) = shared_for_handler.upgrade() else {
                // Scheduler dropped: there is nothing left to deliver to.
                return;
            };
            let facility = SchedulerDistributionSendFacility {
                shared: Arc::clone(&shared),
            };
            let sinks = crate::distribution::control_link::ControlSinks {
                delivery: &facility,
                registry: Some(&facility),
                pg: Some(&facility),
                links: Some(&facility),
                // The authenticated handshake name that keys the connection
                // table: a link control whose `from.node` differs is a forgery
                // and is dropped inside `dispatch_frame`.
                origin_node: Some(origin),
                local_node: Some(shared.local_node.name),
            };
            if let Err(_error) = crate::distribution::control_link::dispatch_frame(
                control,
                payload,
                &shared.atom_table,
                &sinks,
            ) {
                // A malformed, misaddressed, or otherwise rejected frame is
                // dropped without killing the read loop (scenario 9) — but
                // never invisibly.
                #[cfg(feature = "telemetry")]
                crate::telemetry::metrics::record_control_frame_dropped("dispatch");
            }
        });
}

/// Record a remote link on a local process. `false` means dead or absent
/// target ONLY: a duplicate link is idempotent success (OTP parity, ruling 2),
/// so callers may key a noproc reply / `BadTarget` on the return value. The
/// status check happens under the slot lock — no TOCTOU. An
/// Executing-but-tombstoned target is deliberately linked (ruling 3): at
/// store-back the tombstone cleanup runs `propagate_exit`, which sends the
/// linker a wire EXIT with the true terminal reason — self-healing.
pub(in crate::scheduler) fn establish_remote_link(
    shared: &SharedState,
    local_pid: u64,
    remote: RemotePid,
) -> bool {
    let Some(entry) = shared.process_bodies.get(&local_pid) else {
        return false;
    };
    let mut slot = lock_or_recover(&entry);
    match &mut *slot {
        ProcessSlot::Present(ScheduledProcess(process)) => {
            if matches!(process.status(), ProcessStatus::Exited(_)) {
                return false;
            }
            let _ = process.add_remote_link(remote); // duplicate = idempotent success
            true
        }
        ProcessSlot::Executing(metadata) => {
            metadata.add_remote_link(remote);
            true
        }
        ProcessSlot::Absent => false,
    }
}

pub(in crate::scheduler) fn remove_remote_link(
    shared: &SharedState,
    local_pid: u64,
    remote: RemotePid,
) -> bool {
    let Some(entry) = shared.process_bodies.get(&local_pid) else {
        return false;
    };
    let mut slot = lock_or_recover(&entry);
    match &mut *slot {
        ProcessSlot::Present(ScheduledProcess(process)) => process.remove_remote_link(remote),
        ProcessSlot::Executing(metadata) => metadata.remove_remote_link(remote),
        ProcessSlot::Absent => false,
    }
}

/// Real `DistributionControlFacility` backed by scheduler remote-link metadata.
pub(in crate::scheduler) struct SchedulerDistributionControlFacility {
    pub(in crate::scheduler) shared: Arc<SharedState>,
}

impl DistributionControlFacility for SchedulerDistributionControlFacility {
    fn link_remote(&self, caller_pid: u64, target: RemotePid) -> Result<(), RemoteLinkError> {
        // Store and send the serial-0 identity: the peer's decode drops the
        // `to` serial and it mints every later EXIT/UNLINK `from` as
        // (node, pid_number, serial 0), so a link stored with a nonzero
        // embedder-supplied serial could never be severed by the wire EXIT
        // equality gate (DC-4) — the death signal would be silently lost
        // until node-down.
        let target = RemotePid {
            serial: 0,
            ..target
        };
        if self.shared.process_table.get(caller_pid).is_none() {
            return Err(RemoteLinkError::BadTarget);
        }
        if !establish_remote_link(&self.shared, caller_pid, target) {
            return Err(RemoteLinkError::BadTarget);
        }
        // ESTABLISH-THEN-SEND order is load-bearing: if the enqueue overflows
        // and the inline down-hook fires on this thread, `connection_down`
        // must observe the just-established link to convert it to
        // noconnection. The connection precondition surfaces as `send_link`'s
        // `NoConnection` (and a pid outside the wire's u32 range as
        // `BadTarget`); on those arms the just-established local half-link is
        // unwound — an unconnected LINK would otherwise be immortal (no
        // connection ⇒ no down event ⇒ no cleanup, ever). If the inline hook
        // already consumed the link, the unwind is a no-op.
        if let Err(error) = dist_control_out::send_link(&self.shared, caller_pid, target) {
            let _ = remove_remote_link(&self.shared, caller_pid, target);
            return Err(error);
        }
        Ok(())
    }

    fn unlink_remote(&self, caller_pid: u64, target: RemotePid) -> Result<(), RemoteLinkError> {
        // Serial-0 identity, mirroring `link_remote`: the stored half-link is
        // always serial 0, so a nonzero embedder serial must not miss it.
        let target = RemotePid {
            serial: 0,
            ..target
        };
        let _ = remove_remote_link(&self.shared, caller_pid, target);
        dist_control_out::send_unlink(&self.shared, caller_pid, target);
        Ok(())
    }

    /// The EXIT2 (`exit/2`) path: best-effort fire-and-forget (ruling 7) —
    /// delivered iff the pinned connection stays up; always `Ok(())`, exactly
    /// as OTP's `exit/2` returns `true` even when undeliverable.
    fn exit_remote(
        &self,
        caller_pid: u64,
        target: RemotePid,
        reason: ExitReason,
    ) -> Result<(), RemoteLinkError> {
        dist_control_out::send_exit2(&self.shared, caller_pid, target, reason);
        Ok(())
    }
}
