//! Permanent (rule-1) pinning tests for the service inventory and the signed
//! §3.8 idle-tick floor. These assert CURRENT as-built behavior: every service
//! is eager and `Owned` today (spec §11 commit 1). They flip service-by-service
//! in the later commits, and the same assertions catch any silent drift.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::service::ServiceModeLabel;
use super::{Scheduler, SchedulerConfig, dirty, execution, inventory, thread_probe};
use crate::module::ModuleRegistry;

fn new_default_scheduler() -> Scheduler {
    Scheduler::new(SchedulerConfig::default(), Arc::new(ModuleRegistry::new()))
        .unwrap_or_else(|error| panic!("scheduler starts: {error}"))
}

fn entries_by_service(
    scheduler: &Scheduler,
) -> BTreeMap<&'static str, inventory::ServiceInventoryEntry> {
    scheduler
        .service_inventory()
        .into_iter()
        .map(|entry| (entry.service, entry))
        .collect()
}

/// Assertion-1 style: the default profile enumerates the §1 table's ancillary
/// services with today's counts and today's (not-yet-distinct) thread names.
#[test]
fn default_profile_pins_as_built_service_inventory() {
    // The default config carries no distribution, and — the honest-None contract
    // (spec §3.6) — that now means the bundle is Disabled: NEITHER runtime is
    // built. Commit 4's change from `unwrap_or_default()`.
    assert!(SchedulerConfig::default().distribution.is_none());

    let scheduler = new_default_scheduler();
    let by_service = entries_by_service(&scheduler);

    // EXACT service-label set: a new ancillary service cannot ship without
    // appearing here (and thereby re-answering lens Q1/Q2). Distribution is now
    // ONE coherent bundle line (spec §3.6), not a dist-sender/net-kernel pair.
    let labels: Vec<&'static str> = scheduler
        .service_inventory()
        .iter()
        .map(|entry| entry.service)
        .collect();
    assert_eq!(
        labels,
        vec![
            inventory::DIRTY_CPU,
            inventory::DIRTY_IO,
            inventory::FILE_IO_RING,
            inventory::STANDARD_IO_RING,
            inventory::GENERIC_IO_RING,
            inventory::DISTRIBUTION,
        ],
        "the inventory enumerates exactly the service set, in order"
    );

    // Dirty CPU pool: num_cpus, coerced up by `.max(1)`.
    let expected_dirty_cpu = num_cpus::get().max(1);
    let dirty_cpu = &by_service[inventory::DIRTY_CPU];
    assert_eq!(dirty_cpu.mode, ServiceModeLabel::Owned);
    assert_eq!(dirty_cpu.actual, expected_dirty_cpu);
    assert_eq!(dirty_cpu.configured, expected_dirty_cpu);
    let expected_dirty_cpu_names: Vec<String> = (0..expected_dirty_cpu)
        .map(|index| format!("dirty-cpu-{index}"))
        .collect();
    assert_eq!(dirty_cpu.thread_names, expected_dirty_cpu_names);

    // Dirty IO pool: fixed at 10 today.
    let dirty_io = &by_service[inventory::DIRTY_IO];
    assert_eq!(dirty_io.mode, ServiceModeLabel::Owned);
    assert_eq!(dirty_io.actual, dirty::DEFAULT_DIRTY_IO_THREADS);
    let expected_dirty_io_names: Vec<String> = (0..dirty::DEFAULT_DIRTY_IO_THREADS)
        .map(|index| format!("dirty-io-{index}"))
        .collect();
    assert_eq!(dirty_io.thread_names, expected_dirty_io_names);

    // Generic IO ring: off by default (`config.io: None` is a true absence).
    let generic = &by_service[inventory::GENERIC_IO_RING];
    assert_eq!(generic.mode, ServiceModeLabel::Disabled);
    assert_eq!(generic.actual, 0);
    assert!(generic.thread_names.is_empty());

    // Distribution: honest None ⇒ the bundle is Disabled, NEITHER runtime built
    // (spec §3.6). Zero threads, zero configured, the DISABLED instance sentinel.
    let distribution = &by_service[inventory::DISTRIBUTION];
    assert_eq!(distribution.mode, ServiceModeLabel::Disabled);
    assert_eq!(distribution.actual, 0);
    assert_eq!(distribution.configured, 0);
    assert!(distribution.thread_names.is_empty());
    assert_eq!(
        distribution.instance,
        super::service::ServiceInstanceId::DISABLED
    );

    // The heartbeat is a task-class policy line (spec §3.7), Disabled here since
    // distribution is off — never a thread line.
    let heartbeat = scheduler
        .service_policies()
        .into_iter()
        .find(|line| line.policy == inventory::HEARTBEAT)
        .expect("heartbeat policy line present");
    assert_eq!(heartbeat.mode, ServiceModeLabel::Disabled);
    assert_eq!(heartbeat.spawned_total, 0);

    // Two 4-thread fallback rings on non-Linux, now with SERVICE-DISTINCT
    // thread-name prefixes (spec §5): the three-way `beamr-io-thread-pool-*`
    // collision between the file, standard, and generic rings is gone. This
    // pins the new names exactly — the exactness does not weaken, only the
    // label values change (pair-ruled: commit 3 builds against this test and
    // must not loosen it). io_uring on Linux owns no named OS worker threads.
    let file_ring = &by_service[inventory::FILE_IO_RING];
    let standard_ring = &by_service[inventory::STANDARD_IO_RING];
    assert_eq!(file_ring.mode, ServiceModeLabel::Owned);
    assert_eq!(standard_ring.mode, ServiceModeLabel::Owned);
    // File and standard rings carry distinct process-wide instance identities.
    assert_ne!(file_ring.instance, standard_ring.instance);
    #[cfg(not(target_os = "linux"))]
    {
        let expected_file_names: Vec<String> = (0..4)
            .map(|index| format!("{}-{index}", crate::io::FILE_IO_RING_THREAD_PREFIX))
            .collect();
        let expected_standard_names: Vec<String> = (0..4)
            .map(|index| format!("{}-{index}", crate::io::STANDARD_IO_RING_THREAD_PREFIX))
            .collect();
        assert_eq!(file_ring.actual, 4);
        assert_eq!(file_ring.thread_names, expected_file_names);
        assert_eq!(standard_ring.actual, 4);
        assert_eq!(standard_ring.thread_names, expected_standard_names);
        // The distinct prefixes really are distinct — no shared name survives.
        assert!(
            expected_file_names
                .iter()
                .all(|name| !expected_standard_names.contains(name)),
            "file and standard ring worker names must not collide"
        );
    }
    #[cfg(target_os = "linux")]
    {
        assert_eq!(file_ring.actual, 0);
        assert_eq!(standard_ring.actual, 0);
    }

    scheduler.shutdown();
}

