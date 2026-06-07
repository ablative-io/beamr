//! Dirty scheduler thread pool.
//!
//! A separate pool of OS threads for native functions that take
//! a long time (git push, cargo build). Long-running work goes
//! here so normal scheduler threads stay free and fair.
//! Pool size is configurable independently of the normal
//! scheduler thread count (per D10).

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::atom::Atom;
use crate::native::{NativeFn, ProcessContext};
use crate::scheduler::lock_or_recover;
use crate::term::Term;

const DEFAULT_QUEUE_DEPTH: usize = 1024;
const DEFAULT_IO_THREADS: usize = 10;

/// Which dirty scheduler pool should execute a native function.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DirtySchedulerKind {
    /// CPU-bound dirty native work.
    Cpu,
    /// IO-bound dirty native work.
    Io,
}

/// Native work item executed by a dirty scheduler worker.
pub struct DirtyJob {
    /// Process that submitted the native work.
    pub pid: u64,
    /// Native function to execute.
    pub function: NativeFn,
    /// Arguments passed to the native function.
    pub args: Vec<Term>,
    /// Detached native context used while the dirty job runs.
    pub context: ProcessContext<'static>,
    /// Completion channel for the native result.
    pub result_sender: oneshot::Sender<Result<Term, Term>>,
}

// SAFETY: Dirty jobs are only valid with detached `ProcessContext::new()`-style
// contexts. B-077 does not migrate process heaps to dirty threads; attached
// process contexts must stay on normal scheduler threads until B-078 adds the
// migration boundary.
unsafe impl Send for DirtyJob {}

/// Pool of OS threads dedicated to dirty native work.
pub struct DirtyPool {
    state: Arc<PoolState>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    thread_names: Vec<String>,
}

struct PoolState {
    queue: Mutex<QueueState>,
    not_empty: Condvar,
    not_full: Condvar,
    max_depth: usize,
}

struct QueueState {
    shutdown: bool,
    jobs: VecDeque<DirtyJob>,
}

impl DirtyPool {
    /// Create a dirty pool with the default bounded queue depth.
    #[must_use]
    pub fn new(name: &str, thread_count: usize) -> Self {
        Self::with_queue_depth(name, thread_count, DEFAULT_QUEUE_DEPTH)
    }

    /// Create the default CPU dirty pool using one thread per logical CPU.
    #[must_use]
    pub fn cpu_default() -> Self {
        Self::new("dirty-cpu", num_cpus::get())
    }

    /// Create the default IO dirty pool.
    #[must_use]
    pub fn io_default() -> Self {
        Self::new("dirty-io", DEFAULT_IO_THREADS)
    }

    /// Create a dirty pool with an explicit queue depth.
    #[must_use]
    pub fn with_queue_depth(name: &str, thread_count: usize, max_depth: usize) -> Self {
        let state = Arc::new(PoolState {
            queue: Mutex::new(QueueState {
                shutdown: false,
                jobs: VecDeque::new(),
            }),
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
            max_depth: max_depth.max(1),
        });
        let mut threads = Vec::with_capacity(thread_count);
        let mut thread_names = Vec::with_capacity(thread_count);

        for index in 0..thread_count {
            let thread_name = format!("{name}-{index}");
            let state_for_thread = Arc::clone(&state);
            match std::thread::Builder::new()
                .name(thread_name.clone())
                .spawn(move || worker_loop(&state_for_thread))
            {
                Ok(handle) => {
                    thread_names.push(thread_name);
                    threads.push(handle);
                }
                Err(_error) => {
                    signal_shutdown(&state);
                    join_all(threads);
                    threads = Vec::new();
                    thread_names.clear();
                    break;
                }
            }
        }

        Self {
            state,
            threads: Mutex::new(threads),
            thread_names,
        }
    }

    /// Submit a dirty job, blocking while the bounded queue is full.
    pub fn submit(&self, job: DirtyJob) {
        let mut guard = lock_or_recover(&self.state.queue);
        while !guard.shutdown && guard.jobs.len() >= self.state.max_depth {
            guard = wait_or_recover(&self.state.not_full, guard);
        }
        if guard.shutdown {
            let _ = job.result_sender.send(Err(Term::atom(Atom::EXIT)));
            return;
        }
        guard.jobs.push_back(job);
        self.state.not_empty.notify_one();
    }

    /// Shut the pool down and join all worker threads.
    pub fn shutdown(&self) {
        signal_shutdown(&self.state);
        let threads = {
            let mut guard = lock_or_recover(&self.threads);
            guard.drain(..).collect::<Vec<_>>()
        };
        join_all(threads);
    }

    /// Number of OS threads originally created for this pool.
    #[must_use]
    pub fn thread_count(&self) -> usize {
        self.thread_names.len()
    }

    /// Worker thread names for diagnostics and tests.
    #[must_use]
    pub fn thread_names(&self) -> &[String] {
        &self.thread_names
    }

    /// Configured queue capacity.
    #[must_use]
    pub fn max_depth(&self) -> usize {
        self.state.max_depth
    }
}

impl Drop for DirtyPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn worker_loop(state: &PoolState) {
    loop {
        let job = {
            let mut guard = lock_or_recover(&state.queue);
            while !guard.shutdown && guard.jobs.is_empty() {
                guard = wait_or_recover(&state.not_empty, guard);
            }
            if guard.shutdown && guard.jobs.is_empty() {
                return;
            }
            let job = guard.jobs.pop_front();
            state.not_full.notify_one();
            job
        };

        if let Some(mut job) = job {
            let result = catch_unwind(AssertUnwindSafe(|| {
                (job.function)(&job.args, &mut job.context)
            }))
            .unwrap_or_else(|_panic| Err(Term::atom(Atom::ERROR)));
            let _ = job.result_sender.send(result);
        }
    }
}

fn signal_shutdown(state: &PoolState) {
    {
        let mut guard = lock_or_recover(&state.queue);
        guard.shutdown = true;
    }
    state.not_empty.notify_all();
    state.not_full.notify_all();
}

fn join_all(threads: Vec<JoinHandle<()>>) {
    for handle in threads {
        // Worker loops catch native panics before continuing. If a worker still
        // unwinds, shutdown must remain best-effort and idempotent.
        let _ = handle.join();
    }
}

fn wait_or_recover<'a>(
    condvar: &Condvar,
    guard: std::sync::MutexGuard<'a, QueueState>,
) -> std::sync::MutexGuard<'a, QueueState> {
    condvar
        .wait(guard)
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forty_two(_args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
        Ok(Term::small_int(42))
    }

    #[test]
    fn dirty_pool_creates_requested_threads_and_shuts_down() {
        let pool = DirtyPool::new("dirty-test", 4);

        assert_eq!(pool.thread_count(), 4);
        assert_eq!(
            pool.thread_names(),
            &[
                "dirty-test-0",
                "dirty-test-1",
                "dirty-test-2",
                "dirty-test-3"
            ]
        );

        pool.shutdown();
        pool.shutdown();
    }

    #[test]
    fn dirty_pool_executes_submitted_job() {
        let pool = DirtyPool::with_queue_depth("dirty-job", 1, 1);
        let (sender, receiver) = oneshot::channel();
        pool.submit(DirtyJob {
            pid: 1,
            function: forty_two,
            args: Vec::new(),
            context: ProcessContext::new(),
            result_sender: sender,
        });

        assert_eq!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap_or_else(|error| panic!("dirty job result: {error}")),
            Ok(Term::small_int(42))
        );

        pool.shutdown();
    }
}
