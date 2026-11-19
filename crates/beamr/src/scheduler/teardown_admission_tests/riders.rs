//! The commit-6 riders admit before the drain and refuse after it.

use super::*;

#[test]
fn all_five_commit6_riders_admit_before_drain_and_refuse_after() {
    let scheduler = Scheduler::with_services(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        SchedulerServices::minimal().owned_readiness(),
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("rider scheduler starts: {error}"));
    for pid in [101, 102, 103] {
        add_executing_process(&scheduler, pid);
    }
    let services =
        supervision_integration::build_native_services(&scheduler.shared, NamespaceId::DEFAULT);
    let group = services
        .group_leader_facility
        .as_ref()
        .unwrap_or_else(|| panic!("group-leader facility"));
    let supervision = services
        .supervision_facility
        .as_ref()
        .unwrap_or_else(|| panic!("supervision facility"));
    let ets = services
        .ets_facility
        .as_ref()
        .unwrap_or_else(|| panic!("ETS facility"));

    // Each mutation succeeds while admission is open.
    group
        .set_group_leader(101, Term::pid(101))
        .unwrap_or_else(|error| panic!("pre-drain group leader: {error}"));
    let table = ets
        .create_table(EtsTableMetadata::new(
            None,
            0,
            EtsTableType::Set,
            Protection::Protected,
            101,
        ))
        .unwrap_or_else(|error| panic!("pre-drain ETS create: {error}"));
    let mut timer_context =
        ProcessContext::with_timer_services(101, Arc::clone(&scheduler.shared.timers));
    timer_context.set_teardown_admission_facility(services.teardown_admission_facility.clone());
    assert!(
        timer_context
            .schedule_timer(Duration::from_secs(60), 101, Term::atom(Atom::new(450)))
            .is_some()
    );
    let (reader, _writer) =
        UnixStream::pair().unwrap_or_else(|error| panic!("socket pair: {error}"));
    let token = scheduler
        .shared
        .readiness_register(reader.as_raw_fd(), Interest::READABLE, 101, Atom::new(451))
        .unwrap_or_else(|error| panic!("pre-drain readiness register: {error}"));
    supervision
        .exit_signal(101, 102, ExitReason::Kill)
        .unwrap_or_else(|error| panic!("pre-drain exit signal: {error}"));

    scheduler.shared.drain_dirty_completions();

    // Every named row now refuses through its existing typed surface.
    assert_eq!(
        group.set_group_leader(101, Term::pid(101)),
        Err(GroupLeaderError::NoProc)
    );
    assert_eq!(
        supervision.exit_signal(101, 103, ExitReason::Kill),
        Err(SupervisionError::NoProc)
    );
    assert_eq!(
        ets.create_table(EtsTableMetadata::new(
            None,
            0,
            EtsTableType::Set,
            Protection::Protected,
            101,
        )),
        Err(EtsError::Badarg)
    );
    assert!(!ets.delete_table(table));
    assert!(
        timer_context
            .schedule_timer(Duration::from_secs(60), 101, Term::atom(Atom::new(452)))
            .is_none()
    );
    let (straggler, _peer) =
        UnixStream::pair().unwrap_or_else(|error| panic!("straggler pair: {error}"));
    assert_eq!(
        scheduler.shared.readiness_register(
            straggler.as_raw_fd(),
            Interest::READABLE,
            101,
            Atom::new(453),
        ),
        Err(ReadinessError::TeardownInProgress)
    );

    scheduler.readiness_deregister(token);
    scheduler.shutdown();
}
