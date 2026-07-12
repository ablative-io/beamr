//! Hot-function profiling for adaptive JIT compilation.

use crate::atom::Atom;
use dashmap::DashMap;
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

const STATE_INTERPRETING: u8 = 0;
const STATE_PENDING: u8 = 1;
const STATE_COMPILED: u8 = 2;
const STATE_UNSUPPORTED: u8 = 3;

/// Default number of interpreted calls before a function becomes eligible for JIT compilation.
///
/// The value is chosen to amortise Cranelift compilation cost for functions called in tight loops;
/// [`JitProfiler::tune_threshold`] may adjust it at runtime when benchmark data shows a different
/// compilation-cost/speedup trade-off for the current host.
pub const DEFAULT_JIT_THRESHOLD: u32 = 1000;
const MIN_TUNED_THRESHOLD: u32 = 100;
const MAX_TUNED_THRESHOLD: u32 = 10_000;

/// Module/function/arity key for per-function JIT state.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct MfaKey {
    /// Module atom.
    pub module: Atom,
    /// Function atom.
    pub function: Atom,
    /// Function arity.
    pub arity: u8,
}

impl MfaKey {
    /// Creates a new MFA key.
    #[must_use]
    pub fn new(module: Atom, function: Atom, arity: u8) -> Self {
        Self {
            module,
            function,
            arity,
        }
    }
}

struct FunctionProfile {
    counter: AtomicU32,
    state: AtomicU8,
    generation: AtomicU64,
}

impl FunctionProfile {
    fn new(generation: u64) -> Self {
        Self {
            counter: AtomicU32::new(0),
            state: AtomicU8::new(STATE_INTERPRETING),
            generation: AtomicU64::new(generation),
        }
    }
}

/// Result of recording one interpreted function call.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RecordResult {
    /// Continue interpreting without starting a compilation job.
    Continue,
    /// The function reached the hot threshold and should be compiled now.
    CompileNow,
}

/// Snapshot of the compile-outcome counters.
///
/// The counters are plain atomics maintained unconditionally (submission
/// attempts at the call edges, job outcomes on the dirty-CPU workers); only
/// their telemetry-metric exposure is feature-gated, so refusal pins can
/// consume this snapshot in default builds.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct CompileOutcomeCounters {
    /// Compilation requests submitted toward the dirty-CPU service,
    /// including attempts the pool refused.
    pub submissions: u64,
    /// Jobs whose native code reached the cache.
    pub successes: u64,
    /// Jobs refused by the JIT tier as permanently unsupported.
    pub unsupported: u64,
    /// Transient failures: compile errors that reset the profile, refused
    /// submissions, and completions whose publication was refused because
    /// the profile was deleted or re-heated at a newer generation — all
    /// leave the function free to re-heat if it is still called.
    pub transient_failures: u64,
}

/// Per-function hotness profiler for JIT compilation decisions.
pub struct JitProfiler {
    threshold: AtomicU32,
    /// Per-MFA hotness state.
    ///
    /// Growth bound: one slot (two small counters plus a generation stamp) per
    /// distinct MFA ever recorded at a live call edge — an MFA only enters the
    /// map by being interpreted there. Generation stamping reuses a slot across
    /// hot reloads rather than accumulating per-generation entries, and
    /// [`JitProfiler::remove_module`] drops a module's slots at the scheduler's
    /// delete seam. Modules deleted WITHOUT that seam (embedders mutating a raw
    /// `ModuleRegistry` directly) linger until process end, bounded by that
    /// recorded-MFA count.
    profiles: DashMap<MfaKey, FunctionProfile>,
    submissions: AtomicU64,
    successes: AtomicU64,
    unsupported: AtomicU64,
    transient_failures: AtomicU64,
}

impl JitProfiler {
    /// Creates a profiler with the supplied threshold.
    #[must_use]
    pub fn new(threshold: u32) -> Self {
        Self {
            threshold: AtomicU32::new(threshold.max(1)),
            profiles: DashMap::new(),
            submissions: AtomicU64::new(0),
            successes: AtomicU64::new(0),
            unsupported: AtomicU64::new(0),
            transient_failures: AtomicU64::new(0),
        }
    }

