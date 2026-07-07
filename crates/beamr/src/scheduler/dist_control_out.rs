//! Outbound distribution controls: encode on the calling worker, pin the
//! current connection generation, hand off to the must-deliver control lane.
//! Never blocks, never dials (C7).
//!
//! Common body shape (shared by every sender this module grows): resolve the
//! target's connection ONCE at enqueue time — the DC-2 generation pin — then
//! encode via `control_link` and `DistSender::enqueue_control`. A full lane
//! needs no handling by the caller: `enqueue_control` has already marked the
//! pinned connection down (`ControlOverflow`), and the inline down-hook's
//! noconnection backstop supplies the signals (DC-1(b)/DC-3). An encode
//! failure — unreachable for atom/u64 inputs — also downs the pinned
//! connection: DC-1 has no silent arm.
//!
//! Only the link-exit sender exists yet (it serves the inbound LINK-to-dead
//! noproc reply); the LINK/UNLINK/EXIT2 senders land with the outbound
//! facility rewiring that retires `ControlRouter`, which gives them their
//! first production callers.

use std::sync::Arc;

use crate::distribution::control_link::{ControlOp, encode_exit_frame};
use crate::distribution::sender::ControlOutbound;
use crate::process::{ExitReason, RemotePid};

use super::SharedState;

/// Send a wire EXIT (op 3, link-exit) from local endpoint `from_pid` — dead
/// or dying — to remote `target`. `reason` must already be terminal
/// (`propagate_exit` converts Kill to Killed before its remote-link drain).
///
/// Replay mode (`dist_sender` is `None`): no-op — distribution is globally
/// off in replay. Absent connection: drop, which is not lossy for links —
/// the peer's own down-hook already delivered (or will deliver) noconnection
/// for every link to us (DC-3, both sides).
pub(super) fn send_exit_linked(
    shared: &SharedState,
    from_pid: u64,
    target: RemotePid,
    reason: ExitReason,
) {
    let Some(sender) = &shared.dist_sender else {
        return;
    };
    // DC-2 pin: resolve once, at enqueue; the drain writes only to this
    // connection generation and skips it once down.
    let Some(connection) = shared.distribution_connections.get_connection(target.node) else {
        return;
    };
    match encode_exit_frame(
        ControlOp::Exit,
        shared.local_node.name,
        from_pid,
        target,
        reason,
        &shared.atom_table,
    ) {
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
