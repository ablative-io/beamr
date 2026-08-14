//! #87 MEASUREMENT INSTRUMENT — Continue self-requeue re-entry latency.
//!
//! Re-derives, at beamr's own bytes, the relayed figure from liminal-side
//! observation: 8.5–10.6ms re-entry under contention for a process that
//! returns `NativeOutcome::Continue` (the `SliceOutcome::Requeue` path),
//! vs 46–82µs for a real wake. A number measured in another repo's harness
//! is a claim until it is re-measured in the repo it is about to govern.
//!
//! These are `#[ignore]`d measurement runs, not pins: they print latency
//! distributions and assert only structural sanity (samples collected,
//! placement verified at the recorded worker names). Run by hand:
//!
//!   cargo test --test continue_reentry_latency -- --ignored --nocapture
//!
//! Mechanism candidates the topology grid separates (ground measured in
//! gate-logs/106/RUN-RECORD-106.md and scheduler/steal.rs):
//!
//!  - The Requeue push (execution/core.rs:83) fires NO notify_all, so a
//!    parked sibling learns of stealable work only on its ≤5ms timed wake
//!    (`IDLE_PARK_TIMEOUT`).
//!  - A queue holding a SINGLE waiting process is never stolen from
//!    (steal.rs: `steal_half_from` on a 1-entry queue takes nothing), so a
//!    victim waiting alone behind a running spinner is unstealable — that
//!    wait is pure queue time and NO notify design can shorten it.
//!
//! Topology grid, each an isolated scheduler with placement measured (the
//! victim records the `beamr-sched-N` thread name every slice — placement
//! is derived from observation, never assumed from spawn order):
//!
//!  1. `alone`      — thread_count=1, victim only: loop-overhead floor.
//!  2. `pair`       — 4 workers, victim + 1 co-resident spinner, siblings
//!                    idle: single-waiter queue, unstealable ⇒ expect gap
//!                    ≈ spinner slice, insensitive to notify.
//!  3. `trio`       — 4 workers, victim + 2 co-resident spinners, siblings
//!                    idle: ≥2 waiters ⇒ stealable ⇒ rescue only on the
//!                    timed wake. The notify-sensitive topology.
//!  4. `saturated`  — 4 workers, 9 spinners + victim: every worker busy,
//!                    nobody parks ⇒ expect residency×slice queue time,
//!                    insensitive to notify.

#![cfg(feature = "threads")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use beamr::module::ModuleRegistry;
use beamr::native::native_process::{NativeContext, NativeHandler, NativeOutcome};
use beamr::process::ExitReason;
use beamr::scheduler::{NativeBifs, Scheduler, SchedulerConfig};

/// Per-slice cost of a contending spinner. Default 2ms — well below the 5ms
/// park timeout so queue-time and park-time signatures land in different
/// buckets. `BEAMR_87_SPIN_MS` overrides it for the slice-cost scaling
/// control: if slow gaps track multiples of the spin cost, the mechanism is
/// queue arithmetic; if they stay pinned near 5/10ms, it is the park timeout.
fn spin_slice() -> Duration {
    static SPIN: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *SPIN.get_or_init(|| {
        let millis = std::env::var("BEAMR_87_SPIN_MS")
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(2);
        Duration::from_millis(millis)
    })
}
/// Victim slice-entry samples per topology (gaps = samples − 1).
const SAMPLES: usize = 400;
/// Hard ceiling on one topology's run, in case a topology starves the victim
/// far beyond the hypothesis range.
const RUN_CEILING: Duration = Duration::from_secs(60);

/// Parse the running worker's index from its `beamr-sched-N` thread name.
/// usize::MAX marks "not a worker thread" and is asserted absent later.
fn current_worker_index() -> usize {
    std::thread::current()
        .name()
        .and_then(|name| name.strip_prefix("beamr-sched-"))
        .and_then(|index| index.parse().ok())
        .unwrap_or(usize::MAX)
}

struct Sample {
    at: Instant,
    worker: usize,
}

