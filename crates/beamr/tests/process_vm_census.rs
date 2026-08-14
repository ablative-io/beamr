//! #106 positive control for the process-global VM census.
//!
//! Runs as its own test binary ON PURPOSE: cargo gives each integration test
//! binary its own process, so this file's single test owns the census and
//! can assert absolute values. Do not add unrelated tests here — a second
//! concurrently-running test constructing schedulers would race the counts.

use std::sync::Arc;

use beamr::module::ModuleRegistry;
use beamr::scheduler::{NativeBifs, Scheduler, SchedulerConfig, process_vm_census};

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

#[test]
fn census_counts_constructed_not_yet_dropped_vms_with_their_spawned_workers() {
    let baseline = process_vm_census();
    assert_eq!(baseline.vm_count, 0, "this binary owns its process");
    assert_eq!(baseline.scheduler_worker_count, 0);

    let one_worker = scheduler_with_workers(1);
    let three_workers = scheduler_with_workers(3);
    let both = process_vm_census();
    assert_eq!(both.vm_count, 2);
    assert_eq!(
        both.scheduler_worker_count, 4,
        "the census sums the counts each VM actually spawned (1 + 3)"
    );

    // The ruled population is "constructed and not yet dropped", NOT
    // "running": a shut-down scheduler still counts until it drops. This
    // assert is the population claim's own pin — it goes red if anyone
    // "fixes" the census to deregister at shutdown without re-ruling the
    // population and its rustdoc.
    one_worker.shutdown();
    let after_shutdown = process_vm_census();
    assert_eq!(
        after_shutdown.vm_count, 2,
        "shutdown must not deregister: the census counts constructed-not-dropped"
    );
    assert_eq!(after_shutdown.scheduler_worker_count, 4);

    drop(one_worker);
    let after_first_drop = process_vm_census();
    assert_eq!(after_first_drop.vm_count, 1);
    assert_eq!(after_first_drop.scheduler_worker_count, 3);

    three_workers.shutdown();
    drop(three_workers);
    let after_all = process_vm_census();
    assert_eq!(
        after_all.vm_count, 0,
        "every registration pairs with its drop"
    );
    assert_eq!(after_all.scheduler_worker_count, 0);
}
