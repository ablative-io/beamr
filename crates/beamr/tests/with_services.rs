//! Composition entrypoint pins (spec §2.2): profiles, precedence, the SAFE
//! shared dirty pool, and the REFUSED shared IO ring.
//!
//! These exercise only the public composition surface
//! (`Scheduler::with_services` + `SchedulerServices`), so they double as the
//! embedder-facing API smoke test.

#![cfg(feature = "threads")]

use std::sync::Arc;

use beamr::distribution::DistributionConfig;
use beamr::io::RingConfig;
use beamr::module::ModuleRegistry;
use beamr::scheduler::dirty::DirtyPool;
use beamr::scheduler::{
    Scheduler, SchedulerConfig, SchedulerServices, ServiceModeLabel, SharedIoRing,
    WithServicesError, deduped_thread_aggregate,
};

fn registry() -> Arc<ModuleRegistry> {
    Arc::new(ModuleRegistry::new())
}

fn one_worker() -> SchedulerConfig {
    SchedulerConfig {
        thread_count: Some(1),
        ..SchedulerConfig::default()
    }
}

fn mode_of(scheduler: &Scheduler, service: &str) -> ServiceModeLabel {
    scheduler
        .service_inventory()
        .into_iter()
        .find(|entry| entry.service == service)
        .unwrap_or_else(|| panic!("service {service} present in inventory"))
        .mode
}

// ---------------------------------------------------------------------------
// Profiles + precedence (spec §2.2/§3.6).
// ---------------------------------------------------------------------------

#[test]
fn full_runtime_turns_distribution_on_and_keeps_other_services_from_config() {
    let scheduler =
        Scheduler::with_services(one_worker(), SchedulerServices::full_runtime(), registry())
            .expect("full_runtime scheduler starts");

    // full_runtime() explicitly owns distribution (the one service that is
    // honest-absent by default), while the dirty/IO services stay on their
    // legacy defaults (FromConfig ⇒ Owned).
    assert!(
        scheduler.try_distribution_config().is_some(),
        "full_runtime() owns a distribution bundle"
    );
    assert_eq!(mode_of(&scheduler, "distribution"), ServiceModeLabel::Owned);
    assert_eq!(mode_of(&scheduler, "dirty-cpu"), ServiceModeLabel::Owned);
    assert_eq!(
        mode_of(&scheduler, "standard-io-ring"),
        ServiceModeLabel::Owned
    );
    // Process 0 is live (standard-IO ring Owned).
    assert_eq!(scheduler.process_count(), 1);
    scheduler.shutdown();
}

#[test]
fn minimal_disables_every_ancillary_service() {
    let scheduler =
        Scheduler::with_services(one_worker(), SchedulerServices::minimal(), registry())
            .expect("minimal scheduler starts");

    for entry in scheduler.service_inventory() {
        assert_eq!(
            entry.mode,
            ServiceModeLabel::Disabled,
            "{} Disabled under minimal()",
            entry.service
        );
    }
    assert!(scheduler.try_distribution_config().is_none());
    assert!(scheduler.try_dirty_cpu_pool().is_none());
    assert!(scheduler.try_dirty_io_pool().is_none());
    assert_eq!(scheduler.process_count(), 0, "no process 0 under minimal()");
    scheduler.shutdown();
}

#[test]
fn explicit_service_choice_wins_over_the_legacy_config_knob() {
    // config asks (via the legacy knob) for a 3-worker dirty CPU pool, but the
    // explicit composition choice disables it: explicit wins (spec §2.2).
    let config = SchedulerConfig {
        thread_count: Some(1),
        dirty_cpu_threads: Some(3),
        ..SchedulerConfig::default()
    };
    let scheduler = Scheduler::with_services(
        config,
        SchedulerServices::from_config().disable_dirty_cpu(),
        registry(),
    )
    .expect("scheduler starts");
    assert!(
        scheduler.try_dirty_cpu_pool().is_none(),
        "explicit disable overrides dirty_cpu_threads: Some(3)"
    );
    // The dirty IO pool, left FromConfig, still honors its legacy default.
    assert!(scheduler.try_dirty_io_pool().is_some());
    scheduler.shutdown();
}

