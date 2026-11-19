//! Run-queue fairness for busy-polling native processes, with and without
//! spawn/exit churn.

use super::*;

// --- Native busy-poll run-queue fairness (live message-loss defect) ---
//
// A native handler that drains its mailbox and returns
// `NativeOutcome::Continue` every slice (the liminal ConnectionProcess shape:
// drain `ctx.recv()`, poll a non-blocking socket, requeue on WouldBlock) is
// permanently runnable. Under a LIFO owner pop, the Requeue arm pushed the
// just-run pid on top of its own queue and the very next `queue.pop()` took
// it right back, so every OTHER pid on that thread's queue never got a
// slice. A message enqueued to such a starved process was delivered into its
// mailbox (`enqueue_atom_message` returned true) but never observed:
// `wake_process` is a no-op for a runnable pid (it is not in the wait set),
// and the steal path could not rescue it either (mid-slice the owner's queue
// held a single pid, which `steal_half_from` then refused to steal). These tests
// pin the FIFO owner pop that makes co-resident runnable processes
// round-robin.

type BusyPoller = (u64, Arc<Mutex<Vec<Term>>>, Arc<AtomicUsize>);

fn spawn_busy_pollers(scheduler: &Scheduler, count: usize) -> Vec<BusyPoller> {
    (0..count)
        .map(|_| {
            let received = Arc::new(Mutex::new(Vec::new()));
            let slices = Arc::new(AtomicUsize::new(0));
            let factory_received = Arc::clone(&received);
            let factory_slices = Arc::clone(&slices);
            let pid = scheduler
                .spawn_native(Box::new(move || {
                    Box::new(BusyPollHandler {
                        received: Arc::clone(&factory_received),
                        slices: Arc::clone(&factory_slices),
                    })
                }))
                .unwrap_or_else(|error| panic!("spawn native busy poller: {error:?}"));
            (pid, received, slices)
        })
        .collect()
}

fn all_observed_atom(pollers: &[BusyPoller], atom: Atom) -> bool {
    pollers
        .iter()
        .all(|(_, received, _)| lock_or_recover(received).contains(&Term::atom(atom)))
}

#[test]
fn message_to_a_colocated_busy_poll_native_process_is_observed() {
    // One scheduler thread owning TWO permanently-runnable native processes:
    // the exact starvation shape. The message must reach both handlers.
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = single_thread_scheduler(&registry);
    let pollers = spawn_busy_pollers(&scheduler, 2);

    // Steady state: the thread is running busy-poll slices.
    wait_until(10_000, || {
        pollers
            .iter()
            .any(|(_, _, slices)| slices.load(Ordering::Acquire) > 100)
    });

    for (pid, _, _) in &pollers {
        assert!(
            scheduler.enqueue_atom_message(*pid, Atom::OK),
            "enqueue to live busy poller must succeed"
        );
    }
    wait_until(2_000, || all_observed_atom(&pollers, Atom::OK));
    scheduler.shutdown();
}

#[test]
fn messages_to_saturated_busy_poll_native_processes_are_all_observed() {
    // More permanently-runnable native processes than scheduler threads, so
    // every thread owns at least two and no thread ever idles (the saturated
    // production shape: no thread parks, so nobody even attempts a steal).
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(4),
            dirty_cpu_threads: Some(1),
            dirty_io_threads: Some(1),
            dirty_queue_depth: Some(8),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let pollers = spawn_busy_pollers(&scheduler, 12);

    wait_until(10_000, || {
        pollers
            .iter()
            .any(|(_, _, slices)| slices.load(Ordering::Acquire) > 100)
    });

    for (pid, _, _) in &pollers {
        assert!(
            scheduler.enqueue_atom_message(*pid, Atom::OK),
            "enqueue to live busy poller must succeed"
        );
    }
    wait_until(2_000, || all_observed_atom(&pollers, Atom::OK));
    scheduler.shutdown();
}