/// Permanent assertion 3 (spec §5), entity-local half: `distribution: None` ⇒
/// the bundle is Disabled and claims ZERO runtime workers; `Some(config)` ⇒ the
/// bundle is Owned and claims exactly the two NAMED runtime workers. The
/// positive control is what makes the negative gate a real pin — if the
/// instrumentation stopped reporting the runtimes, the Some half would fail. The
/// exact OS-probe form (no such threads actually live) is the quiet-process
/// `tests/thread_inventory_distribution.rs`.
#[test]
fn distribution_none_disables_the_bundle_and_some_names_both_runtimes() {
    // Negative gate: None ⇒ Disabled bundle, zero runtimes, heartbeat off.
    let none = new_default_scheduler();
    let none_entry = entries_by_service(&none)[inventory::DISTRIBUTION].clone();
    assert_eq!(none_entry.mode, ServiceModeLabel::Disabled);
    assert_eq!(none_entry.actual, 0);
    assert!(none_entry.thread_names.is_empty());
    none.shutdown();

    // Positive control: Some ⇒ Owned bundle claiming BOTH named runtime workers
    // in ONE §5 entry (spec §3.6), and the heartbeat policy line goes Owned.
    let some = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            distribution: Some(crate::distribution::DistributionConfig::default()),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let some_entry = entries_by_service(&some)[inventory::DISTRIBUTION].clone();
    assert_eq!(some_entry.mode, ServiceModeLabel::Owned);
    assert_eq!(some_entry.configured, 2);
    assert_eq!(some_entry.actual, 2);
    assert!(
        some_entry
            .thread_names
            .contains(&crate::distribution::sender::DIST_SEND_THREAD_NAME.to_owned()),
        "the sender runtime worker is named in the bundle line: {:?}",
        some_entry.thread_names
    );
    assert!(
        some_entry
            .thread_names
            .contains(&crate::distribution::NET_KERNEL_THREAD_NAME.to_owned()),
        "the net-kernel runtime worker is named in the bundle line: {:?}",
        some_entry.thread_names
    );
    assert_ne!(
        some_entry.instance,
        super::service::ServiceInstanceId::DISABLED
    );

    let heartbeat = some
        .service_policies()
        .into_iter()
        .find(|line| line.policy == inventory::HEARTBEAT)
        .expect("heartbeat policy line present");
    assert_eq!(heartbeat.mode, ServiceModeLabel::Owned);

    some.shutdown();
}

