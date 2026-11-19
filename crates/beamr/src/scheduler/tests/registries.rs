//! ETS tables and process-group membership held through the scheduler.

use super::*;

fn ets_metadata(name: Option<Atom>, owner: u64) -> EtsTableMetadata {
    EtsTableMetadata::new(name, 0, EtsTableType::Set, Protection::Protected, owner)
}

#[test]
fn cleanup_exited_process_purges_pg_membership_without_connection() {
    // A dying process must drop its pg membership locally even when no
    // distribution connection exists — the local purge runs synchronously inside
    // `cleanup_exited_process`, and the propagated leave is a no-op (no peers).
    let scheduler = Scheduler::new(
        SchedulerConfig::default(),
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .expect("scheduler should start");
    let registry = scheduler.pg_registry();
    let scope = registry.default_scope();
    let group = scheduler.shared.atom_table.intern("exiting_workers");
    // A real table entry: finalization is exactly-once, keyed on the atomic
    // process-table removal — a pid the table never knew is skipped.
    let pid = super::supervision_tests::insert_process(&scheduler.shared, 555);

    registry.join(scope, group, pid);
    assert_eq!(
        registry.local_members(scope, group),
        vec![pid],
        "pid should be a local member before exit"
    );

    cleanup_exited_process(&scheduler.shared, pid, ExitReason::Normal);

    assert!(
        registry.local_members(scope, group).is_empty(),
        "cleanup_exited_process must purge the pid's pg membership locally"
    );
    scheduler.shutdown();
}

#[test]
fn ets_registry_create_lookup_name_and_delete() {
    let scheduler = Scheduler::new(
        SchedulerConfig::default(),
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .expect("scheduler should start");
    let name = scheduler.shared.atom_table.intern("named_ets_table");

    let first_id = scheduler.shared.create_table(ets_metadata(Some(name), 99));
    let second_id = scheduler.shared.create_table(ets_metadata(None, 99));

    assert_ne!(first_id, second_id);
    assert!(second_id > first_id);
    assert_eq!(scheduler.shared.lookup_table_by_name(name), Some(first_id));

    let table = scheduler
        .shared
        .lookup_table(first_id)
        .expect("table should be present by id");
    assert_eq!(table.metadata().id, first_id);
    assert_eq!(table.metadata().name, Some(name));

    assert!(scheduler.shared.delete_table(first_id));
    assert!(scheduler.shared.lookup_table(first_id).is_none());
    assert_eq!(scheduler.shared.lookup_table_by_name(name), None);
    assert!(scheduler.shared.lookup_table(second_id).is_some());
    assert!(!scheduler.shared.delete_table(first_id));

    scheduler.shutdown();
}
