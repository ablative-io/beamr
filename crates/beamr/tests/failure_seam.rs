//! B-184 lead amendment B: an embedder's red runs the real panic, guard and
//! latch through the public test-support seam. Built with the `test-support`
//! feature (the crate's dev-dependency on itself turns it on for every
//! integration test). Every wait ends on an event: the seam's mark, the
//! failure callback's channel, or a thread's join. None ends on a clock.

use std::sync::Arc;

use beamr::module::ModuleRegistry;
use beamr::scheduler::{
    NativeBifs, Scheduler, SchedulerConfig, WorkerEnd, WorkerSite, park_seam_panic_message,
};

#[test]
fn a_named_worker_panicked_through_the_public_seam_latches_its_failure() {
    let scheduler = Arc::new(
        Scheduler::new(
            SchedulerConfig {
                thread_count: Some(2),
                ..SchedulerConfig::default()
            },
            Arc::new(ModuleRegistry::new()),
            NativeBifs::none(),
        )
        .unwrap_or_else(|error| panic!("scheduler starts: {error}")),
    );
    let (called, calls) = std::sync::mpsc::channel();
    assert!(scheduler.on_failure(move |failure| {
        let thread = std::thread::current().name().map(str::to_string);
        let _sent = called.send((thread, failure.clone()));
    }));
    assert_eq!(scheduler.failure(), None);

    assert!(scheduler.panic_worker_at_next_park(1));
    assert!(scheduler.await_seam_panicked(1));
    let (thread, called_with) = calls
        .recv()
        .unwrap_or_else(|error| panic!("the failure callback runs: {error}"));

    let failure = scheduler
        .failure()
        .unwrap_or_else(|| panic!("the failure is latched"));
    assert_eq!(failure.worker(), "beamr-sched-1");
    assert_eq!(failure.worker_index(), 1);
    assert_eq!(failure.site(), WorkerSite::Parked);
    assert_eq!(
        failure.end(),
        &WorkerEnd::Panicked(park_seam_panic_message(1))
    );
    assert_eq!(
        failure.panic_message(),
        Some(
            "B-184 park seam: beamr-sched-1 panicked in its park, where std's condvar \
             assert ended the workers on macOS"
        )
    );
    assert_eq!(called_with, failure);
    assert_eq!(thread.as_deref(), Some("beamr-sched-1"));
    assert_eq!(
        scheduler.run_until_exit(1).map(|(reason, _)| reason),
        Err(failure)
    );

    let stopping = Arc::clone(&scheduler);
    let shutdown = std::thread::spawn(move || stopping.shutdown()).join();
    assert!(shutdown.is_ok(), "shutdown returns after a worker died");
}
