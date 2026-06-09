//! Hot-function profiling for adaptive JIT compilation.

use crate::atom::Atom;
use dashmap::DashMap;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

const STATE_INTERPRETING: u8 = 0;
const STATE_PENDING: u8 = 1;
const STATE_COMPILED: u8 = 2;
const STATE_UNSUPPORTED: u8 = 3;

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
}

impl FunctionProfile {
    fn new() -> Self {
        Self {
            counter: AtomicU32::new(0),
            state: AtomicU8::new(STATE_INTERPRETING),
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

/// Per-function hotness profiler for JIT compilation decisions.
pub struct JitProfiler {
    threshold: u32,
    profiles: DashMap<MfaKey, FunctionProfile>,
}

impl JitProfiler {
    /// Creates a profiler with the supplied threshold.
    #[must_use]
    pub fn new(threshold: u32) -> Self {
        Self {
            threshold: threshold.max(1),
            profiles: DashMap::new(),
        }
    }

    /// Returns the normalized compilation threshold.
    #[must_use]
    pub fn threshold(&self) -> u32 {
        self.threshold
    }

    /// Records a call to an MFA without blocking on compilation work.
    pub fn record_call(&self, module: Atom, function: Atom, arity: u8) -> RecordResult {
        let key = MfaKey::new(module, function, arity);
        let profile = self
            .profiles
            .entry(key)
            .or_insert_with(FunctionProfile::new);

        if profile.state.load(Ordering::Acquire) != STATE_INTERPRETING {
            return RecordResult::Continue;
        }

        let new_count = profile
            .counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                Some(count.saturating_add(1))
            })
            .map_or(1, |previous| previous.saturating_add(1));

        if new_count < self.threshold {
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

    /// Marks a pending or interpreted function as compiled.
    pub fn mark_compiled(&self, module: Atom, function: Atom, arity: u8) {
        self.set_state(module, function, arity, STATE_COMPILED);
    }

    /// Marks a function as permanently unsupported by this JIT tier.
    pub fn mark_unsupported(&self, module: Atom, function: Atom, arity: u8) {
        self.set_state(module, function, arity, STATE_UNSUPPORTED);
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

    /// Resets a transient compilation failure so the function can heat up again.
    pub fn reset_counter(&self, module: Atom, function: Atom, arity: u8) {
        let key = MfaKey::new(module, function, arity);
        let profile = self
            .profiles
            .entry(key)
            .or_insert_with(FunctionProfile::new);
        profile.counter.store(0, Ordering::Release);
        profile.state.store(STATE_INTERPRETING, Ordering::Release);
    }

    fn set_state(&self, module: Atom, function: Atom, arity: u8, state: u8) {
        let key = MfaKey::new(module, function, arity);
        let profile = self
            .profiles
            .entry(key)
            .or_insert_with(FunctionProfile::new);
        profile.state.store(state, Ordering::Release);
    }

    fn state_for(&self, module: Atom, function: Atom, arity: u8) -> Option<u8> {
        let key = MfaKey::new(module, function, arity);
        self.profiles
            .get(&key)
            .map(|profile| profile.state.load(Ordering::Acquire))
    }
}

#[cfg(test)]
mod tests {
    use super::{JitProfiler, RecordResult};
    use crate::atom::Atom;

    fn atom(id: u32) -> Atom {
        Atom::new(id)
    }

    #[test]
    fn call_at_threshold_triggers_compile_once() {
        let profiler = JitProfiler::new(1000);
        for _ in 0..999 {
            assert_eq!(
                profiler.record_call(atom(1), atom(2), 0),
                RecordResult::Continue
            );
        }

        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0),
            RecordResult::CompileNow
        );
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0),
            RecordResult::Continue
        );
    }

    #[test]
    fn mark_compiled_prevents_retriggering() {
        let profiler = JitProfiler::new(2);
        profiler.mark_compiled(atom(1), atom(2), 0);

        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0),
            RecordResult::Continue
        );
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 0),
            RecordResult::Continue
        );
    }

    #[test]
    fn first_call_to_new_mfa_continues_and_sets_counter() {
        let profiler = JitProfiler::new(2);
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 1),
            RecordResult::Continue
        );
        assert_eq!(
            profiler.record_call(atom(1), atom(2), 1),
            RecordResult::CompileNow
        );
    }
}
