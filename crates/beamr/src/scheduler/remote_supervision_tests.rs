//! Tests for the inbound link-control apply path and the DC-4 kind-gating:
//! wire LINK/EXIT/EXIT2 frames through the real dispatcher and read loop,
//! the LINK-to-dead noproc reply over a real socket, the D9 Executing-arm
//! regression, and the ManualDisconnect-from-a-plain-thread backstop.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::connection_lifecycle::register_scheduler_connection_subscriber;
use super::connection_lifecycle_tests::{
    assert_noconnection_exit, install_peer, mailbox_message_count,
};
use super::remote_supervision::{RemoteExitKind, apply_inbound_link};
use super::supervision_integration::{
    SchedulerDistributionSendFacility, establish_remote_link, register_distribution_control_handler,
};
use super::supervision_tests::{
    add_remote_link, insert_process, is_alive, make_executing, make_shared_state,
    make_shared_state_with_dist_sender, read_mailbox_tuple, set_trap_exit,
};
use super::*;
use crate::atom::Atom;
use crate::distribution::control::{
    ControlError, ControlMessage, decode_control, encode_send_frame, split_frame,
};
use crate::distribution::control_link::{
    ControlOp, ControlSinks, dispatch_frame, encode_exit_frame, encode_link_frame,
};
use crate::ets::{EtsTableMetadata, EtsTableType, Protection};
use crate::process::{ExitReason, RemotePid};
use crate::scheduler::execution::{
    cleanup_exited_process, cleanup_if_tombstoned_after_store, store_runnable_process,
};
use crate::term::boxed::{ExternalPid, write_external_pid};

/// Route a raw control frame through `dispatch_frame` with the exact sink
/// bundle `register_distribution_control_handler` wires up: the scheduler
/// facility on every sink, the authenticated `origin`, and the local node.
fn dispatch_wire_frame(
    shared: &Arc<SharedState>,
    origin: Atom,
    frame: &[u8],
) -> Result<bool, ControlError> {
    let facility = SchedulerDistributionSendFacility {
        shared: Arc::clone(shared),
    };
    let sinks = ControlSinks {
        delivery: &facility,
        registry: Some(&facility),
        pg: Some(&facility),
        links: Some(&facility),
        origin_node: Some(origin),
        local_node: Some(shared.local_node.name),
    };
    let (control, payload) = split_frame(frame).expect("frame splits");
    dispatch_frame(control, payload, &shared.atom_table, &sinks)
}

fn remote_links_of(shared: &SharedState, pid: u64) -> Vec<RemotePid> {
    let entry = shared
        .process_bodies
        .get(&pid)
        .unwrap_or_else(|| panic!("process {pid} exists"));
    let slot = lock_or_recover(&entry);
    match &*slot {
        ProcessSlot::Present(ScheduledProcess(process)) => process.remote_links().to_vec(),
        ProcessSlot::Executing(metadata) => metadata.remote_links.clone(),
        ProcessSlot::Absent => Vec::new(),
    }
}

fn add_local_link(shared: &SharedState, a: u64, b: u64) {
    for (pid, linked) in [(a, b), (b, a)] {
        let entry = shared
            .process_bodies
            .get(&pid)
            .unwrap_or_else(|| panic!("process {pid} exists"));
        let mut slot = lock_or_recover(&entry);
        let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
            panic!("process {pid} is present");
        };
        process.add_link(linked);
    }
}

/// Read one `[control_len|payload_len]`-framed message from the peer side of
/// a test socket; returns `(control, payload)` ETF bytes.
fn read_peer_frame(peer: &mut std::net::TcpStream) -> (Vec<u8>, Vec<u8>) {
    peer.set_read_timeout(Some(Duration::from_secs(10)))
        .expect("peer read timeout");
    let mut header = [0_u8; 8];
    peer.read_exact(&mut header).expect("frame header");
    let control_len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
    let payload_len = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
    let mut control = vec![0_u8; control_len];
    peer.read_exact(&mut control).expect("frame control");
    let mut payload = vec![0_u8; payload_len];
    peer.read_exact(&mut payload).expect("frame payload");
    (control, payload)
}

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
    let handle = shared.dist_sender.as_ref().expect("dist sender").handle();
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
    let handle = shared.dist_sender.as_ref().expect("dist sender").handle();
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
        let handle = shared.dist_sender.as_ref().expect("dist sender").handle();
        let _context = handle.enter();
        install_peer(&shared, node)
    };

    let shared_for_thread = Arc::clone(&shared);
    let worker = std::thread::spawn(move || {
        assert!(
            shared_for_thread
                .distribution_connections
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
