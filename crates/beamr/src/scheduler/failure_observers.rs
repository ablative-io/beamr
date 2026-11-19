//! How an embedder learns a scheduler has failed: the latched failure,
//! callbacks on failure and on worker death, and the count of deaths.

use super::Scheduler;
use super::failure::{SchedulerFailure, WorkerDeath};

impl Scheduler {
    /// The scheduler's permanent failure, or `None` while every worker is
    /// running (B-184). Latched by the first worker death the recovery cannot
    /// account for (R5's tier 3: a death anywhere but parked or running a
    /// process, a poisoned shared mutex no check proves consistent, a failed
    /// wait-set check, a loop that returned before shutdown, or a restarted
    /// worker dying again before a slice); never cleared. Once it
    /// is set the scheduler has stopped: shutdown is set, every worker is
    /// leaving or has left its loop, processes are being finalized, new
    /// spawns and other admitted mutations are refused, and
    /// [`Scheduler::run_until_exit`] returns it. The embedder decides how its
    /// process ends; beamr never aborts it.
    #[must_use]
    pub fn failure(&self) -> Option<SchedulerFailure> {
        self.shared.failure.get().cloned()
    }
    /// Register the one callback the failure latch runs (B-184, lead
    /// amendment B), only when a tier-3 death latches it (R5). It runs
    /// exactly once: on the dying worker's own thread,
    /// inside its guard, before the other workers and the waiters are woken;
    /// or, registered after the latch, at once on this thread. Returns false,
    /// dropping `callback` unrun, when a callback was registered before.
    ///
    /// The callback runs on a scheduler worker thread: it must not wait on
    /// anything only that worker could provide. [`Scheduler::shutdown`] from
    /// inside it returns without joining the dying worker.
    pub fn on_failure(&self, callback: impl FnOnce(&SchedulerFailure) + Send + 'static) -> bool {
        self.shared.failure.register(Box::new(callback))
    }
    /// Register the one callback every worker death is reported to, of
    /// every tier (B-184 R5, lead amendment A): run on the dying worker's
    /// own thread, once per death, after the recovery decided the tier and
    /// before the worker restarts or the failure latches. A restart is
    /// reported exactly as loudly as a stop. Returns false, dropping
    /// `callback`, when one was registered before. A death before the
    /// registration is not replayed: [`Scheduler::worker_deaths`] counts
    /// every death.
    ///
    /// The callback runs on a scheduler worker thread: it must not wait on
    /// anything only that worker could provide.
    pub fn on_worker_death(&self, callback: impl Fn(&WorkerDeath) + Send + Sync + 'static) -> bool {
        self.shared.failure.deaths.register(Box::new(callback))
    }
    /// How many scheduler workers have died, of every tier (B-184 R5).
    #[must_use]
    pub fn worker_deaths(&self) -> u64 {
        self.shared.failure.deaths.count()
    }
}