/// A `dirty_*_threads: Some(0)` REQUEST disables the pool (spec §3.2/§6): the
/// inventory reports the §5 Disabled entry (zero threads, zero fds, DISABLED
/// sentinel instance), the OS probe sees no `dirty-*` worker, and the
/// `dirty-complete` policy line follows the pools to Disabled once both are
/// off. This is the behavior change from the old `.max(1)` coercion.
#[test]
fn disabled_dirty_pools_report_disabled_entries_and_policy() {
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            dirty_cpu_threads: Some(0),
            dirty_io_threads: Some(0),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let by_service = entries_by_service(&scheduler);
    for service in [inventory::DIRTY_CPU, inventory::DIRTY_IO] {
        let entry = &by_service[service];
        assert_eq!(entry.mode, ServiceModeLabel::Disabled, "{service} disabled");
        assert_eq!(entry.actual, 0);
        assert_eq!(entry.configured, 0);
        assert!(entry.thread_names.is_empty());
        assert!(entry.fd_classes.is_empty());
        assert_eq!(entry.instance, super::service::ServiceInstanceId::DISABLED);
    }

    // The exact zero-worker OS-probe delta is asserted in the isolated
    // `tests/thread_inventory.rs` process; here the parallel `--lib` harness
    // has co-resident schedulers with their own dirty pools, so a process-wide
    // probe is unattributable (same reason the exact assertion-6 lives there).

    // The transient completion policy follows the pools: both off ⇒ Disabled.
    let dirty_complete = scheduler
        .service_policies()
        .into_iter()
        .find(|line| line.policy == inventory::DIRTY_COMPLETE)
        .expect("dirty-complete policy line present");
    assert_eq!(dirty_complete.mode, ServiceModeLabel::Disabled);
    assert_eq!(dirty_complete.spawned_total, 0);

    scheduler.shutdown();
}

/// The transient `dirty-complete-{pid}` thread is a policy line with a counter,
/// not a thread line (spec §5). At rest no dirty call has run, so the counter is
/// zero and no such name appears in any service entry.
#[test]
fn dirty_complete_is_a_policy_line_not_a_thread_line() {
    let scheduler = new_default_scheduler();

    let policies = scheduler.service_policies();
    let dirty_complete = policies
        .iter()
        .find(|line| line.policy == inventory::DIRTY_COMPLETE)
        .expect("dirty-complete policy line present");
    assert_eq!(dirty_complete.mode, ServiceModeLabel::Owned);
    assert_eq!(dirty_complete.spawned_total, 0);

    for entry in scheduler.service_inventory() {
        for name in &entry.thread_names {
            assert!(
                !name.starts_with("dirty-complete-"),
                "transient burst thread must not appear as a thread line: {name}"
            );
        }
    }

    scheduler.shutdown();
}