/// The probe: timestamps every slice entry, returns `Continue` — the exact
/// self-requeue seam under measurement. Sampling starts only once `armed`
/// (set after the whole topology is spawned and settled) so early gaps do
/// not mix spawn transients into the distribution.
struct Victim {
    armed: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<Sample>>>,
    target: usize,
    done: mpsc::Sender<()>,
    finished: bool,
}

impl NativeHandler for Victim {
    fn handle(&mut self, _context: &mut NativeContext<'_>) -> NativeOutcome {
        if self.finished {
            return NativeOutcome::Wait;
        }
        if !self.armed.load(Ordering::Acquire) {
            return NativeOutcome::Continue;
        }
        let mut samples = self.samples.lock().expect("victim samples lock");
        samples.push(Sample {
            at: Instant::now(),
            worker: current_worker_index(),
        });
        if samples.len() >= self.target {
            self.finished = true;
            let _sent = self.done.send(());
            return NativeOutcome::Wait;
        }
        NativeOutcome::Continue
    }
}

/// A contending process: burns a fixed wall-clock slice, records (id, worker)
/// per slice, requeues. `slices_left: None` runs until scheduler shutdown;
/// `Some(n)` stops after n slices (episodic topologies).
struct Spinner {
    id: usize,
    slices_left: Option<usize>,
    workers_seen: Arc<Mutex<Vec<(usize, usize)>>>,
}

impl NativeHandler for Spinner {
    fn handle(&mut self, _context: &mut NativeContext<'_>) -> NativeOutcome {
        let started = Instant::now();
        while started.elapsed() < spin_slice() {
            std::hint::spin_loop();
        }
        self.workers_seen
            .lock()
            .expect("spinner worker lock")
            .push((self.id, current_worker_index()));
        if let Some(left) = self.slices_left.as_mut() {
            *left = left.saturating_sub(1);
            if *left == 0 {
                return NativeOutcome::Stop(ExitReason::Normal);
            }
        }
        NativeOutcome::Continue
    }
}

/// One-slice process that reports which worker it ran on, then exits. Used
/// to observe the round-robin spawn cursor, and as a placement filler.
struct Scout {
    report: mpsc::Sender<usize>,
}