// ---------------------------------------------------------------------------
// Starvation under spawn/exit churn (production shape): a wave of busy-poll
// natives exits while a new wave spawns through the SUPERVISED native spawn
// path (SchedulerSpawnFacility::spawn_native → wait_set.woken tagged for
// scheduler thread 0), the exact shape of liminal-server's connection
// scheduler under a worker restart. The 2026-07 production incident showed
// exactly thread_count natives beating while every other live native received
// ZERO slices until a beater exited — the LIFO owner-pop signature: the
// just-requeued spinner is re-popped immediately, one spinner per thread
// monopolizes its stack top, everything below is never popped, wakes no-op
// (runnable pids are not in the wait set), and no thread ever steals because
// no thread's own queue ever empties. These tests fail within one churn
// cycle if the owner pop ever regresses to LIFO (verified by flipping
// `Worker::new_fifo` to `new_lifo`), and additionally pin the supervised
// spawn path + concurrent exit/spawn/message-delivery interleavings that the
// co-resident tests above do not exercise.
//
// NOTE: the production starvation observed on 2026-07-06/07 with these
// symptoms was NOT a bug on this main — it was a duplicate-dependency build:
// the aion server's [patch] pointed beamr at the fixed local checkout for
// aion's own `beamr ^0.12` dep, while liminal-server's `beamr = "0.11.0"`
// (workspace) requirement silently resolved to crates.io beamr 0.11.0, whose
// RunQueue is LIFO (as is published 0.12.0 — the FIFO fix has not been
// released). The connection scheduler is constructed inside liminal-server,
// so production ran the unfixed copy.
// ---------------------------------------------------------------------------

struct ChurnBusyPollHandler {
    received: Arc<Mutex<Vec<Term>>>,
    slices: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl crate::native::native_process::NativeHandler for ChurnBusyPollHandler {
    fn handle(
        &mut self,
        ctx: &mut crate::native::native_process::NativeContext<'_>,
    ) -> crate::native::native_process::NativeOutcome {
        self.slices.fetch_add(1, Ordering::AcqRel);
        while let Some(message) = ctx.recv() {
            lock_or_recover(&self.received).push(message);
        }
        if self.stop.load(Ordering::Acquire) {
            crate::native::native_process::NativeOutcome::Stop(ExitReason::Normal)
        } else {
            crate::native::native_process::NativeOutcome::Continue
        }
    }
}

struct ChurnPoller {
    pid: u64,
    received: Arc<Mutex<Vec<Term>>>,
    slices: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

fn spawn_churn_poller(scheduler: &Scheduler) -> ChurnPoller {
    let received = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let factory_received = Arc::clone(&received);
    let factory_slices = Arc::clone(&slices);
    let factory_stop = Arc::clone(&stop);
    let pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(ChurnBusyPollHandler {
                received: Arc::clone(&factory_received),
                slices: Arc::clone(&factory_slices),
                stop: Arc::clone(&factory_stop),
            })
        }))
        .unwrap_or_else(|error| panic!("spawn native churn poller: {error:?}"));
    ChurnPoller {
        pid,
        received,
        slices,
        stop,
    }
}

#[test]
fn busy_poll_natives_all_progress_under_spawn_exit_churn() {
    // 4 scheduler threads, waves of 8 spinners. Each cycle: a new wave spawns
    // interleaved with the old wave being told to exit (worker-restart shape).
    // After each cycle EVERY live spinner must keep accruing slices, and an
    // atom enqueued to each must be observed by its handler.
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(4),
            dirty_cpu_threads: Some(1),
            dirty_io_threads: Some(1),
            dirty_queue_depth: Some(8),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let mut wave: Vec<ChurnPoller> = (0..8).map(|_| spawn_churn_poller(&scheduler)).collect();
    wait_until(10_000, || {
        wave.iter()
            .all(|poller| poller.slices.load(Ordering::Acquire) > 100)
    });

    for cycle in 0..5 {
        // Concurrent churn: spawn one new spinner, retire one old spinner.
        let next: Vec<ChurnPoller> = (0..8)
            .map(|i| {
                let fresh = spawn_churn_poller(&scheduler);
                wave[i].stop.store(true, Ordering::Release);
                fresh
            })
            .collect();
        wait_until(10_000, || {
            wave.iter()
                .all(|poller| scheduler.peek_exit_reason(poller.pid).is_some())
        });
        wave = next;

        // Every live spinner must accrue slices over a generous window.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let before: Vec<usize> = wave
            .iter()
            .map(|poller| poller.slices.load(Ordering::Acquire))
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let after: Vec<usize> = wave
            .iter()
            .map(|poller| poller.slices.load(Ordering::Acquire))
            .collect();
        let starved: Vec<(usize, u64, usize)> = wave
            .iter()
            .enumerate()
            .filter(|(i, _)| after[*i] == before[*i])
            .map(|(i, poller)| (i, poller.pid, after[i]))
            .collect();
        assert!(
            starved.is_empty(),
            "cycle {cycle}: starved spinners (index, pid, total slices): {starved:?}; \
             all counts before={before:?} after={after:?}"
        );

        // And a message to each must be observed.
        for poller in &wave {
            assert!(
                scheduler.enqueue_atom_message(poller.pid, Atom::OK),
                "enqueue to live busy poller must succeed"
            );
        }
        wait_until(5_000, || {
            wave.iter()
                .all(|poller| lock_or_recover(&poller.received).contains(&Term::atom(Atom::OK)))
        });
    }
    scheduler.shutdown();
}

