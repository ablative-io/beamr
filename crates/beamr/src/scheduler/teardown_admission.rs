//! Teardown admission: the reservation a dirty submission or a mutating
//! facility operation holds so the shutdown drain waits for it, and the drain
//! that closes intake and joins every completion bridge.

use super::{SharedState, lock_or_recover};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct TeardownAdmissionRegistry {
    /// Set by the shutdown drain; reservations are refused from here on.
    pub(super) closed: bool,
    /// Admissions that have not yet published a bridge handle or released.
    pub(super) reserved: usize,
    /// Retained bridge thread handles, OS-joined by the drain; finished
    /// handles are reaped opportunistically at each publish.
    bridges: Vec<std::thread::JoinHandle<()>>,
}

/// RAII teardown-admission token (spec §4 step 3), LINEARIZED with the
/// shutdown drain under one lock. Two uses:
///
/// - A dirty submission acquires one BEFORE any suspension side effect and
///   CONVERTS it (via [`publish`](Self::publish)) into a retained, joinable
///   bridge handle at spawn; every pre-spawn error path releases it via
///   `Drop`.
/// - A mutating facility operation (spawn family, io-message delivery, link)
///   acquires one and HOLDS it across its whole mutation, releasing via
///   `Drop` — so a mutation admitted before the drain closes intake finishes
///   BEFORE shutdown returns (the drain waits), and one attempted after
///   refuses. A snapshot predicate cannot give this guarantee: it admits a
///   delayed mutation that lands after shutdown has returned.
pub(in crate::scheduler) struct TeardownAdmission {
    shared: Arc<SharedState>,
    published: bool,
}

impl TeardownAdmission {
    /// Convert this reservation into a retained bridge handle.
    pub(in crate::scheduler) fn publish(mut self, handle: std::thread::JoinHandle<()>) {
        let mut registry = lock_or_recover(&self.shared.dirty_completions);
        registry.reserved = registry.reserved.saturating_sub(1);
        // Opportunistic reap: completed bridges cost one `is_finished` check
        // here, so the retained vec tracks in-flight bridges, not history.
        let retained = std::mem::take(&mut registry.bridges);
        for bridge in retained {
            if bridge.is_finished() {
                let _ = bridge.join();
            } else {
                registry.bridges.push(bridge);
            }
        }
        registry.bridges.push(handle);
        drop(registry);
        self.shared.dirty_completions_changed.notify_all();
        self.published = true;
    }
}

impl Drop for TeardownAdmission {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        let mut registry = lock_or_recover(&self.shared.dirty_completions);
        registry.reserved = registry.reserved.saturating_sub(1);
        drop(registry);
        self.shared.dirty_completions_changed.notify_all();
    }
}

impl SharedState {
    /// Reserve admission for one dirty submission, refused once intake is
    /// closed. Closure and reservation share ONE lock, so a reservation
    /// cannot slip behind the shutdown drain: the drain marks `closed` and
    /// waits out every earlier reservation under the same mutex.
    pub(in crate::scheduler) fn try_reserve_teardown_admission(
        self: &Arc<Self>,
    ) -> Option<TeardownAdmission> {
        let mut registry = lock_or_recover(&self.dirty_completions);
        // B-184: a latched failure has stopped the scheduler; nothing it
        // admits now would ever run.
        if registry.closed || self.failure.is_latched() {
            return None;
        }
        registry.reserved += 1;
        drop(registry);
        Some(TeardownAdmission {
            shared: Arc::clone(self),
            published: false,
        })
    }

    /// The shutdown channel each completion bridge selects on.
    pub(in crate::scheduler) fn dirty_completion_shutdown_channel(
        &self,
    ) -> crossbeam_channel::Receiver<()> {
        self.dirty_completion_shutdown_rx.clone()
    }

    /// §4 step 3 for the dirty pools: close intake (linearized with
    /// reservation — no new bridge can spawn behind this), wake every bridge
    /// by closing the shutdown channel, wait out in-flight reservations
    /// (each either releases on an error path or publishes its bridge
    /// handle), then OS-JOIN every bridge. Bounded: woken bridges exit
    /// without waiting for their jobs. After this returns, no bridge THREAD
    /// of this scheduler exists at the OS level, so nothing can deliver a
    /// completion into the scheduler being torn down. Idempotent.
    pub(in crate::scheduler) fn drain_dirty_completions(&self) {
        let mut registry = lock_or_recover(&self.dirty_completions);
        registry.closed = true;
        drop(registry);
        drop(lock_or_recover(&self.dirty_completion_shutdown_tx).take());
        let mut registry = lock_or_recover(&self.dirty_completions);
        while registry.reserved > 0 {
            #[cfg(test)]
            if let Some(hook) = self.teardown_wait_hook.get() {
                hook();
            }
            registry = self
                .dirty_completions_changed
                .wait(registry)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        let bridges = std::mem::take(&mut registry.bridges);
        drop(registry);
        for bridge in bridges {
            // A joined bridge has fully exited at the OS level; a panic
            // payload has nothing to recover during teardown.
            let _ = bridge.join();
        }
    }

    /// Unjoined-bridge count: in-flight reservations plus retained handles
    /// (finished handles are reaped opportunistically at each publish).
    /// 0 after a drained shutdown. Reached via
    /// [`Scheduler::dirty_completions_live_count`].
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn dirty_completions_live_count(&self) -> usize {
        let registry = lock_or_recover(&self.dirty_completions);
        registry.reserved + registry.bridges.len()
    }
}