// ---------------------------------------------------------------------------
// Shared dirty pool — SAFE now (spec §2.1/§5 assertion 5). Dirty completion
// routes by the oneshot the submission carries, so one pool can back two
// schedulers; owner-only teardown means neither scheduler stops it.
// ---------------------------------------------------------------------------

#[test]
fn shared_dirty_pool_is_used_by_two_schedulers_and_stopped_by_neither() {
    // The embedder OWNS the pool; both schedulers inject it Shared.
    let pool = Arc::new(DirtyPool::new("dirty-cpu", 2));
    assert!(!pool.is_shutdown());

    let scheduler_a = Scheduler::with_services(
        one_worker(),
        SchedulerServices::minimal().shared_dirty_cpu(Arc::clone(&pool)),
        registry(),
    )
    .expect("scheduler A starts");
    let scheduler_b = Scheduler::with_services(
        one_worker(),
        SchedulerServices::minimal().shared_dirty_cpu(Arc::clone(&pool)),
        registry(),
    )
    .expect("scheduler B starts");

    // Both report the SAME instance id ⇒ the process-wide dedup collapses them
    // (spec §5/§9 Q2): one shared pool, counted once across both schedulers.
    let entry_a = scheduler_a
        .service_inventory()
        .into_iter()
        .find(|e| e.service == "dirty-cpu")
        .expect("dirty-cpu entry A");
    let entry_b = scheduler_b
        .service_inventory()
        .into_iter()
        .find(|e| e.service == "dirty-cpu")
        .expect("dirty-cpu entry B");
    assert_eq!(entry_a.mode, ServiceModeLabel::Shared);
    assert_eq!(entry_b.mode, ServiceModeLabel::Shared);
    assert_eq!(
        entry_a.instance, entry_b.instance,
        "a shared pool propagates one instance id to both schedulers"
    );
    let both: Vec<_> = scheduler_a
        .service_inventory()
        .into_iter()
        .chain(scheduler_b.service_inventory())
        .collect();
    assert_eq!(
        deduped_thread_aggregate(&both),
        pool.live_worker_names().len(),
        "the shared pool's threads are counted ONCE process-wide, not per scheduler"
    );

    // Scheduler A dies. NEGATIVE gate: the shared pool must SURVIVE — A never
    // stops a pool it does not own (owner-only teardown, spec §4 step 6).
    scheduler_a.shutdown();
    assert!(
        !pool.is_shutdown(),
        "a scheduler must not stop a shared dirty pool it does not own"
    );
    assert!(
        !pool.live_worker_names().is_empty(),
        "the shared pool's workers survive one scheduler's death"
    );

    // B still holds and uses it; B dying also does not stop it.
    scheduler_b.shutdown();
    assert!(
        !pool.is_shutdown(),
        "the second scheduler must not stop the shared pool either"
    );

    // POSITIVE control: the embedder-owner CAN stop it (proving the survival
    // above was owner-discipline, not an inability to stop).
    pool.shutdown();
    assert!(pool.is_shutdown());
    assert!(pool.live_worker_names().is_empty());
}

// ---------------------------------------------------------------------------
// Shared IO ring — REFUSED this release (spec §3.9): cross-scheduler completion
// routing lands with its mechanism in commit 6.
// ---------------------------------------------------------------------------

