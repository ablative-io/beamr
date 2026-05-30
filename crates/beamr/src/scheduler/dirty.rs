//! Dirty scheduler thread pool.
//!
//! A separate pool of OS threads for native functions that take
//! a long time (git push, cargo build). Long-running work goes
//! here so normal scheduler threads stay free and fair.
//! Pool size is configurable independently of the normal
//! scheduler thread count (per D10).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::native::{NativeEntry, ProcessContext};
use crate::process::Process;
use crate::term::Term;

/// Continuation applied to a process when a dirty native call completes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DirtyContinuation {
    /// Store a successful call_ext result in X0; errors exit the process.
    X0,
    /// Store a successful BIF result in the encoded destination; errors jump to `fail_ip`.
    Bif {
        /// Destination operand encoded by the interpreter.
        destination: crate::loader::decode::compact::Operand,
        /// Instruction pointer to jump to if the native call fails.
        fail_ip: usize,
    },
}

/// Native call that must run on a dirty scheduler thread.
#[derive(Clone, Debug)]
pub struct DirtyCall {
    entry: NativeEntry,
    args: Vec<Term>,
    continuation: DirtyContinuation,
}

impl DirtyCall {
    /// Build a dirty call descriptor.
    #[must_use]
    pub fn new(entry: NativeEntry, args: Vec<Term>, continuation: DirtyContinuation) -> Self {
        Self {
            entry,
            args,
            continuation,
        }
    }

    /// Execute the native function on the current thread.
    #[must_use]
    pub fn execute(self) -> DirtyCallResult {
        let mut context = ProcessContext::new();
        let result = (self.entry.function)(&self.args, &mut context);
        DirtyCallResult {
            result,
            continuation: self.continuation,
        }
    }
}

/// Completed dirty native call data.
#[derive(Debug, Eq, PartialEq)]
pub struct DirtyCallResult {
    /// Native function result.
    pub result: Result<Term, Term>,
    /// Resume behavior for the suspended process.
    pub continuation: DirtyContinuation,
}

/// A process migrated to the dirty scheduler pool.
pub struct DirtyJob {
    pid: u64,
    process: DirtyScheduledProcess,
    call: DirtyCall,
}

impl DirtyJob {
    /// Create a dirty scheduler job.
    #[must_use]
    pub fn new(process: Process, call: DirtyCall) -> Self {
        Self {
            pid: process.pid(),
            process: DirtyScheduledProcess(process),
            call,
        }
    }
}

/// Dirty scheduler completion returned to normal scheduler threads.
pub struct DirtyCompletion {
    /// Process identifier.
    pub pid: u64,
    /// Process body after dirty call execution.
    process: DirtyScheduledProcess,
    /// Native call result and continuation.
    pub result: DirtyCallResult,
}

impl DirtyCompletion {
    /// Split this completion into its process body and native result.
    #[must_use]
    pub fn into_parts(self) -> (Process, DirtyCallResult) {
        (self.process.0, self.result)
    }
}

struct DirtyScheduledProcess(Process);

// SAFETY: Dirty jobs are owned exclusively by one dirty worker while the normal
// scheduler has removed the process body from its table slot. The process is
// returned through the completion queue before any normal worker can resume it.
unsafe impl Send for DirtyScheduledProcess {}

struct DirtyState {
    shutdown: AtomicBool,
    queue: Mutex<VecDeque<DirtyJob>>,
    completions: Mutex<VecDeque<DirtyCompletion>>,
    wake: Condvar,
}

/// Pool of OS threads dedicated to dirty native calls.
pub struct DirtyPool {
    state: Arc<DirtyState>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    thread_count: usize,
}

impl DirtyPool {
    /// Start a dirty scheduler pool with `thread_count` worker threads.
    pub fn new(thread_count: usize) -> Result<Self, String> {
        let thread_count = thread_count.max(1);
        let state = Arc::new(DirtyState {
            shutdown: AtomicBool::new(false),
            queue: Mutex::new(VecDeque::new()),
            completions: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
        });
        let mut threads = Vec::with_capacity(thread_count);
        for index in 0..thread_count {
            let state_for_thread = Arc::clone(&state);
            let name = format!("beamr-dirty-{index}");
            let handle = std::thread::Builder::new()
                .name(name.clone())
                .spawn(move || dirty_loop(&state_for_thread))
                .map_err(|error| format!("failed to spawn {name}: {error}"))?;
            threads.push(handle);
        }
        Ok(Self {
            state,
            threads: Mutex::new(threads),
            thread_count,
        })
    }

    /// Number of dirty worker threads.
    #[must_use]
    pub const fn thread_count(&self) -> usize {
        self.thread_count
    }

    /// Submit a process/native call pair to dirty worker threads.
    pub fn submit(&self, job: DirtyJob) {
        let mut queue = lock_or_recover(&self.state.queue);
        queue.push_back(job);
        drop(queue);
        self.state.wake.notify_one();
    }

