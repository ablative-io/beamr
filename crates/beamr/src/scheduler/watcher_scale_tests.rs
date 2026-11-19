//! A process that watches many others must cost about the same to run as one
//! that watches a few.
//!
//! Liminal's channel supervisor monitors every channel actor. On 19 Nov 2026
//! one exchange held about 17,000, and every run of the watcher copied each
//! monitor into its slot and merged them back with a scan per monitor, about
//! 3×10⁸ comparisons a run. One scheduler thread stayed at 100% and every
//! other process on it starved.

use std::time::{Duration, Instant};

use super::execution::{cleanup_exited_process, store_runnable_process, take_runnable_process};
use super::supervision_tests::{add_monitor, insert_process, make_shared_state};
use super::*;
use crate::process::ExitReason;

const WATCHER: u64 = 1;
const WATCHED: u64 = 20_000;

fn watcher_of_many() -> Arc<SharedState> {
    let shared = make_shared_state();
    insert_process(&shared, WATCHER);
    for target in 2..2 + WATCHED {
        insert_process(&shared, target);
        add_monitor(&shared, WATCHER, target);
    }
    shared
}

fn watcher_monitors(shared: &SharedState) -> usize {
    let entry = shared
        .process_bodies
        .get(&WATCHER)
        .unwrap_or_else(|| panic!("the watcher is alive"));
    let slot = lock_or_recover(&entry);
    match &*slot {
        ProcessSlot::Present(ScheduledProcess(process)) => process.monitors().len(),
        _ => panic!("the watcher is stored back"),
    }
}

#[test]
fn running_a_watcher_of_many_costs_nothing_extra_per_run() {
    const RUNS: u32 = 2_000;
    let shared = watcher_of_many();

    let started = Instant::now();
    for _ in 0..RUNS {
        let process = take_runnable_process(&shared, WATCHER)
            .unwrap_or_else(|| panic!("the watcher is runnable"));
        store_runnable_process(&shared, process);
    }
    let took = started.elapsed();

    assert_eq!(
        watcher_monitors(&shared),
        usize::try_from(WATCHED).unwrap_or(0)
    );
    assert!(
        took < Duration::from_secs(1),
        "{RUNS} runs of a watcher of {WATCHED} took {took:?}"
    );
}

#[test]
fn every_watched_process_exiting_while_the_watcher_runs_stays_linear() {
    // The watcher is mid-run for most exits, so each DOWN lands on its slot:
    // the slot's copy of the monitors is the one that changes.
    let shared = watcher_of_many();

    let started = Instant::now();
    let mut running = take_runnable_process(&shared, WATCHER);
    for target in 2..2 + WATCHED {
        cleanup_exited_process(&shared, target, ExitReason::Normal);
        if target % 100 == 0 {
            if let Some(process) = running.take() {
                store_runnable_process(&shared, process);
            }
            running = take_runnable_process(&shared, WATCHER);
        }
    }
    if let Some(process) = running.take() {
        store_runnable_process(&shared, process);
    }
    let took = started.elapsed();

    assert_eq!(watcher_monitors(&shared), 0, "every DOWN took its monitor");
    assert!(
        took < Duration::from_secs(5),
        "{WATCHED} exits under a running watcher took {took:?}"
    );
}
