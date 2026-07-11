//! Isolated-process pins for the service inventory (spec §5 assertion 6) and
//! the signed §3.8 idle-wake bound.
//!
//! These assertions need a QUIET process: `cargo test --lib` runs hundreds of
//! scheduler-spawning tests concurrently, whose identically-named threads
//! (pre-commit-3 name collisions) make an exact process-wide thread diff
//! unattributable there. Each integration-test binary is its own OS process,
//! and this one holds both phases in a single `#[test]` so no sibling test
//! pollutes the probe between the baseline and the diff. The in-lib
//! containment test (`inventory_tests`) remains the fast every-run smoke;
//! this binary is the exact form.

#![cfg(feature = "threads")]

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use beamr::module::ModuleRegistry;
use beamr::scheduler::thread_probe::{process_thread_names, thread_name_multiset};
use beamr::scheduler::{
    IDLE_PARK_TIMEOUT, IDLE_WAKES_PER_SEC_PER_WORKER, Scheduler, SchedulerConfig,
};

fn new_scheduler(threads: Option<usize>) -> Scheduler {
    let config = SchedulerConfig {
        thread_count: threads,
        ..SchedulerConfig::default()
    };
    Scheduler::new(config, Arc::new(ModuleRegistry::new()))
        .unwrap_or_else(|error| panic!("scheduler starts: {error}"))
}

/// Every thread the scheduler attributes to itself: normal workers (outside
/// the service model, spec §2.3) plus every inventory entry's names.
fn claimed_names(scheduler: &Scheduler) -> Vec<String> {
    let mut claimed: Vec<String> = scheduler.worker_names().to_vec();
    for entry in scheduler.service_inventory() {
        claimed.extend(entry.thread_names);
    }
    claimed
}

#[test]
fn inventory_matches_os_probe_exactly_and_idle_bound_holds() {
    // ---- Phase A: assertion 6, exact form (spec §5 / lens Q3). ----
    //
    // T1-grade methodology (comment-as-contract):
    //  - sampling source: `process_thread_names` (mach thread ports +
    //    `pthread_getname_np` on macOS; `/proc/self/task/*/comm` on Linux).
    //  - host state: a fresh integration-test process; the only threads
    //    created between the two probes are the scheduler's own.
    //  - assertion: baseline delta EQUALS the claimed multiset, both
    //    directions — an un-inventoried thread fails exactly like an
    //    over-claimed one.
    //
    // Linux note: `comm` truncates to 15 bytes, so exact-name equality is
    // asserted on macOS only; Linux gets the count-level containment check.
    let baseline = thread_name_multiset(&process_thread_names());

    let scheduler = new_scheduler(None);
    let claimed = thread_name_multiset(&claimed_names(&scheduler));

    #[cfg(target_os = "macos")]
    {
        // tokio workers apply their names shortly after spawn; settle until
        // the delta stops changing or the window elapses, then assert.
        let mut delta = std::collections::BTreeMap::new();
        for _ in 0..200 {
            let live = thread_name_multiset(&process_thread_names());
            delta = live
                .iter()
                .filter_map(|(name, live_count)| {
                    let base = baseline.get(name).copied().unwrap_or(0);
                    (*live_count > base).then(|| (name.clone(), live_count - base))
                })
                .collect();
            if delta == claimed {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            delta, claimed,
            "the scheduler's OS-thread footprint must equal exactly what it \
             claims: every claimed thread live, no unclaimed thread spawned"
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        let live_total = process_thread_names().len();
        let baseline_total: usize = baseline.values().sum();
        let claimed_total: usize = claimed.values().sum();
        assert!(
            live_total.saturating_sub(baseline_total) >= claimed_total,
            "at least the claimed thread count must be live"
        );
    }

    // ---- Phase B: the signed §3.8 idle-wake bound, process-wide shape. ----
    //
    // The signed formula (Q-F ruling): wakes/sec/worker × total workers
    // across ALL schedulers in the process, worker counts sourced from the
    // actually-spawned thread records (`worker_names`), never a config claim.
    // Two co-resident schedulers pin the aggregation the formula demands.
    //
    // T1-grade methodology (comment-as-contract):
    //  - measurement duration: 500 ms wall clock, divided by ACTUAL elapsed.
    //  - sampling source: each scheduler's `idle_park_count()` (one increment
    //    per `park_thread` entry).
    //  - host state: this quiet integration-test process, all workers idle.
    //  - upper bound: 2× the formula — a ceiling, never an exact match.
    //  - floor linkage: NOT a wall-clock lower bound (any lower bound is
    //    load-sensitive — an oversubscribed host can starve workers of
    //    runnable time). The running code's actual wait duration is instead
    //    asserted directly via `observed_park_timeout_millis()`, which the
    //    park primitive writes on every entry — deterministic, and it
    //    catches a wait duration that decoupled from the signed constant.
    let second = new_scheduler(Some(4));
    let schedulers = [&scheduler, &second];
    let total_workers: usize = schedulers.iter().map(|s| s.worker_names().len()).sum();

    thread::sleep(Duration::from_millis(50));
    let starts: Vec<usize> = schedulers.iter().map(|s| s.idle_park_count()).collect();
    let started_at = Instant::now();
    thread::sleep(Duration::from_millis(500));
    let parks: usize = schedulers
        .iter()
        .zip(&starts)
        .map(|(s, start)| s.idle_park_count().saturating_sub(*start))
        .sum();
    let elapsed = started_at.elapsed().as_secs_f64();

    let wakes_per_sec = parks as f64 / elapsed;
    let formula = IDLE_WAKES_PER_SEC_PER_WORKER as f64 * total_workers as f64;
    assert!(
        wakes_per_sec <= 2.0 * formula,
        "aggregate idle wake rate {wakes_per_sec:.1}/s exceeds the signed \
         ceiling {:.1}/s for {total_workers} workers at a {}ms floor",
        2.0 * formula,
        IDLE_PARK_TIMEOUT.as_millis(),
    );
    for scheduler in schedulers {
        assert_eq!(
            scheduler.observed_park_timeout_millis(),
            Some(IDLE_PARK_TIMEOUT.as_millis() as u64),
            "the park primitive's actual wait duration must equal the \
             signed IDLE_PARK_TIMEOUT constant"
        );
    }

    second.shutdown();
    scheduler.shutdown();
}