/// Instance ids are minted once at construction and stable across calls, so the
/// Q2 group-by has a fixed key to group on.
#[test]
fn instance_ids_are_stable_across_inventory_calls() {
    let scheduler = new_default_scheduler();
    let first = entries_by_service(&scheduler);
    let second = entries_by_service(&scheduler);
    for (service, entry) in &first {
        assert_eq!(entry.instance, second[service].instance);
    }
    scheduler.shutdown();
}

/// Assertion-6, robust form: every thread the inventory (plus the normal
/// workers that stay outside the model, spec §2.3) claims is actually live in
/// the OS right now.
///
/// T1-grade methodology (comment-as-contract):
///  - sampling source: [`thread_probe::process_thread_names`] (mach thread
///    ports + `pthread_getname_np` on macOS).
///  - host state: the just-constructed scheduler, threads settled.
///  - assertion: for each `(name, count)` the scheduler attributes, the live
///    probe shows AT LEAST `count` threads of that name.
///
/// Direction: containment only — this is the fast every-run smoke inside the
/// parallel `--lib` harness, where co-resident test schedulers with colliding
/// pre-commit-3 names make exact process-wide equality unattributable. The
/// EXACT two-directional form (baseline delta == claimed multiset, catching
/// un-inventoried threads too) lives in `tests/thread_inventory.rs`, which
/// runs in its own quiet OS process.
#[cfg(target_os = "macos")]
#[test]
fn service_inventory_threads_are_all_live_in_the_os_probe() {
    let scheduler = new_default_scheduler();

    let mut claimed: Vec<String> = scheduler.worker_names().to_vec();
    for entry in scheduler.service_inventory() {
        claimed.extend(entry.thread_names);
    }
    let claimed = thread_probe::thread_name_multiset(&claimed);

    // tokio worker threads apply their name a moment after spawn; retry until
    // every claimed thread is observed, or the window elapses.
    let mut live = BTreeMap::new();
    let mut satisfied = false;
    for _ in 0..100 {
        live = thread_probe::thread_name_multiset(&thread_probe::process_thread_names());
        satisfied = claimed
            .iter()
            .all(|(name, count)| live.get(name).copied().unwrap_or(0) >= *count);
        if satisfied {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(
        satisfied,
        "inventory claims threads absent from the OS probe: claimed={claimed:?} live={live:?}"
    );

    scheduler.shutdown();
}

/// The process-wide Q2 aggregate — [`inventory::deduped_thread_aggregate`],
/// the production enforcement helper (spec §9) — counts each Owned entry once,
/// each distinct Shared instance ONCE across schedulers, and Disabled never.
#[test]
fn process_wide_aggregate_dedups_shared_instances_once() {
    // Real half: two co-resident all-Owned schedulers — every instance is
    // distinct, so the deduped aggregate equals the plain sum, and every
    // Owned instance id must be process-unique across the two inventories.
    let first = new_default_scheduler();
    let second = new_default_scheduler();
    let mut combined: Vec<inventory::ServiceInventoryEntry> = Vec::new();
    combined.extend(first.service_inventory());
    combined.extend(second.service_inventory());

    let mut seen = std::collections::BTreeSet::new();
    let mut naive_sum = 0usize;
    for entry in &combined {
        if entry.mode == ServiceModeLabel::Disabled {
            continue;
        }
        naive_sum += entry.actual;
        assert!(
            seen.insert(entry.instance),
            "Owned instance ids must be process-unique"
        );
    }
    assert_eq!(inventory::deduped_thread_aggregate(&combined), naive_sum);

    // Shared half: the SAME instance reported by two schedulers must be
    // counted once. Until commits 2-5 wire real Shared services, pin the
    // helper against synthesized entries carrying one shared identity.
    let shared_instance = super::service::ServiceInstanceId::mint();
    let shared_entry = |actual: usize| inventory::ServiceInventoryEntry {
        service: "shared-ring",
        mode: ServiceModeLabel::Shared,
        instance: shared_instance,
        configured: actual,
        actual,
        thread_names: Vec::new(),
        fd_classes: Vec::new(),
    };
    let two_reporters = vec![shared_entry(4), shared_entry(4)];
    assert_eq!(
        inventory::deduped_thread_aggregate(&two_reporters),
        4,
        "a shared 4-thread ring serving two schedulers bills 4, never 8"
    );

    // Disabled contributes nothing, whatever its neighbors.
    let mut with_disabled = two_reporters;
    with_disabled.push(inventory::ServiceInventoryEntry {
        service: "off",
        mode: ServiceModeLabel::Disabled,
        instance: super::service::ServiceInstanceId::DISABLED,
        configured: 0,
        actual: 0,
        thread_names: Vec::new(),
        fd_classes: Vec::new(),
    });
    assert_eq!(inventory::deduped_thread_aggregate(&with_disabled), 4);

    first.shutdown();
    second.shutdown();
}

/// After `shutdown()` the inventory stays truthful in BOTH directions (spec §4):
/// every joined service — dirty pools, file ring, standard-IO ring, AND now the
/// distribution bundle's BOTH runtime workers — reports zero live threads, while
/// `configured` keeps the construction request. This pins the §4 distribution
/// teardown rewrite: the runtime workers that used to survive `shutdown()` until
/// the last Arc drop are now JOINED synchronously before it returns.
///
/// Uses an explicit distribution profile so the join is exercised (the default
/// profile is Disabled — zero runtimes to join): the negative "no leak after
/// shutdown" is only a real pin against a scheduler that actually built them.
#[test]
fn post_shutdown_inventory_reports_all_joined_services_as_zero() {
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            distribution: Some(crate::distribution::DistributionConfig::default()),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let before = entries_by_service(&scheduler);
    // The distribution bundle really did build both runtime workers.
    assert_eq!(
        before[inventory::DISTRIBUTION].mode,
        ServiceModeLabel::Owned
    );
    assert_eq!(
        before[inventory::DISTRIBUTION].configured,
        2,
        "owned bundle requests the sender + net-kernel runtimes"
    );

    scheduler.shutdown();
    let after = entries_by_service(&scheduler);

    // Joined by shutdown(): live count drops to zero, request preserved.
    // (On Linux the file/standard rings are io_uring — zero named workers both
    // sides.) The distribution bundle joins BOTH tokio runtime workers HERE now
    // (spec §4): a post-shutdown inventory is truthful rather than reporting
    // workers that outlive shutdown() until the last Arc drop.
    for service in [
        inventory::DIRTY_CPU,
        inventory::DIRTY_IO,
        inventory::FILE_IO_RING,
        inventory::STANDARD_IO_RING,
        inventory::DISTRIBUTION,
    ] {
        assert_eq!(after[service].actual, 0, "{service} joined at shutdown");
        assert_eq!(after[service].configured, before[service].configured);
    }
    // Double shutdown is idempotent — no panic, no hang, still zero.
    scheduler.shutdown();
    let twice = entries_by_service(&scheduler);
    assert_eq!(twice[inventory::DISTRIBUTION].actual, 0);
}

/// With generic IO enabled, `configured` is the CONSTRUCTION request — ring
/// workers plus the completion bridge — and must not change when shutdown
/// stops the bridge and joins the ring: the request is history, not liveness.
/// `actual` truthfully drops to zero (shutdown joins the generic ring and
/// stops the bridge).
#[test]
fn generic_io_configured_is_stable_across_shutdown() {
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            io: Some(crate::io::RingConfig::default()),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let before = entries_by_service(&scheduler)[inventory::GENERIC_IO_RING].clone();
    assert_eq!(before.mode, ServiceModeLabel::Owned);
    assert_eq!(
        before.actual, before.configured,
        "all requested generic-IO threads live at construction"
    );
    assert!(
        before
            .thread_names
            .contains(&crate::io::bridge::IO_COMPLETION_THREAD_NAME.to_owned()),
        "the completion bridge is part of the generic-IO line"
    );

    scheduler.shutdown();
    let after = entries_by_service(&scheduler)[inventory::GENERIC_IO_RING].clone();
    assert_eq!(
        after.configured, before.configured,
        "the construction request must survive shutdown unchanged"
    );
    assert_eq!(after.actual, 0, "ring joined and bridge stopped");
}

/// Structural pin of the 5ms idle-park floor at its source (Q-F ruling, spec
/// §3.8): the constant, and the per-worker wake rate derived FROM it, are what
/// the certifying pair signs. Reading the floor from `IDLE_PARK_TIMEOUT` rather
/// than a duplicated literal is why the signed number and `park_thread` cannot
/// drift.
#[test]
fn idle_park_floor_is_pinned_at_five_ms() {
    assert_eq!(execution::IDLE_PARK_TIMEOUT, Duration::from_millis(5));
    assert_eq!(execution::IDLE_WAKES_PER_SEC_PER_WORKER, 200);
}

/// Signed idle-wake bound (spec §3.8 / §7): the measured idle wake rate stays
/// under the formula's ceiling.
///
/// T1-grade methodology (comment-as-contract):
///  - measurement duration: 500 ms of wall-clock idle.
///  - sampling source: `SharedState::idle_parks`, incremented once per
///    `park_thread` entry (every worker idle wake).
///  - host state: a fresh scheduler with no runnable process beyond the parked
///    standard-IO server — every normal worker parks.
///  - bound: `<= 2 x IDLE_WAKES_PER_SEC_PER_WORKER x workers`, where `workers`
///    is the AS-BUILT normal-worker thread count (`worker_names().len()`, the
///    same actual-thread source the inventory reads — not a config claim, per
///    the signing note). The 2x margin makes it a ceiling, never an exact
///    match, so a merely-loaded host does not flake it.
///
/// Note: normal workers stay outside `service_inventory()` (spec §2.3), so the
/// worker multiplier is sourced from `worker_names()`; both are populated from
/// actually-spawned threads, satisfying "not what a config file claims".
#[test]
fn idle_wake_rate_stays_within_signed_bound() {
    let scheduler = new_default_scheduler();
    let workers = scheduler.worker_names().len();
    assert!(workers >= 1, "a scheduler always has at least one worker");

    // Let the workers reach steady-state idle parking.
    thread::sleep(Duration::from_millis(50));

    let window = Duration::from_millis(500);
    let start = scheduler.idle_park_count();
    let started_at = std::time::Instant::now();
    thread::sleep(window);
    let parks = scheduler.idle_park_count().saturating_sub(start);
    // Divide by the ACTUAL elapsed wall time, not the requested sleep: a
    // loaded host that overslept the test thread must not inflate the rate
    // (workers kept parking during the oversleep).
    let elapsed = started_at.elapsed();

    let wakes_per_sec = parks as f64 / elapsed.as_secs_f64();
    let ceiling = 2.0 * execution::IDLE_WAKES_PER_SEC_PER_WORKER as f64 * workers as f64;
    assert!(
        wakes_per_sec <= ceiling,
        "idle wake rate {wakes_per_sec:.1}/s exceeds signed ceiling {ceiling:.1}/s \
         for {workers} workers (floor {}ms)",
        execution::IDLE_PARK_TIMEOUT.as_millis(),
    );

    scheduler.shutdown();
}