    /// Returns a snapshot of the compile-outcome counters.
    #[must_use]
    pub fn compile_outcome_counters(&self) -> CompileOutcomeCounters {
        CompileOutcomeCounters {
            submissions: self.submissions.load(Ordering::Acquire),
            successes: self.successes.load(Ordering::Acquire),
            unsupported: self.unsupported.load(Ordering::Acquire),
            transient_failures: self.transient_failures.load(Ordering::Acquire),
        }
    }

    pub(crate) fn note_submission(&self) {
        self.submissions.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn note_success(&self) {
        self.successes.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn note_unsupported(&self) {
        self.unsupported.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn note_transient_failure(&self) {
        self.transient_failures.fetch_add(1, Ordering::AcqRel);
    }

    /// Test-support probe: the recorded interpreted-call count for an MFA.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn recorded_call_count(&self, module: Atom, function: Atom, arity: u8) -> Option<u32> {
        self.profiles
            .get(&MfaKey::new(module, function, arity))
            .map(|profile| profile.counter.load(Ordering::Acquire))
    }

    /// Test-support probe: the number of live profile entries.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn profile_entry_count(&self) -> usize {
        self.profiles.len()
    }

    /// Returns the current compilation threshold.
    #[must_use]
    pub fn current_threshold(&self) -> u32 {
        self.threshold.load(Ordering::Acquire)
    }

    /// Returns the current compilation threshold.
    #[must_use]
    pub fn threshold(&self) -> u32 {
        self.current_threshold()
    }

    /// Adjusts the hot-call threshold from observed compilation cost and speedup.
    ///
    /// Fast compilation with a strong speedup compiles sooner; slow compilation or weak speedup
    /// compiles less eagerly. Tuned values are clamped to a production-safe envelope.
    pub fn tune_threshold(&self, compilation_time_us: u64, speedup_factor: f64) {
        let current = self.current_threshold();
        let tuned = if speedup_factor > 2.0 && compilation_time_us < 10_000 {
            current.saturating_mul(3).saturating_add(3) / 4
        } else if speedup_factor < 1.5 || compilation_time_us > 100_000 {
            current.saturating_mul(5).saturating_add(3) / 4
        } else {
            current
        };
        self.threshold
            .store(clamp_tuned_threshold(tuned), Ordering::Release);
    }

    /// Records a call to an MFA at a module generation without blocking on compilation work.
    ///
    /// The profile stamps the generation it heats at. A call from a NEWER generation is a fresh
    /// function: the profile resets to INTERPRETING at the new generation with a zeroed counter
    /// before counting, so a previously-COMPILED function re-heats and a previously-UNSUPPORTED
    /// function retries after a hot load. A call from an OLDER generation than the stamp neither
    /// resets nor counts — the stamped generation's heat is the only heat one profile tracks.
    pub fn record_call(
        &self,
        module: Atom,
        function: Atom,
        arity: u8,
        generation: u64,
    ) -> RecordResult {
        let key = MfaKey::new(module, function, arity);
        let profile = self
            .profiles
            .entry(key)
            .or_insert_with(|| FunctionProfile::new(generation));

        let stamped = profile.generation.load(Ordering::Acquire);
        if generation < stamped {
            return RecordResult::Continue;
        }
        if generation > stamped {
            profile.generation.store(generation, Ordering::Release);
            profile.state.store(STATE_INTERPRETING, Ordering::Release);
            profile.counter.store(0, Ordering::Release);
        }

        if profile.state.load(Ordering::Acquire) != STATE_INTERPRETING {
            return RecordResult::Continue;
        }

        let new_count = profile
            .counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                Some(count.saturating_add(1))
            })
            .map_or(1, |previous| previous.saturating_add(1));

        if new_count < self.current_threshold() {
            return RecordResult::Continue;
        }

