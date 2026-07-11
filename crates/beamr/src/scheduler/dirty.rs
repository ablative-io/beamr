//! Dirty scheduler thread pool.
//!
//! A separate pool of OS threads for native functions that take
//! a long time (git push, cargo build). Long-running work goes
//! here so normal scheduler threads stay free and fair.
//! Pool size is configurable independently of the normal
//! scheduler thread count (per D10).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender};

use crate::ets::OwnedTerm;
use crate::native::{ExceptionClass, NativeContinuation, NativeFn, ProcessContext, SuspendRequest};
use crate::scheduler::lock_or_recover;
use crate::term::Term;

/// Default maximum number of queued dirty jobs per pool.
pub const DEFAULT_DIRTY_QUEUE_DEPTH: usize = 1024;

/// Default number of IO dirty scheduler threads.
pub const DEFAULT_DIRTY_IO_THREADS: usize = 10;

pub use crate::scheduler::DirtySchedulerKind;

/// Minimal oneshot result channel used by dirty jobs.
pub mod oneshot {
    use std::sync::mpsc;

    /// Sends a single value to the matching [`Receiver`].
    pub struct Sender<T>(mpsc::SyncSender<T>);

    /// Receives a single value from the matching [`Sender`].
    pub struct Receiver<T>(mpsc::Receiver<T>);

    /// Error returned when the oneshot receiver has been dropped.
    pub struct SendError<T>(pub T);

    /// Error returned when the oneshot sender has been dropped.
    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    pub struct RecvError;

    /// Creates a single-use channel.
    #[must_use]
    pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
        let (sender, receiver) = mpsc::sync_channel(1);
        (Sender(sender), Receiver(receiver))
    }

    impl<T> Sender<T> {
        /// Sends the result to the receiver.
        pub fn send(self, value: T) -> Result<(), SendError<T>> {
            self.0.send(value).map_err(|error| SendError(error.0))
        }
    }

    impl<T> Receiver<T> {
        /// Blocks until the result arrives or the sender is dropped.
        pub fn recv(self) -> Result<T, RecvError> {
            self.0.recv().map_err(|_| RecvError)
        }
    }
}

/// Result of a native function invocation completed on a dirty scheduler thread.
#[derive(Debug)]
pub struct DirtyResult {
    /// Native function return value or error reason.
    pub result: Result<Term, Term>,
    /// Owns boxed/list allocations reachable from `result` until the process
    /// resumes and copies the dirty native return value onto its own heap.
    pub owned_result: Option<OwnedTerm>,
    /// Exception class requested by the dirty native if it returned `Err`.
    pub exception_class: ExceptionClass,
    /// Stacktrace requested by the dirty native if it returned `Err`.
    pub exception_stacktrace: Term,
    /// Suspend request the dirty native left on its detached context: the
    /// owning thread re-parks the process at the dirty call instruction
    /// under a NEW host-await suspension instead of applying a value.
    /// Ignored when the native returned `Err` (the exception wins, matching
    /// `call_native_entry`).
    pub suspend: Option<SuspendRequest>,
    /// Trampoline request the dirty native left on its detached context:
    /// the owning thread sets up the closure call on resume. Must carry a
    /// continuation (returning straight to the call instruction would
    /// re-submit the dirty call).
    pub trampoline: Option<OwnedDirtyTrampoline>,
}

/// A dirty native's trampoline request with its terms copied into owned
/// storage, so they survive the detached context's teardown until the owning
/// thread copies them onto the resuming process heap.
#[derive(Debug)]
pub struct OwnedDirtyTrampoline {
    /// The closure (fun) term to invoke.
    pub fun: OwnedTerm,
    /// Arguments to pass to the closure.
    pub args: Vec<OwnedTerm>,
    /// Continuation resumed after the closure returns. Must hold no heap
    /// terms (a detached context's terms would dangle); term-carrying
    /// continuations are rejected at the dirty worker.
    pub continuation: NativeContinuation,
}

/// Native function invocation scheduled onto a dirty scheduler thread.
pub struct DirtyJob {
    /// Process id that submitted the dirty job.
    pub pid: u64,
    /// Native function to execute.
    pub function: NativeFn,
    /// Arguments passed to the native function.
    pub args: Vec<Term>,
    /// Native call context for the dirty worker.
    pub context: ProcessContext<'static>,
    /// Channel used to return the native result to the submitter.
    pub result_sender: oneshot::Sender<DirtyResult>,
}

// SAFETY: dirty scheduler jobs are constructed for standalone native calls and
// use `ProcessContext<'static>` so they cannot borrow a scheduler-owned process.
// B-077 does not migrate process bodies to dirty threads; future wiring must keep
// that boundary by submitting only detached contexts.
unsafe impl Send for DirtyJob {}

