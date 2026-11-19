//! Remote links: EXIT controls to remote peers, remote exits to a trapping
//! process, and noconnection when a node goes down.

use super::*;

/// A remote-linked process dying puts a real EXIT(3) control on the wire:
/// `cleanup_exited_process` → `propagate_exit` → `send_remote_exit` →
/// `dist_control_out::send_exit_linked` → control lane → the peer socket,
/// where the frame decodes as `ControlMessage::Exit` with `from` = the local
/// node's serial-0 pid, `to` = the linked remote endpoint, and the terminal
/// reason. (Formerly asserted against the never-drained `ControlRouter`
/// buffer, retired with the wire path.)
#[test]
fn remote_link_exit_sends_exit_control() {
    let shared = make_shared_state_with_dist_sender();
    let handle = shared.dist_sender_or_panic().handle();
    let _context = handle.enter();

    let peer_node = shared.atom_table.intern("peer@test");
    let pid = insert_process(&shared, 1);
    let remote = RemotePid {
        node: peer_node,
        pid_number: 42,
        serial: 7,
    };
    add_remote_link(&shared, pid, remote);
    let mut peer = super::connection_lifecycle_tests::install_peer(&shared, peer_node);

    cleanup_exited_process(&shared, pid, ExitReason::Error);

    let (control, payload) = super::remote_supervision_tests::read_peer_frame(&mut peer);
    assert_eq!(
        crate::distribution::control::decode_control(&control, &shared.atom_table),
        Ok(crate::distribution::control::ControlMessage::Exit {
            from: RemotePid {
                node: shared.local_node.name,
                pid_number: pid,
                serial: 0,
            },
            to_pid: remote.pid_number,
            reason: ExitReason::Error,
        })
    );
    assert_eq!(payload, vec![131, 106], "link controls carry payload = NIL");
}

#[test]
fn remote_exit_to_trapping_process_enqueues_remote_exit_tuple() {
    let shared = make_shared_state();
    let target = insert_process(&shared, 1);
    let remote = RemotePid {
        node: Atom::OK,
        pid_number: 42,
        serial: 7,
    };
    add_remote_link(&shared, target, remote);
    set_trap_exit(&shared, target, true);

    supervision_integration::process_remote_exit_signal(
        &shared,
        remote,
        target,
        ExitReason::Error,
        remote_supervision::RemoteExitKind::LinkExit,
    );

    let tuple = read_mailbox_tuple(&shared, target).expect("remote EXIT message");
    assert_eq!(tuple.len(), 3);
    assert_eq!(tuple[0], Term::atom(Atom::EXIT));
    let source = crate::term::boxed::ExternalPid::new(tuple[1]).expect("remote source pid");
    assert_eq!(source.node(), Some(remote.node));
    assert_eq!(source.pid_number(), remote.pid_number);
    assert_eq!(source.serial(), remote.serial);
    assert_eq!(tuple[2], Term::atom(Atom::ERROR));
    assert!(is_alive(&shared, target));
}

#[test]
fn connection_down_delivers_noconnection_to_all_remote_links_on_node() {
    let shared = make_shared_state();
    let first = insert_process(&shared, 1);
    let second = insert_process(&shared, 2);
    let other_node = insert_process(&shared, 3);
    let node = Atom::OK;
    let remote_a = RemotePid {
        node,
        pid_number: 10,
        serial: 0,
    };
    let remote_b = RemotePid {
        node,
        pid_number: 11,
        serial: 0,
    };
    let remote_c = RemotePid {
        node: Atom::ERROR,
        pid_number: 12,
        serial: 0,
    };
    add_remote_link(&shared, first, remote_a);
    add_remote_link(&shared, first, remote_b);
    add_remote_link(&shared, second, remote_b);
    add_remote_link(&shared, other_node, remote_c);
    set_trap_exit(&shared, first, true);
    set_trap_exit(&shared, second, true);

    supervision_integration::connection_down(&shared, node);

    assert_eq!(
        read_mailbox_tuple(&shared, first).expect("first noconnection")[2],
        Term::atom(Atom::NOCONNECTION)
    );
    assert_eq!(
        read_mailbox_tuple(&shared, second).expect("second noconnection")[2],
        Term::atom(Atom::NOCONNECTION)
    );
    assert!(is_alive(&shared, other_node));
}
