//! Isolated-process pin for spec §5 assertion 2 on the dirty pools: a
//! `dirty_*_threads: Some(0)` request yields a Disabled service that owns ZERO
//! OS threads (spec §3.2/§6).
//!
//! Like `thread_inventory.rs`, this needs a QUIET process — the parallel
//! `--lib` harness has co-resident schedulers with their own (identically
//! named, pre-commit-3) dirty pools, so a process-wide probe there is
//! unattributable. This binary holds baseline and diff in a single `#[test]`
//! so no sibling pollutes the probe between them.

#![cfg(feature = "threads")]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use beamr::module::ModuleRegistry;
use beamr::scheduler::ServiceModeLabel;
use beamr::scheduler::thread_probe::{process_thread_names, thread_name_multiset};
use beamr::scheduler::{Scheduler, SchedulerConfig};

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
fn disabled_dirty_pools_own_zero_os_threads() {
    // T1-grade methodology (comment-as-contract):
    //  - sampling source: `process_thread_names` (mach thread ports on macOS;
    //    `/proc/self/task/*/comm` on Linux).
    //  - host state: a fresh integration-test process; the only threads created
    //    between the two probes are this one scheduler's own.
    //  - assertion: the baseline delta equals exactly what the scheduler claims
    //    (macOS), and the claimed set contains NO `dirty-*` worker — a disabled
    //    pool spawns nothing.
    let baseline = thread_name_multiset(&process_thread_names());

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

    // Inventory: both dirty pools are Disabled with zero live threads.
    for entry in scheduler.service_inventory() {
        if entry.service == "dirty-cpu" || entry.service == "dirty-io" {
            assert_eq!(entry.mode, ServiceModeLabel::Disabled, "{}", entry.service);
            assert_eq!(entry.actual, 0);
            assert!(entry.thread_names.is_empty());
        }
    }

    let claimed = thread_name_multiset(&claimed_names(&scheduler));
    assert!(
        !claimed
            .keys()
            .any(|name| name.starts_with("dirty-cpu-") || name.starts_with("dirty-io-")),
        "a disabled dirty pool must claim no worker thread: {claimed:?}"
    );

    #[cfg(target_os = "macos")]
    {
        let mut delta = BTreeMap::new();
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
            "a disabled-dirty scheduler's OS footprint must equal exactly what \
             it claims — and that claim carries no dirty worker"
        );
        // Directly: no dirty-* thread appeared in the delta.
        assert!(
            !delta
                .keys()
                .any(|name| name.starts_with("dirty-cpu-") || name.starts_with("dirty-io-")),
            "no dirty worker thread may spawn for a disabled pool: {delta:?}"
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = &baseline;
    }

    scheduler.shutdown();
}