#[test]
fn busy_poll_natives_all_progress_under_heavy_concurrent_churn() {
    // Harder production mirror: an embedder thread keeps spawning replacement
    // spinners while old ones exit mid-slice, with concurrent message load.
    // Repeated many cycles; every live spinner must keep accruing slices.
    let scheduler = Arc::new(
        Scheduler::new(
            SchedulerConfig {
                thread_count: Some(4),
                dirty_cpu_threads: Some(1),
                dirty_io_threads: Some(1),
                dirty_queue_depth: Some(8),
                ..SchedulerConfig::default()
            },
            Arc::new(ModuleRegistry::new()),
            NativeBifs::none(),
        )
        .unwrap_or_else(|error| panic!("scheduler starts: {error}")),
    );

    let mut wave: Vec<ChurnPoller> = (0..9).map(|_| spawn_churn_poller(&scheduler)).collect();
    wait_until(10_000, || {
        wave.iter()
            .all(|poller| poller.slices.load(Ordering::Acquire) > 100)
    });

    for cycle in 0..20 {
        // Message blaster: keeps delivering atoms to the current wave from a
        // second embedder thread while churn happens.
        let blast_pids: Vec<u64> = wave.iter().map(|poller| poller.pid).collect();
        let blast_scheduler = Arc::clone(&scheduler);
        let blast_stop = Arc::new(AtomicBool::new(false));
        let blast_stop_thread = Arc::clone(&blast_stop);
        let blaster = std::thread::spawn(move || {
            while !blast_stop_thread.load(Ordering::Acquire) {
                for pid in &blast_pids {
                    let _ = blast_scheduler.enqueue_atom_message(*pid, Atom::OK);
                }
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        });

        // Churn: replace the whole wave, one-in-one-out with jitter.
        let next: Vec<ChurnPoller> = (0..9)
            .map(|i| {
                let fresh = spawn_churn_poller(&scheduler);
                wave[i].stop.store(true, Ordering::Release);
                if i % 3 == 0 {
                    std::thread::sleep(std::time::Duration::from_micros(50 * (i as u64 + 1)));
                }
                fresh
            })
            .collect();
        wait_until(10_000, || {
            wave.iter()
                .all(|poller| scheduler.peek_exit_reason(poller.pid).is_some())
        });
        blast_stop.store(true, Ordering::Release);
        blaster
            .join()
            .unwrap_or_else(|payload| std::panic::resume_unwind(payload));
        wave = next;

        std::thread::sleep(std::time::Duration::from_millis(100));
        let before: Vec<usize> = wave
            .iter()
            .map(|poller| poller.slices.load(Ordering::Acquire))
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let after: Vec<usize> = wave
            .iter()
            .map(|poller| poller.slices.load(Ordering::Acquire))
            .collect();
        let starved: Vec<(usize, u64, usize)> = wave
            .iter()
            .enumerate()
            .filter(|(i, _)| after[*i] == before[*i])
            .map(|(i, poller)| (i, poller.pid, after[i]))
            .collect();
        assert!(
            starved.is_empty(),
            "cycle {cycle}: starved spinners (index, pid, total slices): {starved:?}; \
             all counts before={before:?} after={after:?}"
        );
    }
    scheduler.shutdown();
}