        match profile.state.compare_exchange(
            STATE_INTERPRETING,
            STATE_PENDING,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => RecordResult::CompileNow,
            Err(_) => RecordResult::Continue,
        }
    }

    /// Marks a pending or interpreted function as compiled at a generation.
    ///
    /// No-ops when the profile's stamped generation is newer: a completion for an older generation
    /// must not stamp state onto code that no longer runs.
    pub fn mark_compiled(&self, module: Atom, function: Atom, arity: u8, generation: u64) {
        self.set_state(module, function, arity, generation, STATE_COMPILED);
    }

    /// Publishes a compilation result and marks the profile COMPILED as one step, serialized on
    /// the profile's entry guard. `publish` (the cache insert) runs ONLY while the guard is held
    /// and only if the profile still exists at a generation no newer than `generation` — so a
    /// concurrent [`Self::remove_module`] (the scheduler's delete seam, which runs before the
    /// cache invalidation) either observes the publication and the seam's cache invalidation
    /// removes it, or wins the guard first and the completion publishes nothing. Without this
    /// serialization a job completing during a delete could strand a cache entry that a same-name
    /// reload reaching the same generation number would execute.
    ///
    /// Returns whether the result was published.
    pub(crate) fn publish_compiled(
        &self,
        module: Atom,
        function: Atom,
        arity: u8,
        generation: u64,
        publish: impl FnOnce(),
    ) -> bool {
        let key = MfaKey::new(module, function, arity);
        let Some(profile) = self.profiles.get_mut(&key) else {
            return false;
        };
        if profile.generation.load(Ordering::Acquire) > generation {
            return false;
        }
        publish();
        profile.generation.store(generation, Ordering::Release);
        profile.state.store(STATE_COMPILED, Ordering::Release);
        true
    }

    /// Marks a function as permanently unsupported by this JIT tier at a generation.
    ///
    /// No-ops when the profile's stamped generation is newer (see [`Self::mark_compiled`]).
    pub fn mark_unsupported(&self, module: Atom, function: Atom, arity: u8, generation: u64) {
        self.set_state(module, function, arity, generation, STATE_UNSUPPORTED);
    }

    /// Returns whether an MFA is currently marked compiled.
    #[must_use]
    pub fn is_compiled(&self, module: Atom, function: Atom, arity: u8) -> bool {
        self.state_for(module, function, arity) == Some(STATE_COMPILED)
    }

    /// Returns whether an MFA is permanently unsupported by this JIT tier.
    #[must_use]
    pub fn is_unsupported(&self, module: Atom, function: Atom, arity: u8) -> bool {
        self.state_for(module, function, arity) == Some(STATE_UNSUPPORTED)
    }

    /// Resets a transient compilation failure at a generation so the function can heat up again.
    ///
    /// No-ops when the profile's stamped generation is newer (see [`Self::mark_compiled`]), and
    /// no-ops when no profile exists: completion paths never recreate an entry — a missing
    /// profile means the module was deleted (the scheduler seam dropped it), and resurrecting it
    /// would strand a stale generation stamp against a name whose registry generations restart.
    pub fn reset_counter(&self, module: Atom, function: Atom, arity: u8, generation: u64) {
        let key = MfaKey::new(module, function, arity);
        let Some(profile) = self.profiles.get_mut(&key) else {
            return;
        };
        if profile.generation.load(Ordering::Acquire) > generation {
            return;
        }
        profile.generation.store(generation, Ordering::Release);
        profile.counter.store(0, Ordering::Release);
        profile.state.store(STATE_INTERPRETING, Ordering::Release);
    }

    /// Drops every profile entry for a module.
    ///
    /// Invoked from the scheduler's module-delete seam so a deleted module's hotness state does
    /// not linger; entries for modules deleted outside that seam are bounded per the map's rustdoc.
    pub fn remove_module(&self, module: Atom) {
        self.profiles.retain(|key, _| key.module != module);
    }

    /// Completion-mark write path: no-ops when the stamped generation is newer AND when no
    /// profile exists (see [`Self::reset_counter`] — completions never resurrect deleted
    /// profiles). Only [`Self::record_call`] creates entries: an MFA enters the map by being
    /// interpreted at a live call edge, never by a completion arriving after deletion.
    fn set_state(&self, module: Atom, function: Atom, arity: u8, generation: u64, state: u8) {
        let key = MfaKey::new(module, function, arity);
        let Some(profile) = self.profiles.get_mut(&key) else {
            return;
        };
        if profile.generation.load(Ordering::Acquire) > generation {
            return;
        }
        profile.generation.store(generation, Ordering::Release);
        profile.state.store(state, Ordering::Release);
    }

    fn state_for(&self, module: Atom, function: Atom, arity: u8) -> Option<u8> {
        let key = MfaKey::new(module, function, arity);
        self.profiles
            .get(&key)
            .map(|profile| profile.state.load(Ordering::Acquire))
    }
}

