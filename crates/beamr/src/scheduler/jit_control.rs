//! The JIT: building the per-scheduler compiler, and the operator switch
//! and handles the scheduler exposes.

use super::{Scheduler, SharedState};
use crate::jit::{JitCache, JitCompiler, JitProfiler, JitSettings};
use std::sync::Arc;
use std::sync::atomic::Ordering;

impl SharedState {
    /// The operator JIT switch, read once per native-services assembly.
    ///
    /// Set only through [`Scheduler::set_jit_enabled`] — the engine never
    /// writes it.
    pub(in crate::scheduler) fn jit_enabled(&self) -> bool {
        self.jit_enabled.load(Ordering::Acquire)
    }
}

impl Scheduler {
    /// Turns the JIT on or off for this scheduler at runtime, returning the
    /// previous setting.
    ///
    /// **This is an operator control, not an engine behaviour.** Nothing inside
    /// beamr ever calls it: a runtime that disabled its own JIT on a detected
    /// fault would be a silent fallback, and that is a design decision for an
    /// embedder to make explicitly rather than a switch the engine may flip
    /// behind an operator's back.
    ///
    /// Disabling withholds both the JIT cache and the profiling handle from the
    /// call edges for every subsequent slice, which is the engine's established
    /// disable mechanism (see `build_native_services`). Two consequences worth
    /// stating separately, because a switch that only did the first would be a
    /// false comfort while cached code kept running:
    /// - no function is compiled while the switch is off, and
    /// - **already-compiled code is not entered** — the cache survives, is not
    ///   consulted, and becomes live again if the switch is turned back on.
    ///
    /// The switch takes effect at the next slice boundary, so a call already
    /// executing compiled code runs to its yield. To guarantee that nothing is
    /// ever compiled, disable immediately after constructing the scheduler and
    /// before spawning any process.
    ///
    /// Note that [`SchedulerConfig::jit_threshold`](super::SchedulerConfig::jit_threshold) cannot serve as this
    /// control. A very large threshold does defer compilation past any
    /// realistic workload, so it works as a *workaround* on a scheduler not yet
    /// built — but it is fixed at construction and, decisively, it says nothing
    /// about entering code that is **already compiled**: once a function is in
    /// the cache, no threshold keeps execution out of it. That is the case this
    /// switch exists for, and it is the case an operator actually meets, since
    /// by the time a JIT fault is diagnosed the code is already cached.
    pub fn set_jit_enabled(&self, enabled: bool) -> bool {
        self.shared.jit_enabled.swap(enabled, Ordering::AcqRel)
    }

    /// Whether the JIT is currently enabled for this scheduler.
    #[must_use]
    pub fn jit_enabled(&self) -> bool {
        self.shared.jit_enabled.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn jit_profiler(&self) -> &Arc<JitProfiler> {
        &self.shared.jit_profiler
    }
    #[must_use]
    pub fn jit_cache(&self) -> &Arc<JitCache> {
        &self.shared.jit_cache
    }
}

// Scheduler construction runs on the caller's thread, so a thread-local
// injection scopes the failure to the test that set it — parallel tests
// constructing schedulers are untouched.
#[cfg(test)]
thread_local! {
    pub(super) static INJECT_JIT_COMPILER_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Builds the per-scheduler JIT compiler. Cranelift cannot be made to fail
/// naturally on a supported host, so the constructor-error propagation is
/// pinned through the test-only injection above.
pub(super) fn build_jit_compiler() -> Result<JitCompiler, crate::jit::JitError> {
    #[cfg(test)]
    if INJECT_JIT_COMPILER_FAILURE.with(std::cell::Cell::get) {
        return Err(crate::jit::JitError::CraneliftError(
            "injected construction failure (test seam)".to_owned(),
        ));
    }
    JitCompiler::new(JitSettings)
}
