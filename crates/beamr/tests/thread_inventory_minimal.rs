//! Isolated-process pin for spec §5 permanent assertion 1: the `minimal()`
//! profile's EXACT thread set — the requested normal workers and NOTHING else.
//!
//! `SchedulerServices::minimal()` disables every ancillary service on a LIVE
//! scheduler (the first time each of the file/standard/generic rings and
//! distribution is reachable-Disabled outside replay, spec §2.2/§3.4), so this
//! is the profile that proves "the embedder pays for nothing it doesn't ask
//! for" (spec §0). Like the sibling `thread_inventory*.rs` pins it needs a QUIET
//! process — the parallel `--lib` harness has co-resident schedulers whose
//! threads make a process-wide probe unattributable — so it holds baseline and
//! diff in a single `#[test]` in its own integration binary.

#![cfg(feature = "threads")]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use beamr::module::ModuleRegistry;
use beamr::scheduler::thread_probe::{process_thread_names, thread_name_multiset};
use beamr::scheduler::{Scheduler, SchedulerConfig, SchedulerServices, ServiceModeLabel};

/// Every thread the scheduler attributes to itself: normal workers (outside the
/// service model, spec §2.3) plus every inventory entry's names.
fn claimed_names(scheduler: &Scheduler) -> Vec<String> {
    let mut claimed: Vec<String> = scheduler.worker_names().to_vec();
    for entry in scheduler.service_inventory() {
        claimed.extend(entry.thread_names);
    }
    claimed
}

#[test]
fn minimal_profile_runs_only_the_requested_workers_and_nothing_else() {
    // T1-grade methodology (comment-as-contract):
    //  - sampling source: `process_thread_names` (mach thread ports on macOS;
    //    `/proc/self/task/*/comm` on Linux).
    //  - host state: a fresh integration-test process; the only threads created
    //    between the two probes are this one minimal scheduler's own.
    //  - assertion: the baseline delta equals EXACTLY the claimed set (macOS),
    //    the claim is EXACTLY the two requested `beamr-sched-*` workers, and no
    //    ancillary thread (dirty / file / standard / generic / distribution)
    //    appears anywhere.
    let baseline = thread_name_multiset(&process_thread_names());

    let scheduler = Scheduler::with_services(
        SchedulerConfig {
            thread_count: Some(2),
            ..SchedulerConfig::default()
        },
        SchedulerServices::minimal(),
        Arc::new(ModuleRegistry::new()),
    )
    .unwrap_or_else(|error| panic!("minimal scheduler starts: {error}"));

    // Inventory: EVERY ancillary service is Disabled with zero live threads and
    // the DISABLED instance sentinel — the negative half of assertion 1/2.
    for entry in scheduler.service_inventory() {
        assert_eq!(
            entry.mode,
            ServiceModeLabel::Disabled,
            "{} must be Disabled under minimal()",
            entry.service
        );
        assert_eq!(entry.actual, 0, "{} owns no threads", entry.service);
        assert!(
            entry.thread_names.is_empty(),
            "{} names no threads",
            entry.service
        );
    }
    // The transient policy lines follow the disabled services: nothing can spawn.
    for policy in scheduler.service_policies() {
        assert_eq!(
            policy.mode,
            ServiceModeLabel::Disabled,
            "policy {} must be Disabled under minimal()",
            policy.policy
        );
    }
    // No standard-IO ring ⇒ NO process 0 registered (spec §3.4). A legacy
    // scheduler reports one process here; minimal() reports zero — the pin that
    // the eager-process-0 assumption really moved to its profile.
    assert_eq!(
        scheduler.process_count(),
        0,
        "minimal() registers no process 0 (standard-IO ring disabled)"
    );

    // The claim is EXACTLY the two requested workers, nothing else.
    let claimed = thread_name_multiset(&claimed_names(&scheduler));
    let mut expected = BTreeMap::new();
    expected.insert("beamr-sched-0".to_owned(), 1usize);
    expected.insert("beamr-sched-1".to_owned(), 1usize);
    assert_eq!(
        claimed, expected,
        "minimal() must claim exactly the requested normal workers"
    );

    #[cfg(any(target_os = "macos", target_os = "linux"))]
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
            if delta == expected {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            delta, expected,
            "a minimal scheduler's OS footprint must equal exactly its two \
             requested workers — no dirty, ring, process-0, or distribution thread"
        );
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = &baseline;
    }

    scheduler.shutdown();

    // Post-shutdown: assertion 4 — zero owned beamr-attributed threads remain
    // (the two workers joined; nothing else existed to leak).
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let mut residue = BTreeMap::new();
        for _ in 0..200 {
            let live = thread_name_multiset(&process_thread_names());
            residue = live
                .iter()
                .filter_map(|(name, live_count)| {
                    let base = baseline.get(name).copied().unwrap_or(0);
                    let extra = live_count.saturating_sub(base);
                    (name.starts_with("beamr-sched-") && extra > 0).then(|| (name.clone(), extra))
                })
                .collect();
            if residue.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            residue.is_empty(),
            "shutdown must join every worker the minimal scheduler owned: {residue:?}"
        );
    }
}
