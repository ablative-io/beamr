//! Link removal and unlink while the process is executing survive
//! store-back.

use super::*;

/// Store-back reconciliation (DC-4): a wire EXIT(3) consumed by the Executing
/// arm removes the link from `ProcessMetadata`, and the store-back merge must
/// NOT resurrect it from the checked-out `Process` copy — a resurrected entry
/// would let the later node-down backstop double-fire the exit signal for the
/// same link.
#[test]
fn executing_link_removal_survives_store_back_without_double_fire() {
    let shared = make_shared_state();
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

    // Wire EXIT(3) lands while the target is Executing: the DC-4 gate
    // consumes the link in metadata.
    supervision_integration::process_remote_exit_signal(
        &shared,
        from,
        target,
        ExitReason::Error,
        RemoteExitKind::LinkExit,
    );
    store_runnable_process(&shared, process);

    assert!(
        remote_links_of(&shared, target).is_empty(),
        "the consumed link must not be resurrected at store-back"
    );
    assert_eq!(
        mailbox_message_count(&shared, target),
        1,
        "the trapped EXIT materialized at store-back"
    );

    // The connection later drops: the backstop must find nothing — exactly
    // one of {wire EXIT(3), noconnection} per link (DC-4).
    supervision_integration::connection_down(&shared, peer_node);
    assert_eq!(
        mailbox_message_count(&shared, target),
        1,
        "no second signal for the same link after the node-down backstop"
    );
}

/// Store-back reconciliation, `unlink/1` walk: a process calling unlink on a
/// remote pid is by construction Executing during its own BIF, so the removal
/// lands in `ProcessMetadata`. If store-back resurrected it, the local
/// half-link would persist forever and the next node-down would deliver a
/// spurious noconnection kill to the process that unlinked.
#[test]
fn unlink_while_executing_survives_store_back_without_spurious_noconnection() {
    let shared = make_shared_state();
    let peer_node = shared.atom_table.intern("peer@test");
    let target = insert_process(&shared, 1);
    let from = RemotePid {
        node: peer_node,
        pid_number: 42,
        serial: 0,
    };
    add_remote_link(&shared, target, from);
    let process = make_executing(&shared, target);

    assert!(remove_remote_link(&shared, target, from));
    store_runnable_process(&shared, process);
    assert!(
        remote_links_of(&shared, target).is_empty(),
        "the unlinked half must stay removed after store-back"
    );

    supervision_integration::connection_down(&shared, peer_node);
    assert!(
        is_alive(&shared, target),
        "a node-down after unlink must not kill the non-trapping ex-linker"
    );
}