/// Generic dirty CPU work item for runtime maintenance jobs such as JIT compilation.
pub struct DirtyTask {
    task: Box<dyn FnOnce() + Send + 'static>,
}

impl DirtyTask {
    /// Creates a dirty task from an owned closure.
    pub fn new(task: impl FnOnce() + Send + 'static) -> Self {
        Self {
            task: Box::new(task),
        }
    }
}

enum DirtyMessage {
    RunNative(Box<DirtyJob>),
    RunTask(DirtyTask),
    Shutdown,
}

/// Failure returned when a dirty job cannot be enqueued.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DirtySubmitError {
    /// Submission was attempted after pool shutdown began.
    ShutDown,
    /// The bounded dirty queue is full; the normal scheduler must not block.
    QueueFull,
    /// All dirty workers disconnected from the queue.
    Disconnected,
}

/// A bounded dirty scheduler pool backed by OS threads.
pub struct DirtyPool {
    name: String,
    thread_count: usize,
    queue_depth: usize,
    sender: Sender<DirtyMessage>,
    shutdown: AtomicBool,
    threads: Mutex<Vec<JoinHandle<()>>>,
    worker_names: Vec<String>,
    requested_threads: usize,
}

impl DirtyPool {
    /// Creates a dirty pool with the default bounded queue depth.
    #[must_use]
    pub fn new(name: &str, thread_count: usize) -> Self {
        Self::with_queue_depth(name, thread_count, DEFAULT_DIRTY_QUEUE_DEPTH)
    }

    /// Creates the default CPU dirty pool.
    #[must_use]
    pub fn default_cpu() -> Self {
        Self::new("dirty-cpu", num_cpus::get())
    }

    /// Creates the default IO dirty pool.
    #[must_use]
    pub fn default_io() -> Self {
        Self::new("dirty-io", DEFAULT_DIRTY_IO_THREADS)
    }

    /// Creates a dirty pool with a configurable bounded queue depth.
    #[must_use]
    pub fn with_queue_depth(name: &str, thread_count: usize, queue_depth: usize) -> Self {
        let pool_thread_count = thread_count.max(1);
        let pool_queue_depth = queue_depth.max(1);
        let (sender, receiver) = crossbeam_channel::bounded(pool_queue_depth);
        let mut threads = Vec::with_capacity(pool_thread_count);
        let mut worker_names = Vec::with_capacity(pool_thread_count);

        for index in 0..pool_thread_count {
            let thread_name = format!("{name}-{index}");
            let receiver_for_thread = receiver.clone();
            match std::thread::Builder::new()
                .name(thread_name.clone())
                .spawn(move || worker_loop(receiver_for_thread))
            {
                Ok(handle) => {
                    worker_names.push(thread_name);
                    threads.push(handle);
                }
                Err(_error) => break,
            }
        }

        Self {
            name: name.to_owned(),
            thread_count: worker_names.len(),
            queue_depth: pool_queue_depth,
            sender,
            shutdown: AtomicBool::new(false),
            threads: Mutex::new(threads),
            worker_names,
            requested_threads: pool_thread_count,
        }
    }

