//! Inbound LINK controls: hostile frames, idempotent duplicates, dead
//! targets, and links arriving after a write-side down.

use super::*;

/// The full inbound wiring, plus the scenario-9 read-loop-survival half: a
/// hostile (undecodable) frame arriving first must be dropped without killing
/// the read loop, and a subsequent wire LINK from the peer must still
/// establish a remote link on the local target through
/// `register_control_frame_handler_with_origin` → `dispatch_frame` →
/// `apply_inbound_link`.
#[test]
fn inbound_link_over_real_socket_survives_hostile_frame_and_establishes_link() {
    let shared = make_shared_state_with_dist_sender();
    register_distribution_control_handler(&shared);
    let handle = shared.dist_sender_or_panic().handle();
    let _context = handle.enter();

    let peer_node = shared.atom_table.intern("peer@test");
    let target = insert_process(&shared, 1);
    let mut peer = install_peer(&shared, peer_node);

    // Well-framed garbage: a 4-byte "control" that is not ETF. The handler
    // must swallow the decode error (telemetry-counted drop), not panic the
    // read loop.
    let mut garbage = Vec::new();
    garbage.extend_from_slice(&4_u32.to_be_bytes());
    garbage.extend_from_slice(&0_u32.to_be_bytes());
    garbage.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    peer.write_all(&garbage).expect("garbage frame writes");

    let expected = RemotePid {
        node: peer_node,
        pid_number: 42,
        serial: 0,
    };
    let frame = encode_link_frame(
        peer_node,
        42,
        RemotePid {
            node: shared.local_node.name,
            pid_number: target,
            serial: 0,
        },
        &shared.atom_table,
    )
    .expect("link frame encodes");
    peer.write_all(&frame).expect("link frame writes");

    let deadline = Instant::now() + Duration::from_secs(10);
    while remote_links_of(&shared, target).is_empty() {
        assert!(
            Instant::now() < deadline,
            "inbound LINK never applied — read loop dead or sink unwired"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(remote_links_of(&shared, target), vec![expected]);
}

/// T-4 + T-5: a duplicate inbound LINK is idempotent success — one link
/// entry, NO noproc reply (ruling 2) — while a LINK to a dead pid answers
/// EXIT(noproc), and a LINK to a tombstoned pid answers the real terminal
/// reason. The control lane is FIFO, so the first frame on the peer socket
/// being the noproc reply proves the duplicates produced nothing.
#[test]
fn duplicate_link_is_idempotent_and_link_to_dead_pid_replies_exit() {
    let shared = make_shared_state_with_dist_sender();
    let handle = shared.dist_sender_or_panic().handle();
    let _context = handle.enter();

    let peer_node = shared.atom_table.intern("peer@test");
    let local = shared.local_node.name;
    let live = insert_process(&shared, 1);
    let mut peer = install_peer(&shared, peer_node);
    let from = RemotePid {
        node: peer_node,
        pid_number: 42,
        serial: 0,
    };

    // Duplicate LINK: one entry, no reply.
    apply_inbound_link(&shared, from, live);
    apply_inbound_link(&shared, from, live);
    assert_eq!(remote_links_of(&shared, live), vec![from]);

    // LINK to a never-existing pid: EXIT(noproc) reply.
    apply_inbound_link(&shared, from, 999);

    // LINK to a tombstoned pid: EXIT with the real terminal reason.
    let dead = insert_process(&shared, 2);
    cleanup_exited_process(&shared, dead, ExitReason::Error);
    apply_inbound_link(&shared, from, dead);

    let (control, _payload) = read_peer_frame(&mut peer);
    assert_eq!(
        decode_control(&control, &shared.atom_table),
        Ok(ControlMessage::Exit {
            from: RemotePid {
                node: local,
                pid_number: 999,
                serial: 0,
            },
            to_pid: 42,
            reason: ExitReason::NoProc,
        }),
        "first frame must be the noproc reply — the duplicate LINKs replied nothing"
    );
    let (control, _payload) = read_peer_frame(&mut peer);
    assert_eq!(
        decode_control(&control, &shared.atom_table),
        Ok(ControlMessage::Exit {
            from: RemotePid {
                node: local,
                pid_number: dead,
                serial: 0,
            },
            to_pid: 42,
            reason: ExitReason::Error,
        }),
        "a tombstoned target replies its real terminal reason"
    );
}

/// `establish_remote_link` contract after the ruling-2/3 fix: `false` means
/// dead or absent ONLY — a live duplicate is `true`.
#[test]
fn establish_remote_link_returns_false_only_for_dead_or_absent_targets() {
    let shared = make_shared_state();
    let live = insert_process(&shared, 1);
    let remote = RemotePid {
        node: Atom::OK,
        pid_number: 42,
        serial: 0,
    };

    assert!(establish_remote_link(&shared, live, remote));
    assert!(
        establish_remote_link(&shared, live, remote),
        "duplicate link is idempotent success"
    );
    assert_eq!(remote_links_of(&shared, live), vec![remote]);

    // Exited-but-Present target.
    let exited = insert_process(&shared, 2);
    {
        let entry = shared.process_bodies.get(&exited).expect("process exists");
        let mut slot = lock_or_recover(&entry);
        let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
            panic!("process is present");
        };
        process.terminate(ExitReason::Error);
    }
    assert!(!establish_remote_link(&shared, exited, remote));

    // Absent target.
    assert!(!establish_remote_link(&shared, 999, remote));
}

/// Scenario 6, reverse order (in-crate half): the noconnection backstop runs
/// first and consumes the link; a late wire EXIT(3) for the same link then
/// hits the DC-4 LinkExit gate and no-ops — exactly one `{'EXIT', _, _}`.
#[test]
fn backstop_then_late_wire_exit_delivers_exactly_one_signal() {
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

    supervision_integration::connection_down(&shared, peer_node);
    assert_eq!(mailbox_message_count(&shared, target), 1);

    // The late wire EXIT(3) — the frame that raced the node death.
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

    assert_eq!(
        mailbox_message_count(&shared, target),
        1,
        "the LinkExit gate must no-op once the backstop consumed the link"
    );
    let tuple = read_mailbox_tuple(&shared, target).expect("noconnection EXIT");
    assert_noconnection_exit(&tuple, from);
    assert!(is_alive(&shared, target));
}

/// Inbound LINK racing a write-side down: a down initiated outside the read
/// loop (write timeout, control-lane overflow, heartbeat deadline,
/// `disconnect_node`) runs the backstop scan on the marking thread with no
/// ordering against a concurrent `apply_inbound_link`. When the link is
/// established after the scan already completed (modelled here by applying
/// with no connection installed at all), the post-establish recheck must
/// deliver the noconnection the backstop missed instead of leaking a link no
/// future down event will ever sever.
#[test]
fn inbound_link_after_a_write_side_down_is_noconnectioned_not_leaked() {
    let shared = make_shared_state();
    let peer_node = shared.atom_table.intern("peer@test");
    let target = insert_process(&shared, 1);
    set_trap_exit(&shared, target, true);
    let from = RemotePid {
        node: peer_node,
        pid_number: 42,
        serial: 0,
    };

    apply_inbound_link(&shared, from, target);

    assert!(
        remote_links_of(&shared, target).is_empty(),
        "the link must not outlive the dead session it arrived on"
    );
    let tuple = read_mailbox_tuple(&shared, target).expect("noconnection EXIT");
    assert_noconnection_exit(&tuple, from);
    assert!(is_alive(&shared, target), "trapping target survives");
}
