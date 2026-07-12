//! Dirty-CPU JIT compilation job wiring.

use crate::loader::Instruction;
use crate::scheduler::ServiceMode;
use crate::scheduler::dirty::{DirtyPool, DirtySubmitError, DirtyTask};
use std::sync::Arc;

use super::cache::{JitCache, JitCacheKey};
use super::compiler::{JitCompiler, JitError};
use super::profiler::JitProfiler;

/// Owned function identity and instruction slice for a pending JIT compilation.
pub struct CompilationRequest {
    key: JitCacheKey,
    instructions: Vec<Instruction>,
}

impl CompilationRequest {
    /// Creates a request to compile one generation of an MFA.
    #[must_use]
    pub fn new(
        module: crate::atom::Atom,
        function: crate::atom::Atom,
        arity: u8,
        generation: u64,
        instructions: Vec<Instruction>,
    ) -> Self {
        Self {
            key: JitCacheKey::new(module, function, arity, generation),
            instructions,
        }
    }
}

/// Owned request to compile one BEAM function on a dirty CPU worker.
pub struct CompilationJob {
    request: CompilationRequest,
    compiler: Arc<JitCompiler>,
    profiler: Arc<JitProfiler>,
    cache: Arc<JitCache>,
}

impl CompilationJob {
    /// Creates a compilation job for an MFA and its current instruction slice.
    #[must_use]
    pub fn new(
        request: CompilationRequest,
        compiler: Arc<JitCompiler>,
        profiler: Arc<JitProfiler>,
        cache: Arc<JitCache>,
    ) -> Self {
        Self {
            request,
            compiler,
            profiler,
            cache,
        }
    }

    fn run(self) {
        let request = self.request;
        let key = request.key;
        match self
            .compiler
            .compile(&request.instructions, key.module, key.function, key.arity)
        {
            Ok(native_code) => {
                self.cache.insert(key, native_code);
                self.profiler
                    .mark_compiled(key.module, key.function, key.arity, key.generation);
                self.profiler.note_success();
            }
            Err(
                JitError::UnsupportedOpcode { .. }
                | JitError::UnsupportedOperand { .. }
                | JitError::UnknownLabel { .. },
            ) => {
                self.profiler
                    .mark_unsupported(key.module, key.function, key.arity, key.generation);
                self.profiler.note_unsupported();
            }
            Err(JitError::CraneliftError(_) | JitError::EmptyFunction) => {
                self.profiler
                    .reset_counter(key.module, key.function, key.arity, key.generation);
                self.profiler.note_transient_failure();
            }
        }
    }
}

/// Fire-and-forget submission surface the interpreter's call edges use on
/// [`crate::jit::RecordResult::CompileNow`]. The scheduler implements it over
/// its composed dirty-CPU [`ServiceMode`], attaching its own compiler,
/// profiler, and cache to the job; an `Err` is the pool's refusal and never
/// surfaces into BEAM semantics.
pub trait JitSubmissionFacility: Send + Sync {
    /// Submits one compilation request without blocking the caller.
    fn submit(&self, request: CompilationRequest) -> Result<(), DirtySubmitError>;
}

/// Profiler and submission handle threaded to the interpreter's call edges
/// along the jit-cache seam. Present IFF the jit feature is compiled, replay
/// mode is off, and the dirty-CPU service is live — absence is the disable
/// mechanism, so the edges never branch on replay or service state.
pub struct JitProfilingServices {
    /// Hotness profiler consulted on cache-miss fall-throughs.
    pub profiler: Arc<JitProfiler>,
    /// Threshold-tripped compilation submission into the dirty-CPU service.
    pub submitter: Arc<dyn JitSubmissionFacility>,
}

/// Submits JIT compilation to the dirty CPU pool without blocking the caller.
///
/// Source-compatible raw-pool surface: a caller holding a live `&DirtyPool`
/// has an Owned, enabled pool by construction. Embedders composing with
/// [`ServiceMode`] should use [`try_submit_jit_compilation`], which carries
/// the disabled-pool refusal.
pub fn submit_jit_compilation(
    dirty_cpu: &DirtyPool,
    job: CompilationJob,
) -> Result<(), DirtySubmitError> {
    dirty_cpu.submit_task(DirtyTask::new(move || job.run()))
}

/// Mode-aware JIT submission: a disabled dirty CPU pool refuses with
/// [`DirtySubmitError::Disabled`] before enqueueing anything (spec §3.2) —
/// the same refusal the dirty native dispatch path applies.
pub fn try_submit_jit_compilation(
    dirty_cpu: &ServiceMode<DirtyPool>,
    job: CompilationJob,
) -> Result<(), DirtySubmitError> {
    dirty_cpu.submit_task(DirtyTask::new(move || job.run()))
}

