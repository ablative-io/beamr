//! The distribution control facility: link, unlink and exit2 over the
//! wire, and the Scheduler methods that delegate to it.

use super::*;

/// Scenario 1 (outbound half) + T-4: `link_remote` establishes the local
/// half-link and puts a real LINK control on the wire — with the serial-0
/// local `from` a later EXIT's equality gate needs — re-linking is idempotent
/// success, and `unlink_remote` removes the half and sends UNLINK.
#[test]
fn facility_link_and_unlink_ride_the_wire_and_track_the_local_half() {
    let shared = make_shared_state_with_dist_sender();
    let handle = shared.dist_sender_or_panic().handle();
    let _context = handle.enter();

    let peer_node = shared.atom_table.intern("peer@test");
    let caller = insert_process(&shared, 1);
    let mut peer = install_peer(&shared, peer_node);
    let target = RemotePid {
        node: peer_node,
        pid_number: 42,
        serial: 3,
    };
    let facility = control_facility(&shared);

    assert_eq!(facility.link_remote(caller, target), Ok(()));
    assert_eq!(
        facility.link_remote(caller, target),
        Ok(()),
        "duplicate link_remote is idempotent success (T-4)"
    );
    assert_eq!(
        remote_links_of(&shared, caller),
        vec![RemotePid {
            serial: 0,
            ..target
        }],
        "the stored half-link is the serial-0 wire identity the peer's \
         EXIT/UNLINK `from` will carry (DC-4 equality gate)"
    );

    let (control, payload) = read_peer_frame(&mut peer);
    assert_eq!(
        decode_control(&control, &shared.atom_table),
        Ok(ControlMessage::Link {
            from: RemotePid {
                node: shared.local_node.name,
                pid_number: caller,
                serial: 0,
            },
            to_pid: target.pid_number,
        })
    );
    assert_eq!(payload, vec![131, 106], "link controls carry payload = NIL");
    // The duplicate link_remote also rode the wire (the peer applies it
    // idempotently, ruling 2); skip past it to reach the UNLINK.
    let _duplicate_link = read_peer_frame(&mut peer);

    assert_eq!(facility.unlink_remote(caller, target), Ok(()));
    assert!(remote_links_of(&shared, caller).is_empty());
    let (control, _payload) = read_peer_frame(&mut peer);
    assert_eq!(
        decode_control(&control, &shared.atom_table),
        Ok(ControlMessage::Unlink {
            from: RemotePid {
                node: shared.local_node.name,
                pid_number: caller,
                serial: 0,
            },
            to_pid: target.pid_number,
        })
    );
}

/// `link_remote` preconditions: a dead caller is `BadTarget`; an unconnected
/// target node is `NoConnection` (no auto-dial, C7) and must not leave an
/// immortal local half-link behind. `exit_remote` stays `Ok` regardless —
/// EXIT2 is best-effort fire-and-forget (ruling 7).
#[test]
fn facility_link_remote_preconditions_and_best_effort_exit2() {
    let shared = make_shared_state_with_dist_sender();
    let unconnected = shared.atom_table.intern("unconnected@test");
    let target = RemotePid {
        node: unconnected,
        pid_number: 42,
        serial: 0,
    };
    let facility = control_facility(&shared);

    assert_eq!(
        facility.link_remote(999, target),
        Err(RemoteLinkError::BadTarget),
        "a dead caller cannot link"
    );

    let caller = insert_process(&shared, 1);
    assert_eq!(
        facility.link_remote(caller, target),
        Err(RemoteLinkError::NoConnection),
        "no connection means no LINK — connect first, then link"
    );
    assert!(
        remote_links_of(&shared, caller).is_empty(),
        "the NoConnection arm must not leave an immortal half-link"
    );

    assert_eq!(
        facility.exit_remote(caller, target, ExitReason::Error),
        Ok(()),
        "exit/2 is best-effort: undeliverable is still Ok (ruling 7)"
    );
    assert_eq!(
        facility.unlink_remote(caller, target),
        Ok(()),
        "unlink to an unconnected node drops the control and succeeds"
    );
}

/// Scenario 3, outbound halves: a linked process dying of Kill leaves the node
/// as a wire EXIT carrying `killed` (pre-terminalized by `propagate_exit`),
/// while `exit_remote(.., Kill)` emits an EXIT2 carrying RAW `kill` — the
/// receiver-side untrappable form.
#[test]
fn kill_crosses_the_wire_as_killed_but_exit2_carries_raw_kill() {
    let shared = make_shared_state_with_dist_sender();
    let handle = shared.dist_sender_or_panic().handle();
    let _context = handle.enter();

    let peer_node = shared.atom_table.intern("peer@test");
    let dying = insert_process(&shared, 1);
    let target = RemotePid {
        node: peer_node,
        pid_number: 42,
        serial: 0,
    };
    add_remote_link(&shared, dying, target);
    let mut peer = install_peer(&shared, peer_node);

    cleanup_exited_process(&shared, dying, ExitReason::Kill);
    let (control, _payload) = read_peer_frame(&mut peer);
    assert_eq!(
        decode_control(&control, &shared.atom_table),
        Ok(ControlMessage::Exit {
            from: RemotePid {
                node: shared.local_node.name,
                pid_number: dying,
                serial: 0,
            },
            to_pid: target.pid_number,
            reason: ExitReason::Killed,
        }),
        "link EXIT is pre-terminalized: kill leaves the node as killed"
    );

    let caller = insert_process(&shared, 2);
    let facility = control_facility(&shared);
    assert_eq!(
        facility.exit_remote(caller, target, ExitReason::Kill),
        Ok(())
    );
    let (control, _payload) = read_peer_frame(&mut peer);
    assert_eq!(
        decode_control(&control, &shared.atom_table),
        Ok(ControlMessage::Exit2 {
            from: RemotePid {
                node: shared.local_node.name,
                pid_number: caller,
                serial: 0,
            },
            to_pid: target.pid_number,
            reason: ExitReason::Kill,
        }),
        "EXIT2 carries the raw reason — kill stays untrappable at the receiver"
    );
}

/// The `Scheduler::{link_remote, unlink_remote}` embedder API delegates to the
/// distribution control facility with its preconditions intact.
#[test]
fn scheduler_link_remote_and_unlink_remote_delegate_with_preconditions() {
    let scheduler = Scheduler::new(
        SchedulerConfig::default(),
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .expect("scheduler starts");
    let node = scheduler.shared.atom_table.intern("unconnected@test");
    let remote = RemotePid {
        node,
        pid_number: 7,
        serial: 0,
    };

    assert_eq!(
        scheduler.link_remote(4242, remote),
        Err(RemoteLinkError::BadTarget),
        "a dead caller is BadTarget"
    );

    let pid = insert_process(&scheduler.shared, 4242);
    assert_eq!(
        scheduler.link_remote(pid, remote),
        Err(RemoteLinkError::NoConnection),
        "no connection and no auto-dial"
    );
    assert!(
        remote_links_of(&scheduler.shared, pid).is_empty(),
        "the NoConnection arm must not leave an immortal half-link"
    );

    add_remote_link(&scheduler.shared, pid, remote);
    assert_eq!(scheduler.unlink_remote(pid, remote), Ok(()));
    assert!(remote_links_of(&scheduler.shared, pid).is_empty());
    scheduler.shutdown();
}
