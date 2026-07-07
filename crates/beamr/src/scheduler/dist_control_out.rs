//! Outbound distribution controls: encode on the calling worker, pin the
//! current connection generation, hand off to the must-deliver control lane.
//! Never blocks, never dials (C7).
//!
//! Common body shape (shared by every sender here): resolve the target's
//! connection ONCE at enqueue time — the DC-2 generation pin — then encode
//! via `control_link` and `DistSender::enqueue_control`. A full lane needs no
//! handling by the caller: `enqueue_control` has already marked the pinned
//! connection down (`ControlOverflow`), and the inline down-hook's
//! noconnection backstop supplies the signals (DC-1(b)/DC-3). An encode
//! failure — unreachable for atom/u64 inputs — also downs the pinned
//! connection: DC-1 has no silent arm.
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
/// recorded in the wire spec).
pub(super) fn send_link(
    shared: &SharedState,
    caller_pid: u64,
    target: RemotePid,
) -> Result<(), RemoteLinkError> {
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
            // Unreachable for atom/u64 inputs, but DC-1 has no silent arm: a
            // control that cannot be encoded must down the pinned connection
            // so the noconnection backstop supplies the death signal.
            connection.mark_down_control_overflow();
        }
    }
}
