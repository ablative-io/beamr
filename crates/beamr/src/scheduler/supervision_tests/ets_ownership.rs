//! ETS tables an exiting process owns are deleted or given to a live heir.

use super::*;

fn ets_metadata(name: Option<Atom>, owner: u64) -> EtsTableMetadata {
    EtsTableMetadata::new(name, 0, EtsTableType::Set, Protection::Protected, owner)
}

fn ets_metadata_with_heir(
    name: Option<Atom>,
    owner: u64,
    heir_pid: u64,
    data: Term,
) -> EtsTableMetadata {
    let mut metadata = ets_metadata(name, owner);
    metadata.heir = Some(crate::ets::EtsHeir {
        pid: heir_pid,
        data: crate::ets::copy_term_to_ets(data).expect("heir data copies into ETS"),
    });
    metadata
}

#[test]
fn cleanup_exited_process_deletes_tables_owned_by_process() {
    let shared = make_shared_state();
    let owner = insert_process(&shared, 11);
    let survivor = insert_process(&shared, 12);
    let owned_name = shared.atom_table.intern("owned_table");
    let owned_id = shared.create_table(ets_metadata(Some(owned_name), owner));
    let other_id = shared.create_table(ets_metadata(None, survivor));

    cleanup_exited_process(&shared, owner, ExitReason::Normal);

    assert!(shared.lookup_table(owned_id).is_none());
    assert_eq!(shared.lookup_table_by_name(owned_name), None);
    assert!(shared.lookup_table(other_id).is_some());
}

#[test]
fn cleanup_exited_process_transfers_table_to_live_heir() {
    let shared = make_shared_state();
    let owner = insert_process(&shared, 11);
    let heir = insert_process(&shared, 12);
    let transfer_atom = shared.atom_table.intern("ETS-TRANSFER");
    let data = Term::small_int(77);
    let owned_id = shared.create_table(ets_metadata_with_heir(None, owner, heir, data));

    cleanup_exited_process(&shared, owner, ExitReason::Normal);

    let table = shared.lookup_table(owned_id).expect("table transferred");
    assert_eq!(table.metadata().owner.get(), heir);
    assert_eq!(
        table.check_access(heir, crate::ets::AccessOp::Write),
        Ok(())
    );
    let message = read_mailbox_tuple(&shared, heir).expect("ETS-TRANSFER delivered");
    assert_eq!(message.len(), 4);
    assert_eq!(message[0], Term::atom(transfer_atom));
    assert_eq!(message[1], Term::small_int(owned_id as i64));
    assert_eq!(message[2], Term::pid(owner));
    assert_eq!(message[3], data);
}

#[test]
fn cleanup_exited_process_transfers_table_to_executing_heir() {
    let shared = make_shared_state();
    let owner = insert_process(&shared, 11);
    let heir = insert_process(&shared, 12);
    let transfer_atom = shared.atom_table.intern("ETS-TRANSFER");
    let data = Term::small_int(77);
    let owned_id = shared.create_table(ets_metadata_with_heir(None, owner, heir, data));
    let executing_heir = make_executing(&shared, heir);

    cleanup_exited_process(&shared, owner, ExitReason::Normal);

    let table = shared.lookup_table(owned_id).expect("table transferred");
    assert_eq!(table.metadata().owner.get(), heir);
    assert!(
        read_mailbox_tuple(&shared, heir).is_none(),
        "executing heir receives message after store-back"
    );

    store_runnable_process(&shared, executing_heir);
    let message = read_mailbox_tuple(&shared, heir).expect("ETS-TRANSFER delivered");
    assert_eq!(message.len(), 4);
    assert_eq!(message[0], Term::atom(transfer_atom));
    assert_eq!(message[1], Term::small_int(owned_id as i64));
    assert_eq!(message[2], Term::pid(owner));
    assert_eq!(message[3], data);
}

#[test]
fn cleanup_exited_process_deletes_table_when_heir_is_not_live() {
    let shared = make_shared_state();
    let owner = insert_process(&shared, 11);
    let owned_id =
        shared.create_table(ets_metadata_with_heir(None, owner, 99, Term::small_int(77)));

    cleanup_exited_process(&shared, owner, ExitReason::Normal);

    assert!(shared.lookup_table(owned_id).is_none());
}
