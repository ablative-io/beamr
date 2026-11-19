//! noconnection: a manual disconnect, and a wire exit followed by the
//! connection going down.

use super::*;

/// T-7 (ruling 11): `disconnect_node` from a plain `std::thread` — no tokio
/// runtime context — runs the ManualDisconnect hook chain inline and delivers
/// noconnection to linked processes without deadlocking.
#[test]
fn manual_disconnect_from_plain_thread_delivers_noconnection() {
    let shared = make_shared_state_with_dist_sender();
    register_scheduler_connection_subscriber(&shared);
    let node = shared.atom_table.intern("peer@test");
    let target = insert_process(&shared, 1);
    set_trap_exit(&shared, target, true);
    let remote = RemotePid {
        node,
        pid_number: 10,
        serial: 0,
    };
    add_remote_link(&shared, target, remote);

    let peer = {
        let handle = shared.dist_sender_or_panic().handle();
        let _context = handle.enter();
        install_peer(&shared, node)
    };

    let shared_for_thread = Arc::clone(&shared);
    let worker = std::thread::spawn(move || {
        assert!(
            shared_for_thread
                .distribution_connections_or_panic()
                .disconnect_node(node),
            "disconnect_node finds the installed connection"
        );
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    while !worker.is_finished() {
        assert!(
            Instant::now() < deadline,
            "disconnect_node deadlocked on a plain thread"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    worker.join().expect("disconnect thread joins");

    // INV-SYNC: every hook effect completed before disconnect_node returned.
    let tuple = read_mailbox_tuple(&shared, target).expect("noconnection EXIT");
    assert_noconnection_exit(&tuple, remote);
    assert!(is_alive(&shared, target), "trapping target survives");
    drop(peer);
}

/// Scenario 6, forward order (in-crate half): the wire EXIT lands first and
/// consumes the link; the node death that follows finds no link left, so the
/// noconnection backstop no-ops — exactly one `{'EXIT', _, _}` (DC-4).
#[test]
fn wire_exit_then_connection_down_delivers_exactly_one_signal() {
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
    assert_eq!(mailbox_message_count(&shared, target), 1);

    supervision_integration::connection_down(&shared, peer_node);

    assert_eq!(
        mailbox_message_count(&shared, target),
        1,
        "the backstop must not re-signal a link the wire EXIT consumed (DC-4)"
    );
    let tuple = read_mailbox_tuple(&shared, target).expect("wire EXIT tuple");
    assert_eq!(tuple[2], Term::atom(Atom::ERROR));
    assert!(is_alive(&shared, target));
}
