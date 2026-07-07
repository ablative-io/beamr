//! Outbound distribution controls: encode on the calling worker, pin the
//! current connection generation, hand off to the must-deliver control lane.
//! Never blocks, never dials (C7).
//!
//! Common body shape (shared by every sender here): resolve the target's
//! connection ONCE at enqueue time — the DC-2 generation pin — then encode
//! via `control_link` and `DistSender::enqueue_control`. A full lane needs no
//! handling by the caller: `enqueue_control` has already marked the pinned
//! connection down (`ControlOverflow`), and the inline down-hook's
//! noconnection backstop supplies the signals (DC-1(b)/DC-3). Pid components
//! beyond the wire's u32 range are refused at this boundary
//! ([`wire_encodable`]); an encode failure past that guard — unreachable for
//! the inputs this module produces — still downs the pinned connection: DC-1
//! has no silent arm.
//!
//! Replay mode (`dist_sender` is `None`): LINK fails `NoConnection`;
//! UNLINK/EXIT/EXIT2 no-op — distribution is globally off in replay.

use std::sync::Arc;

use crate::distribution::connection::DistConnection;
use crate::distribution::control_link::{
    ControlOp, encode_exit_frame, encode_link_frame, encode_unlink_frame,
};
use crate::distribution::remote_link::RemoteLinkError;
use crate::distribution::sender::{ControlOutbound, DistSender};
use crate::etf::encode::EncodeError;
use crate::process::{ExitReason, RemotePid};

use super::SharedState;

/// Send a wire LINK from local `caller_pid` to remote `target`.
///
/// Absent connection (or replay mode) ⇒ `Err(NoConnection)`: an unconnected
/// LINK would create an immortal local half-link — no connection means no
/// down event and therefore no cleanup, ever. No auto-dial (C7; explicit
/// `connect_node` is the embedder pattern; the OTP auto-connect divergence is
/// recorded in the wire spec). An endpoint pid beyond the wire's u32 range ⇒
/// `Err(BadTarget)` ([`wire_encodable`]).
pub(super) fn send_link(
    shared: &SharedState,
    caller_pid: u64,
    target: RemotePid,
) -> Result<(), RemoteLinkError> {
    if !wire_encodable(caller_pid, target) {
        // A link whose endpoints cannot ride the wire could never be severed
        // by a wire EXIT — refuse it here instead of letting the encode
        // failure down the whole connection.
        return Err(RemoteLinkError::BadTarget);
    }
    let Some(sender) = &shared.dist_sender else {
        return Err(RemoteLinkError::NoConnection);
    };
    // DC-2 pin: resolve once, at enqueue; the drain writes only to this
    // connection generation and skips it once down.
    let Some(connection) = shared.distribution_connections.get_connection(target.node) else {
        return Err(RemoteLinkError::NoConnection);
    };
    let frame = encode_link_frame(
        shared.local_node.name,
        caller_pid,
        target,
        &shared.atom_table,
    );
    enqueue_pinned(sender, connection, frame);
    Ok(())
}

/// Send a wire UNLINK from local `caller_pid` to remote `target`.
///
/// Absent connection ⇒ drop. Not lossy: the peer's own down-hook already
/// delivered (or will deliver) noconnection for every link to us (DC-3, both
/// sides), which severs the remote half too.
pub(super) fn send_unlink(shared: &SharedState, caller_pid: u64, target: RemotePid) {
    if !wire_encodable(caller_pid, target) {
        // No remote link can exist between wire-unencodable endpoints
        // (`send_link` refuses them; inbound endpoints decode from u32 wire
        // fields), so there is nothing to unlink — dropping is not lossy.
        return;
    }
    let Some(sender) = &shared.dist_sender else {
        return;
    };
    let Some(connection) = shared.distribution_connections.get_connection(target.node) else {
        return;
    };
    let frame = encode_unlink_frame(
        shared.local_node.name,
        caller_pid,
        target,
        &shared.atom_table,
    );
    enqueue_pinned(sender, connection, frame);
}