const fn clamp_tuned_threshold(threshold: u32) -> u32 {
    if threshold < MIN_TUNED_THRESHOLD {
        MIN_TUNED_THRESHOLD
    } else if threshold > MAX_TUNED_THRESHOLD {
        MAX_TUNED_THRESHOLD
    } else {
        threshold
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_JIT_THRESHOLD, JitProfiler, RecordResult};
    use crate::atom::Atom;

    fn atom(id: u32) -> Atom {
        Atom::new(id)
    }

    #[test]
    fn call_at_threshold_triggers_compile_once() {
        let profiler = JitProfiler::new(1000);
        for _ in 0..999 {
            assert_eq!(
                profiler.record_call(atom(1), atom(2), 0, 1),
                RecordResult::Continue
            );
        }

        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 1),
            RecordResult::CompileNow
        );
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 1),
            RecordResult::Continue
        );
    }

    #[test]
    fn mark_compiled_prevents_retriggering() {
        let profiler = JitProfiler::new(2);
        // Entries are born at a live call edge; completions only update them.
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 1),
            RecordResult::Continue
        );
        profiler.mark_compiled(atom(1), atom(2), 0, 1);

        // Within the marked generation, COMPILED stays terminal.
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 1),
            RecordResult::Continue
        );
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 1),
            RecordResult::Continue
        );

        // A newer generation is a fresh function: it re-heats and recompiles.
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 2),
            RecordResult::Continue
        );
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 2),
            RecordResult::CompileNow
        );
    }

    #[test]
    fn first_call_to_new_mfa_continues_and_sets_counter() {
        let profiler = JitProfiler::new(2);
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 1, 1),
            RecordResult::Continue
        );
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 1, 1),
            RecordResult::CompileNow
        );
    }

    #[test]
    fn older_generation_call_neither_resets_nor_counts() {
        let profiler = JitProfiler::new(2);
        // One call heats the profile at generation 2.
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 2),
            RecordResult::Continue
        );
        // An older-generation call must not count: were it counted, the counter
        // would reach the threshold here.
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 1),
            RecordResult::Continue
        );
        // The next generation-2 call is the threshold-crosser, proving the
        // older-generation call was not counted.
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 2),
            RecordResult::CompileNow
        );
    }

    #[test]
    fn newer_generation_reheats_compiled_function() {
        let profiler = JitProfiler::new(2);
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 1),
            RecordResult::Continue
        );
        profiler.mark_compiled(atom(1), atom(2), 0, 1);
        assert!(profiler.is_compiled(atom(1), atom(2), 0));

        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 2),
            RecordResult::Continue
        );
        assert!(!profiler.is_compiled(atom(1), atom(2), 0));
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 2),
            RecordResult::CompileNow
        );
    }

    #[test]
    fn newer_generation_retries_unsupported_function() {
        let profiler = JitProfiler::new(2);
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 1),
            RecordResult::Continue
        );
        profiler.mark_unsupported(atom(1), atom(2), 0, 1);
        assert!(profiler.is_unsupported(atom(1), atom(2), 0));

        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 2),
            RecordResult::Continue
        );
        assert!(!profiler.is_unsupported(atom(1), atom(2), 0));
    }

    #[test]
    fn completion_mark_no_ops_when_profile_generation_is_newer() {
        let profiler = JitProfiler::new(1);
        // Generation 1 heats to PENDING.
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 1),
            RecordResult::CompileNow
        );
        // A generation-2 call re-heats before the generation-1 job completes.
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 2),
            RecordResult::CompileNow
        );
        // The stale generation-1 completion must not stamp COMPILED.
        profiler.mark_compiled(atom(1), atom(2), 0, 1);
        assert!(!profiler.is_compiled(atom(1), atom(2), 0));
    }

    #[test]
    fn generation_stamping_reuses_one_slot_per_mfa_across_reloads() {
        let profiler = JitProfiler::new(10);
        for generation in 1..=5 {
            assert_eq!(
                profiler.record_call(atom(1), atom(2), 0, generation),
                RecordResult::Continue
            );
        }

        assert_eq!(
            profiler.profile_entry_count(),
            1,
            "a reload-without-delete cycle must reuse the MFA's slot, not accumulate per-generation entries"
        );
    }

    #[test]
    fn publish_compiled_refuses_after_remove_module_and_stale_generations() {
        let profiler = JitProfiler::new(1);
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 2),
            RecordResult::CompileNow
        );
        profiler.remove_module(atom(1));
        let mut published = false;
        assert!(!profiler.publish_compiled(atom(1), atom(2), 0, 2, || published = true));
        assert!(
            !published,
            "a completion racing a delete must publish nothing"
        );
        assert_eq!(profiler.profile_entry_count(), 0);

        // Stale generation: the profile re-heated at generation 3 before the
        // generation-2 job completed.
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 3),
            RecordResult::CompileNow
        );
        let mut stale_published = false;
        assert!(!profiler.publish_compiled(atom(1), atom(2), 0, 2, || stale_published = true));
        assert!(!stale_published, "a stale-generation completion must publish nothing");
        assert!(!profiler.is_compiled(atom(1), atom(2), 0));

        // The live case publishes and marks COMPILED as one step.
        let mut live_published = false;
        assert!(profiler.publish_compiled(atom(1), atom(2), 0, 3, || live_published = true));
        assert!(live_published);
        assert!(profiler.is_compiled(atom(1), atom(2), 0));
    }

    #[test]
    fn remove_module_drops_only_that_modules_entries() {
        let profiler = JitProfiler::new(2);
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 1),
            RecordResult::Continue
        );
        assert_eq!(
            profiler.record_call(atom(9), atom(2), 0, 1),
            RecordResult::Continue
        );
        profiler.mark_compiled(atom(1), atom(2), 0, 1);
        profiler.mark_compiled(atom(9), atom(2), 0, 1);
        assert!(profiler.is_compiled(atom(1), atom(2), 0));
        assert!(profiler.is_compiled(atom(9), atom(2), 0));

        profiler.remove_module(atom(1));

        assert!(!profiler.is_compiled(atom(1), atom(2), 0));
        assert!(profiler.is_compiled(atom(9), atom(2), 0));
    }

    #[test]
    fn completion_marks_never_resurrect_a_deleted_profile() {
        let profiler = JitProfiler::new(1);
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 2),
            RecordResult::CompileNow
        );
        profiler.remove_module(atom(1));
        assert_eq!(profiler.profile_entry_count(), 0);

        // A queued job completing after the delete must not recreate the
        // profile at its stale generation: registry generations restart at 1
        // after a delete, so a resurrected stamp-2 profile would treat every
        // call from the replacement module as "older" and wedge it out of the
        // JIT permanently.
        profiler.mark_compiled(atom(1), atom(2), 0, 2);
        profiler.mark_unsupported(atom(1), atom(2), 0, 2);
        profiler.reset_counter(atom(1), atom(2), 0, 2);
        assert_eq!(
            profiler.profile_entry_count(),
            0,
            "completion paths must never resurrect a deleted profile"
        );
        assert!(!profiler.is_compiled(atom(1), atom(2), 0));
        assert!(!profiler.is_unsupported(atom(1), atom(2), 0));

        // The replacement module heats from scratch at its restarted
        // generation, unimpeded by the stale completion.
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0, 1),
            RecordResult::CompileNow
        );
    }

    #[test]
    fn default_jit_threshold_is_b130_value() {
        assert_eq!(DEFAULT_JIT_THRESHOLD, 1000);
        assert_eq!(
            JitProfiler::new(DEFAULT_JIT_THRESHOLD).current_threshold(),
            1000
        );
    }

    #[test]
    fn tune_threshold_fast_compile_high_speedup_decreases_threshold() {
        let profiler = JitProfiler::new(1000);

        profiler.tune_threshold(5_000, 2.5);

        assert!(profiler.current_threshold() < 1000);
    }

    #[test]
    fn tune_threshold_slow_compile_low_speedup_increases_threshold() {
        let profiler = JitProfiler::new(1000);

        profiler.tune_threshold(150_000, 1.2);

        assert!(profiler.current_threshold() > 1000);
    }

    #[test]
    fn tune_threshold_never_goes_below_minimum() {
        let profiler = JitProfiler::new(101);

        profiler.tune_threshold(1_000, 3.0);

        assert_eq!(profiler.current_threshold(), 100);
    }

    #[test]
    fn tune_threshold_never_goes_above_maximum() {
        let profiler = JitProfiler::new(9_999);

        profiler.tune_threshold(200_000, 1.0);

        assert_eq!(profiler.current_threshold(), 10_000);
    }
}