impl NativeHandler for Scout {
    fn handle(&mut self, _context: &mut NativeContext<'_>) -> NativeOutcome {
        let _sent = self.report.send(current_worker_index());
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

fn scheduler_with_workers(worker_count: usize) -> Scheduler {
    Scheduler::new(
        SchedulerConfig {
            thread_count: Some(worker_count),
            dirty_cpu_threads: Some(1),
            dirty_io_threads: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .expect("scheduler starts")
}

fn spawn_scout(scheduler: &Scheduler) -> usize {
    let (report, seen) = mpsc::channel();
    scheduler
        .spawn_native(Box::new(move || {
            Box::new(Scout {
                report: report.clone(),
            })
        }))
        .expect("spawn scout");
    seen.recv_timeout(Duration::from_secs(10))
        .expect("scout reports its worker")
}

fn spawn_victim(
    scheduler: &Scheduler,
    armed: &Arc<AtomicBool>,
    samples: &Arc<Mutex<Vec<Sample>>>,
    target: usize,
) -> mpsc::Receiver<()> {
    let (done, done_rx) = mpsc::channel();
    let armed = Arc::clone(armed);
    let samples = Arc::clone(samples);
    scheduler
        .spawn_native(Box::new(move || {
            Box::new(Victim {
                armed: Arc::clone(&armed),
                samples: Arc::clone(&samples),
                target,
                done: done.clone(),
                finished: false,
            })
        }))
        .expect("spawn victim");
    done_rx
}

fn spawn_spinner(
    scheduler: &Scheduler,
    workers_seen: &Arc<Mutex<Vec<(usize, usize)>>>,
    id: usize,
    slices: Option<usize>,
) {
    let workers_seen = Arc::clone(workers_seen);
    scheduler
        .spawn_native(Box::new(move || {
            Box::new(Spinner {
                id,
                slices_left: slices,
                workers_seen: Arc::clone(&workers_seen),
            })
        }))
        .expect("spawn spinner");
}

/// Per-spinner placement summary from the recorded (id, worker) stream:
/// first worker, final worker, and how many times the spinner migrated.
fn spinner_placement_summary(workers_seen: &[(usize, usize)]) -> String {
    let mut ids: Vec<usize> = workers_seen.iter().map(|(id, _)| *id).collect();
    ids.sort_unstable();
    ids.dedup();
    ids.iter()
        .map(|id| {
            let path: Vec<usize> = workers_seen
                .iter()
                .filter(|(seen_id, _)| seen_id == id)
                .map(|(_, worker)| *worker)
                .collect();
            let migrations = path.windows(2).filter(|pair| pair[0] != pair[1]).count();
            format!(
                "s{id}:w{}->w{}(x{migrations})",
                path.first().copied().unwrap_or(usize::MAX),
                path.last().copied().unwrap_or(usize::MAX),
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

struct GapStats {
    micros: Vec<u64>,
    migrations: usize,
    same_worker_micros: Vec<u64>,
    migrated_micros: Vec<u64>,
}

fn gap_stats(samples: &[Sample]) -> GapStats {
    let mut stats = GapStats {
        micros: Vec::with_capacity(samples.len().saturating_sub(1)),
        migrations: 0,
        same_worker_micros: Vec::new(),
        migrated_micros: Vec::new(),
    };
    for pair in samples.windows(2) {
        let micros = pair[1].at.duration_since(pair[0].at).as_micros() as u64;
        stats.micros.push(micros);
        if pair[1].worker == pair[0].worker {
            stats.same_worker_micros.push(micros);
        } else {
            stats.migrations += 1;
            stats.migrated_micros.push(micros);
        }
    }
    stats
}

fn percentile(sorted: &[u64], numerator: usize, denominator: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (sorted.len().saturating_sub(1)) * numerator / denominator;
    sorted[rank]
}

fn print_distribution(label: &str, stats: &GapStats) {
    let mut sorted = stats.micros.clone();
    sorted.sort_unstable();
    println!("== {label} ==");
    println!(
        "gaps={} min={}us p50={}us p90={}us p99={}us max={}us",
        sorted.len(),
        sorted.first().copied().unwrap_or(0),
        percentile(&sorted, 50, 100),
        percentile(&sorted, 90, 100),
        percentile(&sorted, 99, 100),
        sorted.last().copied().unwrap_or(0),
    );
    const BUCKETS: [(u64, &str); 8] = [
        (100, "<100us"),
        (1_000, "<1ms"),
        (3_000, "1-3ms"),
        (5_000, "3-5ms"),
        (7_000, "5-7ms"),
        (9_000, "7-9ms"),
        (11_000, "9-11ms"),
        (u64::MAX, ">=11ms"),
    ];
    let mut counts = [0usize; BUCKETS.len()];
    for &gap in &sorted {
        for (slot, (ceiling, _)) in BUCKETS.iter().enumerate() {
            if gap < *ceiling {
                counts[slot] += 1;
                break;
            }
        }
    }
    let histogram: Vec<String> = BUCKETS
        .iter()
        .zip(counts.iter())
        .filter(|(_, count)| **count > 0)
        .map(|((_, name), count)| format!("{name}:{count}"))
        .collect();
    println!("histogram {}", histogram.join(" "));
    let mut same = stats.same_worker_micros.clone();
    let mut moved = stats.migrated_micros.clone();
    same.sort_unstable();
    moved.sort_unstable();
    println!(
        "migrations={} same-worker p50={}us (n={}) migrated p50={}us (n={})",
        stats.migrations,
        percentile(&same, 50, 100),
        same.len(),
        percentile(&moved, 50, 100),
        moved.len(),
    );
    // WHERE the slow gaps sit separates a start-transient (consecutive low
    // indexes until a steal separates the collision) from a recurring cost
    // (scattered through the run).
    let slow_indexes: Vec<String> = stats
        .micros
        .iter()
        .enumerate()
        .filter(|(_, gap)| **gap >= 1_000)
        .take(60)
        .map(|(index, gap)| format!("{index}:{gap}us"))
        .collect();
    println!(
        "slow gaps (>=1ms, first {} of {}): {}",
        slow_indexes.len(),
        stats.micros.iter().filter(|gap| **gap >= 1_000).count(),
        slow_indexes.join(" "),
    );
}

/// Run one topology: spawn victim + `co_resident_spinners` on ONE worker
/// (round-robin cursor walked with scout fillers, then verified from the
/// recorded worker indexes) + `free_spinners` wherever the cursor lands.
fn run_topology(
    label: &str,
    worker_count: usize,
    co_resident_spinners: usize,
    free_spinners: usize,
) -> GapStats {
    let scheduler = scheduler_with_workers(worker_count);
    let armed = Arc::new(AtomicBool::new(false));
    let samples: Arc<Mutex<Vec<Sample>>> = Arc::new(Mutex::new(Vec::with_capacity(SAMPLES)));
    let spinner_workers: Arc<Mutex<Vec<(usize, usize)>>> = Arc::new(Mutex::new(Vec::new()));

    // Observe the spawn cursor, then place the victim on the next slot.
    let scout_worker = spawn_scout(&scheduler);
    assert_ne!(
        scout_worker,
        usize::MAX,
        "scout must run on a worker thread"
    );
    let victim_worker_by_cursor = (scout_worker + 1) % worker_count;
    let done = spawn_victim(&scheduler, &armed, &samples, SAMPLES);

    // Each co-resident spinner needs the cursor walked a full lap back to
    // the victim's slot: worker_count − 1 scout fillers, then the spinner.
    let mut spinner_id = 0;
    for _ in 0..co_resident_spinners {
        for _ in 0..worker_count - 1 {
            let _filler_worker = spawn_scout(&scheduler);
        }
        spawn_spinner(&scheduler, &spinner_workers, spinner_id, None);
        spinner_id += 1;
    }
    for _ in 0..free_spinners {
        spawn_spinner(&scheduler, &spinner_workers, spinner_id, None);
        spinner_id += 1;
    }

    // Settle spawn transients, then open the sampling gate.
    std::thread::sleep(Duration::from_millis(50));
    let parks_at_arm = scheduler.idle_park_count();
    armed.store(true, Ordering::Release);
    done.recv_timeout(RUN_CEILING)
        .expect("victim finishes its sample budget inside the ceiling");
    let parks_at_done = scheduler.idle_park_count();
    scheduler.shutdown();

    let samples = samples.lock().expect("victim samples lock");
    assert_eq!(samples.len(), SAMPLES, "full sample budget collected");
    assert!(
        samples.iter().all(|sample| sample.worker != usize::MAX),
        "every victim slice ran on a named worker thread"
    );
    println!(
        "[{label}] placement: victim cursor-slot {victim_worker_by_cursor}, victim first slice on {}, parks during sampling {}",
        samples
            .first()
            .map(|sample| sample.worker)
            .unwrap_or(usize::MAX),
        parks_at_done.saturating_sub(parks_at_arm),
    );
    if co_resident_spinners + free_spinners > 0 {
        let spinner_workers = spinner_workers.lock().expect("spinner worker lock");
        println!(
            "[{label}] spinners: {}",
            spinner_placement_summary(&spinner_workers)
        );
    }
    let stats = gap_stats(&samples);
    print_distribution(label, &stats);
    stats
}

/// Episodic collision: what a burst-shaped workload pays PER COLLISION, not
/// at steady state. Each episode boots a fresh scheduler, spawns the victim
/// and two bounded co-resident spinners with sampling armed from the first
/// slice, and records the victim's first gaps — the window before stealing
/// separates the collision. Models the relayed liminal shape: bursts
/// re-create the collision, so the transient IS the recurring cost.
fn run_episodes(label: &str, episodes: usize, gaps_per_episode: usize) {
    let mut all_gaps: Vec<u64> = Vec::new();
    let mut per_episode_max: Vec<u64> = Vec::new();
    let mut first_gaps: Vec<u64> = Vec::new();
    for _ in 0..episodes {
        let scheduler = scheduler_with_workers(4);
        let armed = Arc::new(AtomicBool::new(true));
        let samples: Arc<Mutex<Vec<Sample>>> =
            Arc::new(Mutex::new(Vec::with_capacity(gaps_per_episode + 1)));
        let spinner_workers: Arc<Mutex<Vec<(usize, usize)>>> = Arc::new(Mutex::new(Vec::new()));

        let scout_worker = spawn_scout(&scheduler);
        assert_ne!(scout_worker, usize::MAX);
        let done = spawn_victim(&scheduler, &armed, &samples, gaps_per_episode + 1);
        for spinner_id in 0..2 {
            for _ in 0..3 {
                let _filler_worker = spawn_scout(&scheduler);
            }
            spawn_spinner(&scheduler, &spinner_workers, spinner_id, Some(40));
        }
        done.recv_timeout(RUN_CEILING)
            .expect("episode victim finishes inside the ceiling");
        scheduler.shutdown();

        let samples = samples.lock().expect("victim samples lock");
        let stats = gap_stats(&samples);
        if let Some(first) = stats.micros.first() {
            first_gaps.push(*first);
        }
        per_episode_max.push(stats.micros.iter().copied().max().unwrap_or(0));
        all_gaps.extend(stats.micros);
    }
    let stats = GapStats {
        micros: all_gaps,
        migrations: 0,
        same_worker_micros: Vec::new(),
        migrated_micros: Vec::new(),
    };
    print_distribution(label, &stats);
    first_gaps.sort_unstable();
    per_episode_max.sort_unstable();
    println!(
        "first-gap p50={}us p90={}us max={}us | per-episode max p50={}us p90={}us max={}us (episodes={})",
        percentile(&first_gaps, 50, 100),
        percentile(&first_gaps, 90, 100),
        first_gaps.last().copied().unwrap_or(0),
        percentile(&per_episode_max, 50, 100),
        percentile(&per_episode_max, 90, 100),
        per_episode_max.last().copied().unwrap_or(0),
        first_gaps.len(),
    );
}

#[test]
#[ignore = "#87 measurement instrument, run by hand with --ignored --nocapture"]
fn measure_alone_single_worker_floor() {
    let stats = run_topology("alone: 1 worker, victim only", 1, 0, 0);
    assert!(!stats.micros.is_empty());
}

#[test]
#[ignore = "#87 measurement instrument, run by hand with --ignored --nocapture"]
fn measure_pair_co_resident_unstealable() {
    let stats = run_topology("pair: 4 workers, victim + 1 co-resident spinner", 4, 1, 0);
    assert!(!stats.micros.is_empty());
}

#[test]
#[ignore = "#87 measurement instrument, run by hand with --ignored --nocapture"]
fn measure_trio_co_resident_stealable() {
    let stats = run_topology("trio: 4 workers, victim + 2 co-resident spinners", 4, 2, 0);
    assert!(!stats.micros.is_empty());
}

#[test]
#[ignore = "#87 measurement instrument, run by hand with --ignored --nocapture"]
fn measure_saturated_all_workers_busy() {
    let stats = run_topology("saturated: 4 workers, victim + 9 spinners", 4, 1, 8);
    assert!(!stats.micros.is_empty());
}

#[test]
#[ignore = "#87 measurement instrument, run by hand with --ignored --nocapture"]
fn measure_episodic_collision_resolution() {
    run_episodes(
        "episodic: 40 fresh collisions, victim + 2 bounded co-resident spinners",
        40,
        12,
    );
}
