//! Controls and sends that must not apply: misaddressed or forged frames,
//! embedder serials, and pids beyond the wire's u32 range.

use super::*;

/// T-6 (R6 fix): a SEND whose to-pid is an external pid naming another node
/// is dropped as misaddressed — the local pid with the same number is
/// untouched — while a node-less to-pid still delivers (legacy tolerance).
#[test]
fn misaddressed_send_is_dropped_but_node_less_send_delivers() {
    let shared = make_shared_state_with_dist_sender();
    let peer_node = shared.atom_table.intern("peer@test");
    let third_node = shared.atom_table.intern("third@test");
    let target = insert_process(&shared, 7);

    let mut heap = [0_u64; 4];
    let to = write_external_pid(&mut heap, third_node, target, 0).expect("external pid fits");
    let frame = encode_send_frame(
        Term::atom(Atom::OK),
        to,
        Term::atom(Atom::OK),
        &shared.atom_table,
    )
    .expect("send frame encodes");
    assert_eq!(
        dispatch_wire_frame(&shared, peer_node, &frame),
        Err(ControlError::MisAddressed)
    );
    assert_eq!(
        mailbox_message_count(&shared, target),
        0,
        "a frame addressed to third@test must not deliver locally"
    );

    let frame = encode_send_frame(
        Term::atom(Atom::OK),
        Term::pid(target),
        Term::atom(Atom::OK),
        &shared.atom_table,
    )
    .expect("send frame encodes");
    assert_eq!(dispatch_wire_frame(&shared, peer_node, &frame), Ok(true));
    assert_eq!(mailbox_message_count(&shared, target), 1);
}

/// Scenario 9 (dispatch half): a link control whose `from` forges a node
/// other than the authenticated origin is dropped (`Ok(false)`) with no state
/// change; a link control addressed to a third node is `MisAddressed`.
#[test]
fn forged_origin_and_misaddressed_link_controls_do_not_apply() {
    let shared = make_shared_state_with_dist_sender();
    let peer_node = shared.atom_table.intern("peer@test");
    let third_node = shared.atom_table.intern("third@test");
    let target = insert_process(&shared, 1);

    // `from` says third@test but the frame arrived on peer@test's connection.
    let forged = encode_link_frame(
        third_node,
        42,
        RemotePid {
            node: shared.local_node.name,
            pid_number: target,
            serial: 0,
        },
        &shared.atom_table,
    )
    .expect("link frame encodes");
    assert_eq!(dispatch_wire_frame(&shared, peer_node, &forged), Ok(false));
    assert!(
        remote_links_of(&shared, target).is_empty(),
        "a forged LINK must not establish anything"
    );

    // `to` names a third node: misaddressed.
    let misaddressed = encode_link_frame(
        peer_node,
        42,
        RemotePid {
            node: third_node,
            pid_number: target,
            serial: 0,
        },
        &shared.atom_table,
    )
    .expect("link frame encodes");
    assert_eq!(
        dispatch_wire_frame(&shared, peer_node, &misaddressed),
        Err(ControlError::MisAddressed)
    );
    assert!(remote_links_of(&shared, target).is_empty());
}

/// A nonzero embedder-supplied `RemotePid.serial` is normalized to the wire
/// link identity (serial 0) at establishment, so the peer's later EXIT —
/// whose `from` is always minted with serial 0 — hits the DC-4 equality gate
/// instead of silently no-oping and losing the death signal until node-down.
#[test]
fn nonzero_embedder_serial_cannot_dodge_the_wire_exit_gate() {
    let shared = make_shared_state_with_dist_sender();
    let handle = shared.dist_sender_or_panic().handle();
    let _context = handle.enter();

    let peer_node = shared.atom_table.intern("peer@test");
    let caller = insert_process(&shared, 1);
    set_trap_exit(&shared, caller, true);
    let peer = install_peer(&shared, peer_node);
    let facility = control_facility(&shared);

    assert_eq!(
        facility.link_remote(
            caller,
            RemotePid {
                node: peer_node,
                pid_number: 7,
                serial: 3,
            }
        ),
        Ok(())
    );

    // The peer's pid 7 dies; its wire EXIT mints from = (peer, 7, serial 0).
    let frame = encode_exit_frame(
        ControlOp::Exit,
        peer_node,
        7,
        RemotePid {
            node: shared.local_node.name,
            pid_number: caller,
            serial: 0,
        },
        ExitReason::Error,
        &shared.atom_table,
    )
    .expect("exit frame encodes");
    assert_eq!(dispatch_wire_frame(&shared, peer_node, &frame), Ok(true));

    assert!(
        remote_links_of(&shared, caller).is_empty(),
        "the serial-0 wire EXIT must sever the stored link"
    );
    assert_eq!(
        mailbox_message_count(&shared, caller),
        1,
        "the death signal was delivered, not gated out"
    );
    drop(peer);
}

/// Endpoint pids beyond the wire's NEW_PID_EXT u32 fields are refused at the
/// send boundary: `link_remote` maps them to `BadTarget` (with the half-link
/// unwound) and the fire-and-forget senders drop, instead of the encode
/// failure tearing the whole pinned connection down per control — the churn
/// loop a long-lived node would enter once its pid counter passes 2^32.
#[test]
fn pids_beyond_the_wire_u32_range_are_refused_not_a_connection_teardown() {
    let shared = make_shared_state_with_dist_sender();
    let handle = shared.dist_sender_or_panic().handle();
    let _context = handle.enter();

    let peer_node = shared.atom_table.intern("peer@test");
    let caller = insert_process(&shared, 1);
    let peer = install_peer(&shared, peer_node);
    let facility = control_facility(&shared);
    let oversized = RemotePid {
        node: peer_node,
        pid_number: u64::from(u32::MAX) + 1,
        serial: 0,
    };

    assert_eq!(
        facility.link_remote(caller, oversized),
        Err(RemoteLinkError::BadTarget),
        "an unencodable target pid is BadTarget"
    );
    assert!(
        remote_links_of(&shared, caller).is_empty(),
        "no immortal half-link is left behind"
    );
    assert_eq!(
        facility.exit_remote(caller, oversized, ExitReason::Error),
        Ok(()),
        "EXIT2 stays best-effort Ok"
    );
    assert_eq!(facility.unlink_remote(caller, oversized), Ok(()));

    let connection = shared
        .distribution_connections_or_panic()
        .get_connection(peer_node)
        .expect("the connection must survive the refused controls");
    assert!(
        !connection.is_down(),
        "an unencodable pid is refused at the send boundary, not converted \
         into a whole-connection teardown"
    );
    drop(peer);
}
