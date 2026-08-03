use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::distribution::etf::MAX_DIST_FRAME_BYTES;

/// Inbound receive-residency ENVELOPE, in bytes: the 4 GiB spike allowance out
/// of beamr's 13 GiB memory bound. This is the primitive quantity — the budget
/// the accept path is allowed to spend.
///
/// `u64` rather than `usize` deliberately: 4 GiB does not fit a 32-bit `usize`,
/// and the `connection` module is compiled for wasm32 (see
/// [`FrameError::LengthOverflow`](super::frame::FrameError::LengthOverflow)).
pub(super) const INBOUND_RESIDENCY_ENVELOPE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Worst-case inbound receive residency attributable to ONE peer: a single
/// framed buffer, i.e. [`MAX_DIST_FRAME_BYTES`], which is precisely what the
/// per-frame cap bounds.
///
/// This is the CURRENCY the accept path spends. The peer ceiling it implies is
/// `INBOUND_RESIDENCY_ENVELOPE_BYTES / INBOUND_RESIDENCY_PER_PEER_BYTES` — the
/// 64-peer design target, now DERIVED from the two byte quantities rather than
/// configured as a count. No peer count is written anywhere in the
/// `connection` module.
pub(super) const INBOUND_RESIDENCY_PER_PEER_BYTES: u64 = MAX_DIST_FRAME_BYTES as u64;

/// Byte meter for inbound accept-side residency.
///
/// A leaf: counters and a ceiling, nothing more. That is what lets a permit be
/// held by a per-connection lifecycle task without the task keeping the manager
/// alive — [`DistConnection::manager`](super::link::DistConnection) is already
/// `Weak` for the same reason,
/// and this accounting must not reintroduce the ownership edge that `Weak`
/// exists to break.
pub(super) struct InboundResidency {
    /// Bytes currently reserved by admitted inbound links.
    pub(super) charged: AtomicU64,
    /// Ceiling `charged` may not exceed.
    envelope: u64,
    /// Bytes one admitted inbound link reserves.
    per_peer: u64,
    /// Accepted streams declined for want of envelope, since construction.
    /// Counter-plus-accessor, matching `heartbeat_tasks_spawned`: a refusal is
    /// an async event with no thread of its own, so it is inventoried as a
    /// count.
    pub(super) refused: AtomicU64,
}

impl InboundResidency {
    pub(super) fn new() -> Self {
        Self {
            charged: AtomicU64::new(0),
            envelope: INBOUND_RESIDENCY_ENVELOPE_BYTES,
            per_peer: INBOUND_RESIDENCY_PER_PEER_BYTES,
            refused: AtomicU64::new(0),
        }
    }

    /// Reserve one peer's worth of residency, or refuse.
    ///
    /// `None` means admitting this peer would carry inbound residency past the
    /// envelope. Lock-free (a CAS retry over one word) because this runs inline
    /// on the accept loop, which must keep accepting.
    pub(super) fn try_admit(self: &Arc<Self>) -> Option<InboundAdmissionPermit> {
        let mut current = self.charged.load(Ordering::Relaxed);
        loop {
            // Saturating: an overflow must refuse, never wrap into a
            // spuriously-available envelope.
            let next = current.saturating_add(self.per_peer);
            if next > self.envelope {
                self.refused.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            match self.charged.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(InboundAdmissionPermit {
                        residency: Arc::clone(self),
                        bytes: self.per_peer,
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }
}

/// One admitted inbound link's residency reservation, released on drop.
///
/// Held by the link's read-lifecycle task, which runs for exactly as long as
/// the link does: it exits on peer EOF, on a read error, or when `mark_down`
/// fires the shutdown `Notify`. Release is tied to `Drop` rather than to an
/// explicit call at each of those exits because a leaked reservation is a
/// slow-starve — it would shrink the envelope permanently until the node
/// refused every new peer. Drop also covers a task dropped wholesale at runtime
/// teardown.
pub(super) struct InboundAdmissionPermit {
    residency: Arc<InboundResidency>,
    bytes: u64,
}

impl Drop for InboundAdmissionPermit {
    fn drop(&mut self) {
        self.residency
            .charged
            .fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
