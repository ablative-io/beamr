//! Wire EXIT controls to executing targets, kills, and cascades that send an
//! EXIT for a remote link.

use super::*;

/// Scenario 8, trapping Executing target over a real wire frame: the EXIT(3)
/// parks in `pending_exit_messages` (source `Remote`) and materializes as
/// `{'EXIT', ExternalPid, error}` at store-back.
#[test]
fn wire_exit_to_trapping_executing_target_is_delivered_at_store_back() {
    let shared = make_shared_state_with_dist_sender();
    let peer_node = shared.atom_table.intern("peer@test");
    let target = insert_process(&shared, 1);
    set_trap_exit(&shared, target, true);
    let from = RemotePid {
        node: peer_node,
        pid_number: 42,
        serial: 0,
    };
    add_remote_link(&shared, target, from);
    let process = make_executing(&shared, target);

    let frame = encode_exit_frame(
        ControlOp::Exit,
        peer_node,
        42,
        RemotePid {
            node: shared.local_node.name,
            pid_number: target,
            serial: 0,
        },
        ExitReason::Error,
        &shared.atom_table,
    )
    .expect("exit frame encodes");
    assert_eq!(dispatch_wire_frame(&shared, peer_node, &frame), Ok(true));

    assert!(
        read_mailbox_tuple(&shared, target).is_none(),
        "while Executing, the signal is parked in metadata, not the mailbox"
    );

    store_runnable_process(&shared, process);

    let tuple = read_mailbox_tuple(&shared, target).expect("EXIT tuple after store-back");
    assert_eq!(tuple.len(), 3);
    assert_eq!(tuple[0], Term::atom(Atom::EXIT));
    let source = ExternalPid::new(tuple[1]).expect("remote source pid");
    assert_eq!(source.node(), Some(peer_node));
    assert_eq!(source.pid_number(), 42);
    assert_eq!(tuple[2], Term::atom(Atom::ERROR));
    assert!(is_alive(&shared, target), "trapping target survives");
}

/// Scenario 8, non-trapping Executing target (the D9 alignment): the wire
/// EXIT(3) defers death through `shared_exit_tombstone` — the ETS-heir
/// transfer happens at tombstone time — and store-back runs the FULL cleanup
/// cascade (local links included), not a bare table removal.
#[test]
fn wire_exit_to_non_trapping_executing_target_tombstones_with_full_cascade() {
    let shared = make_shared_state_with_dist_sender();
    let peer_node = shared.atom_table.intern("peer@test");
    let target = insert_process(&shared, 1);
    let heir = insert_process(&shared, 2);
    let linked = insert_process(&shared, 3);
    add_local_link(&shared, target, linked);
    let from = RemotePid {
        node: peer_node,
        pid_number: 42,
        serial: 0,
    };
    add_remote_link(&shared, target, from);

    let mut metadata =
        EtsTableMetadata::new(None, 0, EtsTableType::Set, Protection::Protected, target);
    metadata.heir = Some(crate::ets::EtsHeir {
        pid: heir,
        data: crate::ets::copy_term_to_ets(Term::NIL).expect("heir data copies"),
    });
    let table_id = shared
        .ets_registry
        .try_create_table(metadata)
        .expect("ets table creates");

    let process = make_executing(&shared, target);

    let frame = encode_exit_frame(
        ControlOp::Exit,
        peer_node,
        42,
        RemotePid {
            node: shared.local_node.name,
            pid_number: target,
            serial: 0,
        },
        ExitReason::Error,
        &shared.atom_table,
    )
    .expect("exit frame encodes");
    assert_eq!(dispatch_wire_frame(&shared, peer_node, &frame), Ok(true));

    // Death is deferred, but the FULL tombstone already ran: the heir owns
    // the table and got its ETS-TRANSFER message (D9 fix — the old arm
    // inserted a bare tombstone and skipped this).
    assert!(is_alive(&shared, target), "death deferred while Executing");
    assert_eq!(shared.exit_tombstones.get(&target), Some(ExitReason::Error));
    let owner = shared
        .ets_registry
        .lookup_table(table_id)
        .expect("table survives via heir")
        .metadata()
        .owner
        .get();
    assert_eq!(owner, heir, "ETS table heired at tombstone time");
    assert_eq!(
        mailbox_message_count(&shared, heir),
        1,
        "heir received ETS-TRANSFER"
    );

    store_runnable_process(&shared, process);
    assert!(
        cleanup_if_tombstoned_after_store(&shared, target),
        "the tombstone must be observed at store-back"
    );
    assert!(!is_alive(&shared, target), "target dead after store-back");
    assert!(
        !is_alive(&shared, linked),
        "local link cascaded at store-back cleanup"
    );
}