    /// Drain all completed dirty calls currently available.
    #[must_use]
    pub fn drain_completions(&self) -> Vec<DirtyCompletion> {
        let mut completions = lock_or_recover(&self.state.completions);
        completions.drain(..).collect()
    }

    /// Stop workers after queued jobs complete and join their threads.
    pub fn shutdown(&self) {
        self.state.shutdown.store(true, Ordering::Release);
        self.state.wake.notify_all();
        let mut threads = lock_or_recover(&self.threads);
        for handle in threads.drain(..) {
            if let Err(payload) = handle.join() {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

impl Drop for DirtyPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn dirty_loop(state: &DirtyState) {
    loop {
        let job = next_job(state);
        let Some(job) = job else {
            return;
        };
        let result = job.call.execute();
        let completion = DirtyCompletion {
            pid: job.pid,
            process: job.process,
            result,
        };
        lock_or_recover(&state.completions).push_back(completion);
    }
}

fn next_job(state: &DirtyState) -> Option<DirtyJob> {
    let mut queue = lock_or_recover(&state.queue);
    loop {
        if let Some(job) = queue.pop_front() {
            return Some(job);
        }
        if state.shutdown.load(Ordering::Acquire) {
            return None;
        }
        queue = match state.wake.wait(queue) {
            Ok(guard) => guard,
            Err(error) => error.into_inner(),
        };
    }
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::{DirtyCall, DirtyContinuation, DirtyJob, DirtyPool};
    use crate::native::NativeEntry;
    use crate::process::Process;
    use crate::process::heap::DEFAULT_HEAP_SIZE;
    use crate::term::Term;

    fn thread_name_native(
        args: &[Term],
        context: &mut crate::native::ProcessContext,
    ) -> Result<Term, Term> {
        let _ = (args.len(), context.allocate_term(Term::NIL));
        let is_dirty = std::thread::current()
            .name()
            .is_some_and(|name| name.starts_with("beamr-dirty-"));
        if is_dirty {
            Ok(Term::try_small_int(1).unwrap_or(Term::NIL))
        } else {
            Ok(Term::try_small_int(0).unwrap_or(Term::NIL))
        }
    }

    fn echo_native(
        args: &[Term],
        context: &mut crate::native::ProcessContext,
    ) -> Result<Term, Term> {
        Ok(context.allocate_term(args.first().copied().unwrap_or(Term::NIL)))
    }

    #[test]
    fn dirty_call_executes_on_dirty_thread() {
        let pool = DirtyPool::new(1).expect("pool starts");
        let call = DirtyCall::new(
            NativeEntry {
                function: thread_name_native,
                is_dirty: true,
            },
            Vec::new(),
            DirtyContinuation::X0,
        );
        pool.submit(DirtyJob::new(Process::new(1, DEFAULT_HEAP_SIZE), call));

        let completions = wait_for_completions(&pool, 1);
        assert_eq!(
            completions[0].result.result,
            Ok(Term::try_small_int(1).unwrap_or(Term::NIL))
        );
    }

    #[test]
    fn concurrent_dirty_calls_complete() {
        let pool = DirtyPool::new(2).expect("pool starts");
        for pid in 0..4 {
            let call = DirtyCall::new(
                NativeEntry {
                    function: echo_native,
                    is_dirty: true,
                },
                vec![Term::try_small_int(pid).unwrap_or(Term::NIL)],
                DirtyContinuation::X0,
            );
            pool.submit(DirtyJob::new(
                Process::new(pid as u64, DEFAULT_HEAP_SIZE),
                call,
            ));
        }

        let completions = wait_for_completions(&pool, 4);
        assert_eq!(completions.len(), 4);
    }

    #[test]
    fn normal_thread_is_free_after_submit() {
        let pool = DirtyPool::new(1).expect("pool starts");
        let (tx, rx) = mpsc::channel();
        fn blocking_native(
            args: &[Term],
            context: &mut crate::native::ProcessContext,
        ) -> Result<Term, Term> {
            let _ = (args.len(), context.allocate_term(Term::NIL));
            std::thread::sleep(Duration::from_millis(25));
            Ok(Term::NIL)
        }
        let call = DirtyCall::new(
            NativeEntry {
                function: blocking_native,
                is_dirty: true,
            },
            Vec::new(),
            DirtyContinuation::X0,
        );
        pool.submit(DirtyJob::new(Process::new(9, DEFAULT_HEAP_SIZE), call));
        tx.send(()).expect("send after submit");
        assert!(rx.recv_timeout(Duration::from_millis(5)).is_ok());
    }

    fn wait_for_completions(pool: &DirtyPool, expected: usize) -> Vec<super::DirtyCompletion> {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut completions = Vec::new();
        while completions.len() < expected && Instant::now() < deadline {
            completions.extend(pool.drain_completions());
            std::thread::sleep(Duration::from_millis(1));
        }
        completions
    }
}
