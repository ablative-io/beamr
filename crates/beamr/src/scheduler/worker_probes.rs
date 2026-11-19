//! Test-support probes into worker parking, wakes, failures and suspension
//! bookkeeping.

#[cfg(all(feature = "threads", any(test, feature = "test-support")))]
use super::execution::{ObservedParkTimeout, WakeCauses, WorkerHold, WorkerParkCounts};
use super::{Scheduler, execution, lock_or_recover};
use std::sync::atomic::Ordering;

impl Scheduler {
    /// Cumulative idle parks across this scheduler's normal workers — one per
    /// `park_thread` entry. The sampling source for the signed §3.8 idle-wake
    /// bound; test-only instrumentation (`test-support`), absent from
    /// production builds.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn idle_park_count(&self) -> usize {
        self.shared.idle_parks.load(Ordering::Acquire)
    }
    /// The park timeout the workers are ACTUALLY using, in milliseconds, as
    /// the most recent park by any worker passed it to its wait: `Some` for a
    /// park with a deadline, `None` for a park with no timeout (or before the
    /// first park; [`Scheduler::observed_park_timeout`] tells those apart).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn observed_park_timeout_millis(&self) -> Option<u64> {
        self.shared.park_seams.observed_any().bounded_millis()
    }
    /// The timeout the most recent park by any worker waited with.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn observed_park_timeout(&self) -> ObservedParkTimeout {
        self.shared.park_seams.observed_any()
    }
    /// The timeout worker `worker` most recently waited with.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn worker_observed_park_timeout(&self, worker: usize) -> ObservedParkTimeout {
        self.shared.park_seams.observed(worker)
    }
    /// Why worker 0 woke by the clock, and how many replay cursor edges ran:
    /// every wake the signed §3.8 bound allows, counted under its cause.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn wake_causes(&self) -> WakeCauses {
        self.shared.park_seams.causes()
    }
    /// Block until worker `worker` waits in a park it entered after `since`
    /// was read from [`Scheduler::worker_park_counts`], and answer that
    /// park's timeout. Ends on the worker starting to wait, never a clock.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn await_worker_parked(
        &self,
        worker: usize,
        since: WorkerParkCounts,
    ) -> ObservedParkTimeout {
        execution::await_worker_parked(&self.shared, worker, since)
    }
    /// B-184's seam: make worker `worker` panic at its next park, where it is
    /// about to wait with the `wait_set` lock held and counted as parked
    /// (the place std's condvar assert panicked on macOS), with
    /// [`park_seam_panic_message`]. Runs edge E so a worker already waiting
    /// parks again. False, arming nothing, when there is no such worker.
    #[cfg(any(test, feature = "test-support"))]
    pub fn panic_worker_at_next_park(&self, worker: usize) -> bool {
        let armed = self.shared.park_seams.arm_panic_at_park(worker);
        if armed {
            self.shared.wake_edge();
        }
        armed
    }
    /// B-184's seam: make worker `worker` panic as it starts its next slice,
    /// with the popped process still `Present` and anything else it queued
    /// left on its local run queue, with [`slice_seam_panic_message`].
    /// False, arming nothing, when there is no such worker.
    #[cfg(any(test, feature = "test-support"))]
    pub fn panic_worker_at_next_slice(&self, worker: usize) -> bool {
        self.shared.park_seams.arm_panic_at_slice(worker)
    }
    /// B-184 R5's seam for a death no recovery can prove safe (tier 3):
    /// poisons a second scheduler-shared mutex, `link_set`, by a thread that
    /// panics holding it (joined here), then arms the park panic as
    /// [`Scheduler::panic_worker_at_next_park`] does. False, poisoning and
    /// arming nothing, when there is no such worker, and false, arming
    /// nothing, when the poisoning thread could not be started.
    #[cfg(any(test, feature = "test-support"))]
    pub fn panic_worker_unrecoverably(&self, worker: usize) -> bool {
        if worker >= self.shared.thread_count {
            return false;
        }
        execution::poison_link_set(&self.shared) && self.panic_worker_at_next_park(worker)
    }
    /// B-184 R5's hold (test builds): once worker `worker` has died at its
    /// park or in a slice, its thread waits, before its recovery checks
    /// anything, until the returned [`WorkerHold`] is dropped.
    #[cfg(test)]
    pub(crate) fn hold_worker_before_recovery(&self, worker: usize) -> Option<WorkerHold> {
        self.shared.park_seams.arm_hold_before_recovery(worker)
    }
    /// B-184's hold (lead amendment B): once the latch has recorded worker
    /// `worker` dead, its thread stays wedged inside its guard, before it
    /// finalizes anything or reports its own exit, until the returned
    /// [`WorkerHold`] is dropped. `None` when there is no such worker.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn hold_worker_after_death(&self, worker: usize) -> Option<WorkerHold> {
        self.shared.park_seams.arm_hold_after_death(worker)
    }
    /// Block until worker `worker`'s thread has reported its own exit, the
    /// last thing its guard does. Ends on the report, never a clock; false
    /// at once when there is no such worker.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn await_worker_exit_reported(&self, worker: usize) -> bool {
        self.shared.worker_exits.await_reported(worker)
    }
    /// The workers shutdown has not joined after the latch, as its log line
    /// named them: `"beamr-sched-N (why)"`.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn shutdown_skipped_workers(&self) -> Vec<String> {
        lock_or_recover(&self.shared.shutdown_skipped).clone()
    }
    /// Block until a B-184 seam has panicked worker `worker` (at least once:
    /// the mark is never cleared). Ends on the seam's mark, never a clock;
    /// false at once when there is no such worker. After it returns the
    /// worker runs nothing more until its recovery restarts it (R5).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn await_seam_panicked(&self, worker: usize) -> bool {
        execution::await_seam_panicked(&self.shared, worker)
    }
    /// Block until every normal worker is waiting at once.
    #[cfg(any(test, feature = "test-support"))]
    pub fn await_all_parked(&self) {
        execution::await_all_parked(&self.shared);
    }
    /// Workers inside their idle window right now (§3.8.2 C1's idle count).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn idle_worker_count(&self) -> usize {
        self.shared.idle_wake.idle_workers()
    }
    /// Workers waiting on the wake condvar right now.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn parked_worker_count(&self) -> usize {
        self.shared.idle_wake.parked_workers()
    }
    /// Edges E run to date: every advance of the wake sequence.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn wake_edge_count(&self) -> u64 {
        self.shared.idle_wake.edges()
    }
    /// How many times worker `worker` parked, and how those parks ended.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn worker_park_counts(&self, worker: usize) -> WorkerParkCounts {
        self.shared.park_seams.counts(worker)
    }
    /// Cumulative suspension-mirror registrations across this scheduler —
    /// one per `register_suspension_mirror` call, counted at registration
    /// rather than inferred from the live mirror map. Boundary instrument
    /// for the §3.2 refusal ordering: a refused dirty call must leave this
    /// unmoved, which distinguishes refusal-before-registration from
    /// refusal-after-registration-plus-cleanup. Test-only instrumentation
    /// (`test-support`), absent from production builds.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn suspension_mirror_registration_count(&self) -> u64 {
        self.shared
            .suspension_mirror_registrations
            .load(Ordering::Acquire)
    }
    /// Cumulative entries into the dirty-call gated-suspension side-effect
    /// sequence (first step: call-id allocation). Paired with
    /// [`Self::suspension_mirror_registration_count`], the two instruments
    /// bracket the whole sequence, so the §3.2 refusal gates catch a check
    /// that regresses to anywhere inside it. Test-only instrumentation
    /// (`test-support`), absent from production builds.
    /// Live dirty completion-bridge threads this scheduler has spawned —
    /// 0 after a drained shutdown (spec §4 step 3). Test-support instrument.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn dirty_completions_live_count(&self) -> usize {
        self.shared.dirty_completions_live_count()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn dirty_suspension_allocation_count(&self) -> u64 {
        self.shared
            .dirty_suspension_allocations
            .load(Ordering::Acquire)
    }
}