#[cfg(test)]
mod tests {
    use super::{
        CompilationJob, CompilationRequest, submit_jit_compilation, try_submit_jit_compilation,
    };
    use crate::atom::Atom;
    use crate::jit::cache::JitCache;
    use crate::jit::compiler::{JitCompiler, JitSettings};
    use crate::jit::profiler::{JitProfiler, RecordResult};
    use crate::loader::Instruction;
    use crate::scheduler::ServiceMode;
    use crate::scheduler::dirty::{DirtyPool, DirtySubmitError};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if predicate() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// Exercises the source-compatible raw-pool surface: a caller holding a
    /// live `&DirtyPool` submits directly, no [`ServiceMode`] in sight.
    #[test]
    fn empty_return_function_marks_compiled() {
        let pool = DirtyPool::with_queue_depth("jit-compile-success", 1, 4);
        let compiler = Arc::new(JitCompiler::new(JitSettings).unwrap());
        let profiler = Arc::new(JitProfiler::new(1));
        let cache = Arc::new(JitCache::new());
        assert_eq!(
            profiler.record_call(Atom::MODULE, Atom::OK, 0, 1),
            RecordResult::CompileNow
        );

        let job = CompilationJob::new(
            CompilationRequest::new(Atom::MODULE, Atom::OK, 0, 1, vec![Instruction::Return]),
            Arc::clone(&compiler),
            Arc::clone(&profiler),
            Arc::clone(&cache),
        );
        assert_eq!(submit_jit_compilation(&pool, job), Ok(()));

        assert!(wait_until(|| profiler.is_compiled(
            Atom::MODULE,
            Atom::OK,
            0
        ) && cache
            .lookup(Atom::MODULE, Atom::OK, 0, 1)
            .is_some()));
        drop(pool);
    }

    #[test]
    fn unsupported_function_marks_unsupported() {
        let pool = ServiceMode::Owned(DirtyPool::with_queue_depth("jit-compile-unsupported", 1, 4));
        let compiler = Arc::new(JitCompiler::new(JitSettings).unwrap());
        let profiler = Arc::new(JitProfiler::new(1));
        let cache = Arc::new(JitCache::new());
        assert_eq!(
            profiler.record_call(Atom::MODULE, Atom::ERROR, 0, 1),
            RecordResult::CompileNow
        );

        let job = CompilationJob::new(
            CompilationRequest::new(
                Atom::MODULE,
                Atom::ERROR,
                0,
                1,
                vec![Instruction::Generic {
                    opcode: 255,
                    name: "unknown",
                    operands: Vec::new(),
                }],
            ),
            Arc::clone(&compiler),
            Arc::clone(&profiler),
            Arc::clone(&cache),
        );
        assert_eq!(try_submit_jit_compilation(&pool, job), Ok(()));

        assert!(wait_until(|| profiler.is_unsupported(
            Atom::MODULE,
            Atom::ERROR,
            0
        )));
        assert!(cache.lookup(Atom::MODULE, Atom::ERROR, 0, 1).is_none());
        for _ in 0..10 {
            assert_eq!(
                profiler.record_call(Atom::MODULE, Atom::ERROR, 0, 1),
                RecordResult::Continue
            );
        }
        drop(pool);
    }

    /// A disabled dirty CPU pool refuses JIT submission with the typed
    /// [`DirtySubmitError::Disabled`] before enqueueing anything (spec §3.2):
    /// the same refusal-before-side-effect discipline as the dirty native path.
    #[test]
    fn disabled_pool_refuses_jit_submission() {
        let pool: ServiceMode<DirtyPool> = ServiceMode::Disabled;
        let compiler = Arc::new(JitCompiler::new(JitSettings).unwrap());
        let profiler = Arc::new(JitProfiler::new(1));
        let cache = Arc::new(JitCache::new());
        let job = CompilationJob::new(
            CompilationRequest::new(Atom::MODULE, Atom::OK, 0, 1, vec![Instruction::Return]),
            compiler,
            Arc::clone(&profiler),
            cache,
        );
        assert_eq!(
            try_submit_jit_compilation(&pool, job),
            Err(DirtySubmitError::Disabled)
        );
        // Nothing was compiled or scheduled.
        assert!(!profiler.is_compiled(Atom::MODULE, Atom::OK, 0));
    }
}