/// D9 regression: an inbound Kill (EXIT2, `Direct`) to a trapping Executing
/// target must die `killed`, untrapped — should-die is computed BEFORE the
/// trap check and Kill clears `trap_exit`.
#[test]
fn d9_kill_to_trapping_executing_target_dies_killed_untrapped() {
    let shared = make_shared_state();
    let target = insert_process(&shared, 1);
    set_trap_exit(&shared, target, true);
    let from = RemotePid {
        node: Atom::OK,
        pid_number: 42,
        serial: 0,
    };
    let process = make_executing(&shared, target);

    supervision_integration::process_remote_exit_signal(
        &shared,
        from,
        target,
        ExitReason::Kill,
        RemoteExitKind::Direct,
    );

    assert_eq!(
        shared.exit_tombstones.get(&target),
        Some(ExitReason::Killed),
        "Kill tombstones as killed immediately"
    );

    store_runnable_process(&shared, process);
    assert!(
        read_mailbox_tuple(&shared, target).is_none(),
        "the Kill must NOT be trapped into a {{'EXIT', _, _}} message"
    );
    assert!(cleanup_if_tombstoned_after_store(&shared, target));
    assert!(!is_alive(&shared, target), "target dies killed");
    assert!(shared.process_bodies.get(&target).is_none());
}

/// Ruling 1 / DC-4 kind-gating on a Present target: `Direct` (EXIT2) never
/// touches link state, and the remaining link is consumed by exactly one
/// later `LinkExit` — a second one no-ops.
#[test]
fn direct_exit_leaves_links_alone_and_link_exit_consumes_exactly_once() {
    let shared = make_shared_state();
    let target = insert_process(&shared, 1);
    set_trap_exit(&shared, target, true);
    let from = RemotePid {
        node: Atom::OK,
        pid_number: 42,
        serial: 0,
    };
    add_remote_link(&shared, target, from);

    supervision_integration::process_remote_exit_signal(
        &shared,
        from,
        target,
        ExitReason::Error,
        RemoteExitKind::Direct,
    );
    assert_eq!(mailbox_message_count(&shared, target), 1);
    assert_eq!(
        remote_links_of(&shared, target),
        vec![from],
        "Direct never touches links"
    );

    supervision_integration::process_remote_exit_signal(
        &shared,
        from,
        target,
        ExitReason::Error,
        RemoteExitKind::LinkExit,
    );
    assert_eq!(mailbox_message_count(&shared, target), 2);
    assert!(remote_links_of(&shared, target).is_empty());

    supervision_integration::process_remote_exit_signal(
        &shared,
        from,
        target,
        ExitReason::Error,
        RemoteExitKind::LinkExit,
    );
    assert_eq!(
        mailbox_message_count(&shared, target),
        2,
        "no link left — a second LinkExit is a DC-4 no-op"
    );
    assert!(is_alive(&shared, target));
}

/// A LOCAL link-cascade kill of a process that owns a remote link must send
/// that remote half exactly one wire EXIT (pre-terminalized), and the
/// cascaded process must be fully finalized — the cascade arm once dropped
/// remote links silently with the body (found by the wait-set-residue
/// review; this is its regression pin).
#[test]
fn cascade_killed_process_sends_exit_for_its_remote_link_and_is_finalized() {
    let shared = make_shared_state_with_dist_sender();
    let handle = shared.dist_sender_or_panic().handle();
    let _context = handle.enter();

    let peer_node = shared.atom_table.intern("peer@test");
    let dying = insert_process(&shared, 1);
    let cascaded = insert_process(&shared, 2);
    add_local_link(&shared, dying, cascaded);
    let remote = RemotePid {
        node: peer_node,
        pid_number: 77,
        serial: 0,
    };
    add_remote_link(&shared, cascaded, remote);
    let mut peer = install_peer(&shared, peer_node);

    // Direct kill of `dying` cascades through the local link into `cascaded`,
    // whose remote half-link must leave the node as a wire EXIT.
    cleanup_exited_process(&shared, dying, ExitReason::Kill);

    let (control, _payload) = read_peer_frame(&mut peer);
    assert_eq!(
        decode_control(&control, &shared.atom_table),
        Ok(ControlMessage::Exit {
            from: RemotePid {
                node: shared.local_node.name,
                pid_number: cascaded,
                serial: 0,
            },
            to_pid: remote.pid_number,
            reason: ExitReason::Killed,
        }),
        "the cascaded process's remote link gets its EXIT"
    );
    // And the cascade finalized fully: no body, no table entry, tombstone set.
    assert!(!shared.process_bodies.contains_key(&cascaded));
    assert!(shared.process_table.get(cascaded).is_none());
    assert!(shared.exit_tombstones.contains_key(&cascaded));
}