    /// Enqueues a dirty job without blocking a normal scheduler thread.
    pub fn submit(&self, job: DirtyJob) -> Result<(), DirtySubmitError> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(DirtySubmitError::ShutDown);
        }
        self.sender
            .try_send(DirtyMessage::RunNative(Box::new(job)))
            .map_err(|error| match error {
                crossbeam_channel::TrySendError::Full(_) => DirtySubmitError::QueueFull,
                crossbeam_channel::TrySendError::Disconnected(_) => DirtySubmitError::Disconnected,
            })
    }

    /// Enqueues a generic dirty task without blocking a normal scheduler thread.
    pub fn submit_task(&self, task: DirtyTask) -> Result<(), DirtySubmitError> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(DirtySubmitError::ShutDown);
        }
        self.sender
            .try_send(DirtyMessage::RunTask(task))
            .map_err(|error| match error {
                crossbeam_channel::TrySendError::Full(_) => DirtySubmitError::QueueFull,
                crossbeam_channel::TrySendError::Disconnected(_) => DirtySubmitError::Disconnected,
            })
    }

    /// Signals all dirty workers to stop and joins them.
    pub fn shutdown(&self) {
        if self.shutdown.swap(true, Ordering::AcqRel) {
            return;
        }

        let mut threads = lock_or_recover(&self.threads);
        for _ in 0..threads.len() {
            let _ = self.sender.send(DirtyMessage::Shutdown);
        }
        for handle in threads.drain(..) {
            if let Err(payload) = handle.join() {
                std::panic::resume_unwind(payload);
            }
        }
    }

    /// Number of worker threads successfully started for this pool.
    #[must_use]
    pub fn thread_count(&self) -> usize {
        self.thread_count
    }

    /// Configured bounded queue depth.
    #[must_use]
    pub fn queue_depth(&self) -> usize {
        self.queue_depth
    }

    /// Pool base name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Names of worker OS threads in this pool.
    #[must_use]
    pub fn worker_names(&self) -> &[String] {
        &self.worker_names
    }

    /// Names of worker OS threads LIVE right now — the `actual` half of the
    /// service inventory's requested-vs-live split (spec §5).
    ///
    /// Liveness comes from synchronized JOIN-HANDLE state, not the shutdown
    /// request flag: the flag is raised before the joins run (an in-flight
    /// dirty call can hold a join open arbitrarily long), so flag-gating
    /// would report zero while workers are still live. Locking the handles
    /// makes a mid-shutdown caller wait for the joins instead, and
    /// `is_finished` drops a worker that panicked early. `shutdown()` drains
    /// the handle vector under this same lock, so callers only ever observe
    /// all-handles-present or fully-drained, never a partial join.
    #[must_use]
    pub fn live_worker_names(&self) -> Vec<String> {
        let threads = lock_or_recover(&self.threads);
        self.worker_names
            .iter()
            .zip(threads.iter())
            .filter(|(_, handle)| !handle.is_finished())
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Worker threads this pool was ASKED to run (after the `.max(1)`
    /// coercion), independent of how many spawned successfully or remain
    /// live — the `configured` half of the same split.
    #[must_use]
    pub fn requested_threads(&self) -> usize {
        self.requested_threads
    }

    /// Whether shutdown has been requested.
    #[must_use]
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }
}

impl Drop for DirtyPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn worker_loop(receiver: Receiver<DirtyMessage>) {
    while let Ok(message) = receiver.recv() {
        match message {
            DirtyMessage::RunNative(mut job) => {
                let _pid = job.pid;
                let result = (job.function)(&job.args, &mut job.context);
                let raw_result = match &result {
                    Ok(value) | Err(value) => *value,
                };
                let owned_result = job.context.take_detached_result(raw_result).or_else(|| {
                    if raw_result.is_list() || raw_result.is_boxed() {
                        crate::ets::copy_term_to_ets(raw_result).ok()
                    } else {
                        None
                    }
                });
                let result = match owned_result.as_ref() {
                    Some(owned) => result.map(|_| owned.root()).map_err(|_| owned.root()),
                    None => result,
                };
                let exception_class = job.context.take_exception_class();
                let exception_stacktrace = job.context.take_exception_stacktrace();
                // Follow-up requests a dirty native is allowed to make:
                // re-suspend (host await) or trampoline a closure call. The
                // exception path wins over both, matching call_native_entry.
                let suspend = job.context.take_suspend().filter(|_| result.is_ok());
                let trampoline = match job.context.take_trampoline().filter(|_| result.is_ok()) {
                    None => None,
                    Some(request) => match own_dirty_trampoline(request) {
                        Ok(owned) => Some(owned),
                        Err(reason) => {
                            // Reject malformed requests loudly: the process
                            // raises instead of silently dropping them.
                            let _ = job.result_sender.send(DirtyResult {
                                result: Err(Term::atom(crate::atom::Atom::BADARG)),
                                owned_result: None,
                                exception_class: ExceptionClass::Error,
                                exception_stacktrace: Term::NIL,
                                suspend: None,
                                trampoline: None,
                            });
                            let _trace = reason;
                            continue;
                        }
                    },
                };
                let _ = job.result_sender.send(DirtyResult {
                    result,
                    owned_result,
                    exception_class,
                    exception_stacktrace,
                    suspend,
                    trampoline,
                });
            }
            DirtyMessage::RunTask(task) => {
                (task.task)();
            }
            DirtyMessage::Shutdown => break,
        }
    }
}

/// Copy a dirty native's trampoline request into owned storage.
///
/// Rejects requests without a continuation (returning to the call
/// instruction would re-submit the dirty call) and continuations that hold
/// heap terms (a detached context's terms dangle once the job is dropped).
fn own_dirty_trampoline(
    request: crate::native::TrampolineRequest,
) -> Result<OwnedDirtyTrampoline, &'static str> {
    let Some(continuation) = request.continuation else {
        return Err("dirty trampoline requires a continuation");
    };
    let mut holds_terms = false;
    continuation.for_each_term(&mut |_| holds_terms = true);
    if holds_terms {
        return Err("dirty trampoline continuation must not hold heap terms");
    }
    let fun = own_term(request.fun).map_err(|_| "dirty trampoline fun copy failed")?;
    let mut args = Vec::with_capacity(request.args.len());
    for arg in request.args {
        args.push(own_term(arg).map_err(|_| "dirty trampoline arg copy failed")?);
    }
    Ok(OwnedDirtyTrampoline {
        fun,
        args,
        continuation,
    })
}

