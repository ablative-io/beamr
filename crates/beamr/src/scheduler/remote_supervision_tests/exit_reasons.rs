//! Exit reasons across the wire: killed, normal, and messages sent before
//! the exit.

use super::*;

/// Scenario 3, receive half: an inbound link EXIT carrying `killed` (the
/// pre-terminalized form) IS trappable — parity with the local
/// `killed_signal_is_trappable_by_linked_process`.
#[test]
fn wire_killed_reason_is_trappable_by_the_linked_target() {
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
        ExitReason::Killed,
        &shared.atom_table,
    )
    .expect("exit frame encodes");
    assert_eq!(dispatch_wire_frame(&shared, peer_node, &frame), Ok(true));

    let tuple = read_mailbox_tuple(&shared, target).expect("killed EXIT tuple");
    assert_eq!(tuple.len(), 3);
    assert_eq!(tuple[0], Term::atom(Atom::EXIT));
    assert_eq!(
        tuple[2],
        Term::atom(Atom::KILLED),
        "reason is killed, not kill"
    );
    assert!(is_alive(&shared, target), "killed is trappable");
}

/// Scenario 2, in-crate half: a wire EXIT carrying `normal` never kills a
/// non-trapping target (no message either), and a trapping target receives
/// `{'EXIT', _, normal}` — C3 includes Normal for trappers.
#[test]
fn wire_normal_exit_spares_non_trapper_and_traps_as_normal_tuple() {
    let shared = make_shared_state_with_dist_sender();
    let peer_node = shared.atom_table.intern("peer@test");
    let quiet = insert_process(&shared, 1);
    let trapper = insert_process(&shared, 2);
    set_trap_exit(&shared, trapper, true);
    let from = RemotePid {
        node: peer_node,
        pid_number: 42,
        serial: 0,
    };
    add_remote_link(&shared, quiet, from);
    add_remote_link(&shared, trapper, from);

    for target in [quiet, trapper] {
        let frame = encode_exit_frame(
            ControlOp::Exit,
            peer_node,
            42,
            RemotePid {
                node: shared.local_node.name,
                pid_number: target,
                serial: 0,
            },
            ExitReason::Normal,
            &shared.atom_table,
        )
        .expect("exit frame encodes");
        assert_eq!(dispatch_wire_frame(&shared, peer_node, &frame), Ok(true));
    }

    assert!(is_alive(&shared, quiet), "a normal remote exit never kills");
    assert_eq!(mailbox_message_count(&shared, quiet), 0);
    let tuple = read_mailbox_tuple(&shared, trapper).expect("normal EXIT tuple");
    assert_eq!(tuple.len(), 3);
    assert_eq!(tuple[0], Term::atom(Atom::EXIT));
    assert_eq!(tuple[2], Term::atom(Atom::NORMAL));
    assert!(is_alive(&shared, trapper));
}

/// Scenario 1, in-crate half over a real socket: messages the (test-driven)
/// peer sends before its death all precede the EXIT in the target's mailbox —
/// the receive-side face of DC-6 (one socket, frames applied serially) — and
/// the trapping target gets exactly one `{'EXIT', <ext-pid>, error}` 3-tuple.
#[test]
fn presend_messages_precede_wire_exit_and_trapping_target_survives() {
    let shared = make_shared_state_with_dist_sender();
    register_distribution_control_handler(&shared);
    let handle = shared.dist_sender_or_panic().handle();
    let _context = handle.enter();

    let peer_node = shared.atom_table.intern("peer@test");
    let target = insert_process(&shared, 1);
    set_trap_exit(&shared, target, true);
    let from = RemotePid {
        node: peer_node,
        pid_number: 42,
        serial: 0,
    };
    add_remote_link(&shared, target, from);
    let mut peer = install_peer(&shared, peer_node);

    let mut heap = [0_u64; 4];
    let to = write_external_pid(&mut heap, shared.local_node.name, target, 0)
        .expect("external pid fits");
    let count = 8_usize;
    for index in 0..count {
        let frame = encode_send_frame(
            Term::atom(Atom::OK),
            to,
            Term::small_int(index as i64),
            &shared.atom_table,
        )
        .expect("send frame encodes");
        peer.write_all(&frame).expect("send frame writes");
    }
    let exit = encode_exit_frame(
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
    peer.write_all(&exit).expect("exit frame writes");

    let deadline = Instant::now() + Duration::from_secs(30);
    while mailbox_message_count(&shared, target) < count + 1 {
        assert!(
            Instant::now() < deadline,
            "pre-death sends + EXIT never all arrived"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let terms = mailbox_terms(&shared, target);
    assert_eq!(
        terms.len(),
        count + 1,
        "exactly one EXIT after the payloads"
    );
    for (index, term) in terms[..count].iter().enumerate() {
        assert_eq!(
            *term,
            Term::small_int(index as i64),
            "pre-death sends stay ahead of the EXIT and in order"
        );
    }
    let tuple = crate::term::boxed::Tuple::new(terms[count]).expect("EXIT is a tuple");
    assert_eq!(tuple.arity(), 3);
    assert_eq!(tuple.get(0), Some(Term::atom(Atom::EXIT)));
    let source = ExternalPid::new(tuple.get(1).unwrap_or(Term::NIL)).expect("remote source pid");
    assert_eq!(source.node(), Some(peer_node));
    assert_eq!(source.pid_number(), 42);
    assert_eq!(tuple.get(2), Some(Term::atom(Atom::ERROR)));
    assert!(is_alive(&shared, target), "trapping target survives");
}
