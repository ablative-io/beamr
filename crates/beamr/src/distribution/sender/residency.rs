use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::DistOutbound;

/// Byte meter for one lane's resident frames.
///
/// A leaf: it holds a counter and a budget and nothing else. That is what keeps
/// it safe to hand to every in-flight frame — a charge can outlive the sender
/// without keeping the sender (and therefore its runtime) alive, so the
/// `Arc`-cycle invariant in the module docs is untouched by this accounting.
pub(super) struct LaneResidency {
    /// Bytes currently charged to the lane.
    charged: AtomicUsize,
    /// Ceiling `charged` may not exceed.
    budget: usize,
}

impl LaneResidency {
    pub(super) const fn new(budget: usize) -> Self {
        Self {
            charged: AtomicUsize::new(0),
            budget,
        }
    }

    /// Reserve `bytes` if they fit, returning the RAII charge that releases
    /// them. `None` means the budget is exhausted — the caller must refuse.
    ///
    /// Lock-free and wait-free-ish by construction (a CAS retry loop over a
    /// single word), because the enqueue path it sits on must never block a
    /// scheduler worker.
    pub(super) fn try_charge(self: &Arc<Self>, bytes: usize) -> Option<ResidencyCharge> {
        let mut current = self.charged.load(Ordering::Relaxed);
        loop {
            // Saturating, not wrapping: a hypothetical overflow must refuse,
            // never wrap around into a spuriously-available budget.
            let next = current.saturating_add(bytes);
            if next > self.budget {
                return None;
            }
            match self.charged.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(ResidencyCharge {
                        residency: Arc::clone(self),
                        bytes,
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }

    /// Bytes currently charged.
    pub(super) fn resident_bytes(&self) -> usize {
        self.charged.load(Ordering::Acquire)
    }
}

/// An outstanding byte reservation, released on drop.
///
/// Release is tied to `Drop` rather than to an explicit call at each exit
/// because a LEAKED reservation is a slow-starve: bytes that are never returned
/// shrink the budget permanently until the lane refuses everything. Drop covers
/// every exit path there is — the drain finishing a write, a write error, a
/// write timeout, the pinned connection being marked down, a `try_send` that
/// bounces the item straight back, and the receiver being dropped at shutdown
/// with items still queued.
pub(super) struct ResidencyCharge {
    residency: Arc<LaneResidency>,
    bytes: usize,
}

impl Drop for ResidencyCharge {
    fn drop(&mut self) {
        self.residency
            .charged
            .fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// A data-lane item travelling with its byte reservation.
///
/// The charge rides INSIDE the channel so the reservation's lifetime is exactly
/// the item's residency in the lane, with no exit path able to skip the
/// release.
pub(super) struct ChargedOutbound {
    pub(super) item: DistOutbound,
    pub(super) charge: ResidencyCharge,
}