fn own_term(term: Term) -> Result<OwnedTerm, crate::ets::EtsError> {
    if term.is_list() || term.is_boxed() {
        crate::ets::copy_term_to_ets(term)
    } else {
        Ok(OwnedTerm::immediate(term))
    }
}

#[cfg(test)]
mod tests {
    use super::{DirtyJob, DirtyPool, DirtySchedulerKind, oneshot};
    use crate::native::{ExceptionClass, ProcessContext};
    use crate::term::Term;

    fn forty_two(_args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
        Ok(Term::small_int(42))
    }

    #[test]
    fn dirty_pool_starts_named_threads_and_shuts_down_cleanly() {
        let pool = DirtyPool::new("dirty-test", 4);

        assert_eq!(pool.thread_count(), 4);
        assert_eq!(pool.worker_names().len(), 4);
        assert_eq!(
            pool.worker_names(),
            &[
                "dirty-test-0".to_owned(),
                "dirty-test-1".to_owned(),
                "dirty-test-2".to_owned(),
                "dirty-test-3".to_owned(),
            ]
        );

        pool.shutdown();
        assert!(pool.is_shutdown());
        pool.shutdown();
    }

    /// `live_worker_names` reports join-handle liveness, never the shutdown
    /// request flag: with a worker deliberately blocked in a dirty task, a
    /// shutdown in progress must not produce a false zero — a concurrent
    /// probe blocks on the handle lock until the join completes and then
    /// observes the fully-drained state, and before shutdown the blocked
    /// worker still counts as live.
    #[test]
    fn live_worker_names_track_joins_not_the_shutdown_flag() {
        use super::DirtyTask;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let pool = Arc::new(DirtyPool::with_queue_depth("dirty-live", 1, 1));
        let release = Arc::new(AtomicBool::new(false));

        let release_for_task = Arc::clone(&release);
        pool.submit_task(DirtyTask::new(move || {
            while !release_for_task.load(Ordering::Acquire) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }))
        .expect("task submitted");

        // The worker is (or is about to be) blocked in the task; it is LIVE.
        assert_eq!(pool.live_worker_names(), vec!["dirty-live-0".to_owned()]);

        // Start shutdown; it raises the flag immediately but stays in the
        // join (holding the handle lock) until the task releases. Wait until
        // the join loop provably holds the lock, then probe: the probe must
        // BLOCK rather than report a false zero — under flag-gated liveness
        // it would return [] instantly while the worker is still live.
        let pool_for_shutdown = Arc::clone(&pool);
        let shutdown = std::thread::spawn(move || pool_for_shutdown.shutdown());
        loop {
            match pool.threads.try_lock() {
                Err(std::sync::TryLockError::WouldBlock) => break,
                Ok(guard) => drop(guard),
                Err(std::sync::TryLockError::Poisoned(error)) => {
                    panic!("handle lock poisoned: {error}")
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let pool_for_probe = Arc::clone(&pool);
        let probe = std::thread::spawn(move || pool_for_probe.live_worker_names());
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            !probe.is_finished(),
            "the probe must wait for the join, never report a false zero"
        );

        release.store(true, Ordering::Release);
        shutdown.join().expect("shutdown joins");
        assert!(
            probe.join().expect("probe joins").is_empty(),
            "a probe concurrent with shutdown reports the joined state"
        );
        assert!(pool.live_worker_names().is_empty());
    }

    #[test]
    fn dirty_pool_executes_submitted_job_and_returns_result() {
        let pool = DirtyPool::with_queue_depth("dirty-test-job", 1, 1);
        let (result_sender, result_receiver) = oneshot::channel();

        assert_eq!(
            pool.submit(DirtyJob {
                pid: 7,
                function: forty_two,
                args: Vec::new(),
                context: ProcessContext::new(),
                result_sender,
            }),
            Ok(())
        );

        let result = result_receiver.recv().expect("dirty result");
        assert_eq!(result.result, Ok(Term::small_int(42)));
        assert!(result.owned_result.is_none());
        assert_eq!(result.exception_class, ExceptionClass::Error);
        assert_eq!(result.exception_stacktrace, Term::NIL);
        pool.shutdown();
    }

    #[test]
    fn dirty_scheduler_kind_distinguishes_cpu_and_io() {
        assert_eq!(DirtySchedulerKind::Cpu, DirtySchedulerKind::Cpu);
        assert_ne!(DirtySchedulerKind::Cpu, DirtySchedulerKind::Io);
    }
}