/// Send a wire EXIT (op 3, link-exit) from local endpoint `from_pid` — dead
/// or dying — to remote `target`. `reason` must already be terminal
/// (`propagate_exit` converts Kill to Killed before its remote-link drain).
///
/// Absent connection: drop, which is not lossy for links — the peer's own
/// down-hook already delivered (or will deliver) noconnection for every link
/// to us (DC-3, both sides).
pub(super) fn send_exit_linked(
    shared: &SharedState,
    from_pid: u64,
    target: RemotePid,
    reason: ExitReason,
) {
    send_exit(shared, ControlOp::Exit, from_pid, target, reason);
}

/// Send a wire EXIT2 (op 8, `exit/2`) from local `caller_pid` to remote
/// `target`. Best-effort fire-and-forget (ruling 7): delivered iff the pinned
/// connection stays up — no connection, overflow, or down ⇒ dropped, with no
/// backstop claimed. MAY carry `Kill` (untrappable at the receiver).
pub(super) fn send_exit2(
    shared: &SharedState,
    caller_pid: u64,
    target: RemotePid,
    reason: ExitReason,
) {
    send_exit(shared, ControlOp::Exit2, caller_pid, target, reason);
}

fn send_exit(
    shared: &SharedState,
    op: ControlOp,
    from_pid: u64,
    target: RemotePid,
    reason: ExitReason,
) {
    if !wire_encodable(from_pid, target) {
        // EXIT (op 3): defensive only — link endpoints are wire-encodable by
        // construction (`send_link` refuses oversized pids; inbound endpoints
        // decode from u32 wire fields), so no link-exit can reach this arm.
        // EXIT2 (op 8): best-effort fire-and-forget (ruling 7) — dropping an
        // undeliverable signal is its contract; tearing the connection down
        // per control is not.
        return;
    }
    let Some(sender) = &shared.dist_sender else {
        return;
    };
    let Some(connection) = shared.distribution_connections.get_connection(target.node) else {
        return;
    };
    let frame = encode_exit_frame(
        op,
        shared.local_node.name,
        from_pid,
        target,
        reason,
        &shared.atom_table,
    );
    enqueue_pinned(sender, connection, frame);
}

/// Whether both endpoint pids fit the wire's NEW_PID_EXT `u32` fields
/// (`encode_external_pid` rejects larger components). Without this boundary
/// guard, once a long-lived node's monotonic pid counter passes 2^32, every
/// control naming such a pid would fail encode and tear the pinned connection
/// down — a connect/noconnection/redial churn loop while the caller sees
/// success.
fn wire_encodable(from_pid: u64, target: RemotePid) -> bool {
    u32::try_from(from_pid).is_ok()
        && u32::try_from(target.pid_number).is_ok()
        && u32::try_from(target.serial).is_ok()
}

/// Hand one encoded control to the lane against its pinned connection.
fn enqueue_pinned(
    sender: &DistSender,
    connection: Arc<DistConnection>,
    frame: Result<Vec<u8>, EncodeError>,
) {
    match frame {
        Ok(frame) => {
            // Overflow already marked the pinned connection down inside
            // `enqueue_control`; Closed means scheduler teardown (peers
            // converge via EOF). Neither needs caller action (DC-1).
            let _ = sender.enqueue_control(ControlOutbound {
                connection,
                frame: Arc::from(frame.into_boxed_slice()),
            });
        }
        Err(_) => {
            // Unreachable for the inputs this module produces (pids are
            // guarded by `wire_encodable`; node and reason atoms come from
            // the interned table), but DC-1 has no silent arm: a control that
            // cannot be encoded must down the pinned connection so the
            // noconnection backstop supplies the death signal.
            connection.mark_down_control_overflow();
        }
    }
}
