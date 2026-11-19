//! Configuration a threaded scheduler is constructed from.

use crate::distribution::DistributionConfig;
use crate::io::RingConfig;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Default)]
pub struct SchedulerConfig {
    pub thread_count: Option<usize>,
    /// Dirty CPU pool sizing (spec §3.2).
    ///
    /// - `None` — legacy default: one worker per core (`num_cpus`).
    /// - `Some(n)` with `n > 0` — an owned pool of `n` workers.
    /// - `Some(0)` — the pool is **Disabled**: zero threads, zero fds, and a
    ///   dirty CPU call is refused with a typed service-unavailable error
    ///   before any suspension or queue side effect.
    ///
    /// **Behavior change (spec §6):** `Some(0)` previously coerced to a
    /// one-worker pool; it now disables the pool. Requesting zero workers and
    /// still dispatching dirty work is a composition error, surfaced loudly at
    /// the calling process rather than silently papered over.
    pub dirty_cpu_threads: Option<usize>,
    /// Dirty IO pool sizing. Same semantics as [`dirty_cpu_threads`](Self::dirty_cpu_threads):
    /// `None` is the legacy default (10 workers), `Some(n > 0)` owns `n`,
    /// `Some(0)` disables the pool (was coerced to 1 before — spec §6).
    pub dirty_io_threads: Option<usize>,
    pub dirty_queue_depth: Option<usize>,
    pub io: Option<RingConfig>,
    pub node_name: Option<String>,
    pub creation: Option<u32>,
    pub distribution: Option<DistributionConfig>,
    /// Hot-call count at which a function is submitted for compilation
    /// (`None` = [`DEFAULT_JIT_THRESHOLD`](crate::jit::DEFAULT_JIT_THRESHOLD)).
    ///
    /// **This is a tuning knob, not an off-switch.** The hotness counter
    /// increments with `saturating_add` and trips on `count >= threshold`, so
    /// no value forbids compilation outright: `Some(u32::MAX)` merely defers it
    /// until the counter saturates at `u32::MAX` recorded calls *to that MFA*,
    /// which is beyond any realistic workload but is deferral, not refusal.
    ///
    /// Two things it cannot do at all, which is why
    /// [`Scheduler::set_jit_enabled`](super::Scheduler::set_jit_enabled) exists: it is fixed at construction, so
    /// it cannot be changed on a live scheduler; and it governs only whether
    /// code is *compiled*, never whether **already-compiled code is entered** —
    /// once a function is in the cache the threshold is irrelevant to it.
    pub jit_threshold: Option<u32>,
    /// Minimum interval between per-process telemetry samples at scheduler slice boundaries.
    pub telemetry_sample_interval: Option<Duration>,
    /// Embedder-supplied private data handed to every native call via
    /// [`crate::native::ProcessContext::nif_private_data`] — the ERTS
    /// `enif_priv_data` equivalent, scoped to this scheduler instance so
    /// embedders hosting several runtimes in one OS process never need
    /// process-wide globals.
    pub nif_private_data: Option<Arc<dyn std::any::Any + Send + Sync>>,
}

impl std::fmt::Debug for SchedulerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchedulerConfig")
            .field("thread_count", &self.thread_count)
            .field("dirty_cpu_threads", &self.dirty_cpu_threads)
            .field("dirty_io_threads", &self.dirty_io_threads)
            .field("dirty_queue_depth", &self.dirty_queue_depth)
            .field("io", &self.io)
            .field("node_name", &self.node_name)
            .field("creation", &self.creation)
            .field("distribution", &self.distribution)
            .field("jit_threshold", &self.jit_threshold)
            .field("telemetry_sample_interval", &self.telemetry_sample_interval)
            .field(
                "nif_private_data",
                &self.nif_private_data.as_ref().map(|_| ".."),
            )
            .finish()
    }
}