#[test]
fn shared_io_ring_injection_is_refused_naming_commit_6() {
    // Typed refusal, exact variant (file ring).
    let file_services =
        SchedulerServices::minimal().shared_file_io(SharedIoRing::file(RingConfig::default()));
    assert_eq!(
        file_services.validate(),
        Err(WithServicesError::SharedRingRoutingDeferred {
            service: "file-io-ring"
        })
    );
    // ...and the generic ring.
    let generic_services = SchedulerServices::minimal()
        .shared_generic_io(SharedIoRing::generic(RingConfig::default()));
    assert_eq!(
        generic_services.validate(),
        Err(WithServicesError::SharedRingRoutingDeferred {
            service: "generic-io-ring"
        })
    );

    // with_services itself enforces the refusal (not just the standalone
    // validate): construction fails with a message naming commit 6. (`Scheduler`
    // is not `Debug`, so match rather than `expect_err`.)
    let error = match Scheduler::with_services(one_worker(), file_services, registry()) {
        Ok(_) => panic!("a shared file ring must be refused by with_services"),
        Err(error) => error,
    };
    assert!(
        error.contains("commit 6"),
        "the refusal must name the routing gate / commit 6: {error}"
    );

    // POSITIVE controls: the SAFE compositions validate cleanly — a pin that
    // cannot fail is not a pin.
    assert_eq!(SchedulerServices::minimal().validate(), Ok(()));
    assert_eq!(SchedulerServices::full_runtime().validate(), Ok(()));
    let shared_pool = Arc::new(DirtyPool::new("dirty-io", 1));
    assert_eq!(
        SchedulerServices::minimal()
            .shared_dirty_io(shared_pool)
            .validate(),
        Ok(()),
        "a shared dirty pool is SAFE and must not be refused"
    );
}

// ---------------------------------------------------------------------------
// Replay ⇒ Disabled distribution bundle (pair ruling, commit 5): under replay
// NEITHER distribution runtime is built, even with Some(config).
// ---------------------------------------------------------------------------

#[test]
fn replay_disables_the_distribution_bundle_even_with_some_config() {
    use beamr::replay::ReplayLog;

    let scheduler = Scheduler::new_replay(
        SchedulerConfig {
            distribution: Some(DistributionConfig::default()),
            ..SchedulerConfig::default()
        },
        ReplayLog::default(),
    )
    .expect("replay scheduler starts");

    // NEITHER runtime exists: the bundle is Disabled (no "beamr-net-kernel"
    // worker), not merely sender-less. This is the commit-5 resolution of the
    // commit-4 inconsistency.
    assert_eq!(
        mode_of(&scheduler, "distribution"),
        ServiceModeLabel::Disabled,
        "replay must build NEITHER distribution runtime"
    );
    assert!(
        scheduler.try_distribution_config().is_none(),
        "a replay-disabled bundle exposes no distribution config"
    );
    let distribution_threads: Vec<String> = scheduler
        .service_inventory()
        .into_iter()
        .find(|e| e.service == "distribution")
        .map(|e| e.thread_names)
        .unwrap_or_default();
    assert!(
        distribution_threads.is_empty(),
        "no distribution runtime worker under replay: {distribution_threads:?}"
    );
    scheduler.shutdown();
}

/// A shared-ring handle passed to the WRONG service slot is refused as a
/// KIND MISMATCH naming both sides — never misattributed to the deferred
/// routing refusal of the wrong service (commit-5 round-1 minor).
#[test]
fn crossed_shared_ring_handles_are_refused_as_a_kind_mismatch() {
    let crossed_file =
        SchedulerServices::minimal().shared_file_io(SharedIoRing::generic(RingConfig::default()));
    assert_eq!(
        crossed_file.validate(),
        Err(WithServicesError::SharedRingKindMismatch {
            slot: "file-io-ring",
            handle: "generic-io-ring",
        })
    );
    let crossed_generic =
        SchedulerServices::minimal().shared_generic_io(SharedIoRing::file(RingConfig::default()));
    assert_eq!(
        crossed_generic.validate(),
        Err(WithServicesError::SharedRingKindMismatch {
            slot: "generic-io-ring",
            handle: "file-io-ring",
        })
    );
}
