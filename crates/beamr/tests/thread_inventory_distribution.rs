//! Isolated-process pin for spec §5 permanent assertion 3: `distribution: None`
//! builds NEITHER tokio runtime, and `Some(config)` builds exactly the two
//! named runtime workers — checked against the OS thread probe, the ground
//! truth the inventory is validated against.
//!
//! Like `thread_inventory.rs`, this needs a QUIET process: the parallel `--lib`
//! harness has co-resident schedulers whose distribution runtimes make a
//! process-wide diff unattributable there (the entity-local half of the
//! assertion lives in `inventory_tests`). This binary is the only test in its
//! own OS process, so the "beamr-dist-send"/"beamr-net-kernel" workers it
//! observes are exactly the ones the scheduler under test built.

#![cfg(feature = "threads")]

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use beamr::distribution::DistributionConfig;
use beamr::distribution::NET_KERNEL_THREAD_NAME;
use beamr::distribution::sender::DIST_SEND_THREAD_NAME;
use beamr::module::ModuleRegistry;
use beamr::scheduler::thread_probe::{process_thread_names, thread_name_multiset};
use beamr::scheduler::{Scheduler, SchedulerConfig};

fn live_count(name: &str) -> usize {
    thread_name_multiset(&process_thread_names())
        .get(name)
        .copied()
        .unwrap_or(0)
}

/// Wait (bounded) until both distribution runtime workers are live, or the
/// window elapses — tokio names its workers a moment after spawn.
fn settle_until(predicate: impl Fn() -> bool) -> bool {
    for _ in 0..200 {
        if predicate() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    predicate()
}

#[test]
fn distribution_runtimes_exist_only_when_configured_and_are_joined_at_shutdown() {
    // ---- Positive control: Some(config) ⇒ BOTH named workers live in the OS.
    let owned = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            distribution: Some(DistributionConfig::default()),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    assert!(
        settle_until(
            || live_count(DIST_SEND_THREAD_NAME) >= 1 && live_count(NET_KERNEL_THREAD_NAME) >= 1
        ),
        "an owned distribution bundle must build both runtime workers: \
         dist-send={}, net-kernel={}",
        live_count(DIST_SEND_THREAD_NAME),
        live_count(NET_KERNEL_THREAD_NAME),
    );
    // EXACTLY one of each — this isolated process holds one scheduler, so a
    // second worker under either name is a duplicated-runtime construction
    // regression (the single-bundle claim), which a >=1 check would mask.
    assert_eq!(
        live_count(DIST_SEND_THREAD_NAME),
        1,
        "one bundle owns exactly one sender runtime worker"
    );
    assert_eq!(
        live_count(NET_KERNEL_THREAD_NAME),
        1,
        "one bundle owns exactly one net-kernel runtime worker"
    );

    // Shutdown JOINS both runtime workers BEFORE returning (spec §4): the probe
    // is checked immediately, with no settle window — an eventual-cleanup
    // regression (the old unjoined-helper drop) would empty the probe a moment
    // later and pass a settled check, so a settled check here is a pin that
    // can't fail.
    owned.shutdown();
    assert_eq!(
        live_count(DIST_SEND_THREAD_NAME),
        0,
        "shutdown must join the sender runtime worker before returning"
    );
    assert_eq!(
        live_count(NET_KERNEL_THREAD_NAME),
        0,
        "shutdown must join the net-kernel runtime worker before returning"
    );

    // ---- Negative gate: None ⇒ NEITHER runtime is ever built (honest absence).
    // With the owned scheduler joined, the process holds zero distribution
    // workers; a None scheduler must add none.
    assert_eq!(live_count(DIST_SEND_THREAD_NAME), 0);
    assert_eq!(live_count(NET_KERNEL_THREAD_NAME), 0);

    let disabled = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    assert!(SchedulerConfig::default().distribution.is_none());

    // Give it the same settle window the owned bundle needed; the workers must
    // still never appear.
    thread::sleep(Duration::from_millis(200));
    assert_eq!(
        live_count(DIST_SEND_THREAD_NAME),
        0,
        "distribution: None must not build the sender runtime"
    );
    assert_eq!(
        live_count(NET_KERNEL_THREAD_NAME),
        0,
        "distribution: None must not build the net-kernel runtime"
    );

    // And the scheduler claims neither in its own inventory.
    for entry in disabled.service_inventory() {
        assert!(
            !entry
                .thread_names
                .iter()
                .any(|name| name == DIST_SEND_THREAD_NAME || name == NET_KERNEL_THREAD_NAME),
            "a Disabled distribution bundle claims no runtime worker: {:?}",
            entry.thread_names
        );
    }

    disabled.shutdown();
}
