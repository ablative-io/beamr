//! Tests for the inbound link-control apply path and the DC-4 kind-gating:
//! wire LINK/EXIT/EXIT2 frames through the real dispatcher and read loop,
//! the LINK-to-dead noproc reply over a real socket, the D9 Executing-arm
//! regression, and the ManualDisconnect-from-a-plain-thread backstop.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::connection_lifecycle::register_scheduler_connection_subscriber;
use super::connection_lifecycle_tests::{
    assert_noconnection_exit, install_peer, mailbox_message_count, socket_pair,
};
use super::remote_supervision::{RemoteExitKind, apply_inbound_link};
use super::supervision_integration::{
    SchedulerDistributionControlFacility, SchedulerDistributionSendFacility, establish_remote_link,
    register_distribution_control_handler, remove_remote_link,
};
use super::supervision_tests::{
    add_remote_link, insert_process, is_alive, make_executing, make_shared_state,
    make_shared_state_with_dist_sender, make_shared_state_with_dist_sender_named,
    read_mailbox_tuple, set_trap_exit,
};
use super::*;
use crate::atom::Atom;
use crate::distribution::control::{
    ControlError, ControlMessage, decode_control, encode_send_frame, split_frame,
};
use crate::distribution::control_link::{
    ControlOp, ControlSinks, dispatch_frame, encode_exit_frame, encode_link_frame,
};
use crate::distribution::remote_link::{DistributionControlFacility, RemoteLinkError};
use crate::distribution::sender::ControlOutbound;
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
pub(super) fn read_peer_frame(peer: &mut std::net::TcpStream) -> (Vec<u8>, Vec<u8>) {
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

/// All mailbox messages of a Present process, in mailbox order.
fn mailbox_terms(shared: &SharedState, pid: u64) -> Vec<Term> {
    let entry = shared
        .process_bodies
        .get(&pid)
        .unwrap_or_else(|| panic!("process {pid} exists"));
    let mut slot = lock_or_recover(&entry);
    let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
        panic!("process {pid} is present");
    };
    process.mailbox_mut().drain_arrival();
    process.mailbox().scan_iter().copied().collect()
}

fn control_facility(shared: &Arc<SharedState>) -> SchedulerDistributionControlFacility {
    SchedulerDistributionControlFacility {
        shared: Arc::clone(shared),
    }
}

/// Scenario 1 (outbound half) + T-4: `link_remote` establishes the local
/// half-link and puts a real LINK control on the wire — with the serial-0
/// local `from` a later EXIT's equality gate needs — re-linking is idempotent
/// success, and `unlink_remote` removes the half and sends UNLINK.
#[test]
fn facility_link_and_unlink_ride_the_wire_and_track_the_local_half() {
    let shared = make_shared_state_with_dist_sender();
    let handle = shared.dist_sender.as_ref().expect("dist sender").handle();
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
    let handle = shared.dist_sender.as_ref().expect("dist sender").handle();
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

/// Scenario 1, in-crate half over a real socket: messages the (test-driven)
/// peer sends before its death all precede the EXIT in the target's mailbox —
/// the receive-side face of DC-6 (one socket, frames applied serially) — and
/// the trapping target gets exactly one `{'EXIT', <ext-pid>, error}` 3-tuple.
#[test]
fn presend_messages_precede_wire_exit_and_trapping_target_survives() {
    let shared = make_shared_state_with_dist_sender();
    register_distribution_control_handler(&shared);
    let handle = shared.dist_sender.as_ref().expect("dist sender").handle();
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

/// The `Scheduler::{link_remote, unlink_remote}` embedder API delegates to the
/// distribution control facility with its preconditions intact.
#[test]
fn scheduler_link_remote_and_unlink_remote_delegate_with_preconditions() {
    let scheduler = Scheduler::new(SchedulerConfig::default(), Arc::new(ModuleRegistry::new()))
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

/// T-2 (DC-4, the R1 kill-shot): a 2000-process exit storm between two real
/// shared-state "nodes" on one socket pair — far past the 256-slot control
/// lane by construction — converges to EXACTLY one `{'EXIT', _, R}` per link,
/// each R ∈ {error, noconnection}: wire EXITs that rode generation G before
/// any overflow, and the noconnection backstop for everything after DC-1(b)
/// downed it. Nothing is lost, nothing is double-signalled.
#[test]
fn exit_storm_delivers_exactly_one_signal_per_link_across_nodes() {
    const STORM: u64 = 2000;
    const BASE: u64 = 1000;
    let shared_a = make_shared_state_with_dist_sender_named("a@test");
    let shared_b = make_shared_state_with_dist_sender_named("b@test");
    register_distribution_control_handler(&shared_b);
    register_scheduler_connection_subscriber(&shared_a);
    register_scheduler_connection_subscriber(&shared_b);

    let node_b_on_a = shared_a.atom_table.intern("b@test");
    let node_a_on_b = shared_b.atom_table.intern("a@test");
    let (server, client, addr) = socket_pair();
    {
        let handle = shared_a
            .dist_sender
            .as_ref()
            .expect("dist sender a")
            .handle();
        let _context = handle.enter();
        shared_a
            .distribution_connections
            .register_test_connection(node_b_on_a, addr, server)
            .expect("register a->b connection");
    }
    {
        let handle = shared_b
            .dist_sender
            .as_ref()
            .expect("dist sender b")
            .handle();
        let _context = handle.enter();
        shared_b
            .distribution_connections
            .register_test_connection(node_a_on_b, addr, client)
            .expect("register b->a connection");
    }

    for index in 0..STORM {
        let a_pid = insert_process(&shared_a, BASE + index);
        add_remote_link(
            &shared_a,
            a_pid,
            RemotePid {
                node: node_b_on_a,
                pid_number: BASE + index,
                serial: 0,
            },
        );
        let b_pid = insert_process(&shared_b, BASE + index);
        set_trap_exit(&shared_b, b_pid, true);
        add_remote_link(
            &shared_b,
            b_pid,
            RemotePid {
                node: node_a_on_b,
                pid_number: BASE + index,
                serial: 0,
            },
        );
    }

    // The burst. If the lane overflows mid-loop, the inline down-hook runs
    // right here on this thread and the remaining sends drop (no connection)
    // — their signals must arrive via B's own EOF backstop instead.
    for index in 0..STORM {
        cleanup_exited_process(&shared_a, BASE + index, ExitReason::Error);
    }

    // HS-5-style watchdog: convergence bounded by a deadline, not a sleep.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let total: usize = (0..STORM)
            .map(|index| mailbox_message_count(&shared_b, BASE + index))
            .sum();
        if total >= STORM as usize {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "exit storm never converged: {total}/{STORM} signals delivered"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Settle briefly so a hypothetical late duplicate would be caught below.
    std::thread::sleep(Duration::from_millis(100));

    let mut wire_exits = 0_usize;
    let mut coarsened = 0_usize;
    for index in 0..STORM {
        let pid = BASE + index;
        assert_eq!(
            mailbox_message_count(&shared_b, pid),
            1,
            "exactly one exit signal per link (DC-4), pid {pid}"
        );
        let tuple = read_mailbox_tuple(&shared_b, pid).expect("exit tuple");
        assert_eq!(tuple.len(), 3);
        assert_eq!(tuple[0], Term::atom(Atom::EXIT));
        if tuple[2] == Term::atom(Atom::ERROR) {
            wire_exits += 1;
        } else if tuple[2] == Term::atom(Atom::NOCONNECTION) {
            coarsened += 1;
        } else {
            panic!("unexpected exit reason {:?} for pid {pid}", tuple[2]);
        }
        assert!(is_alive(&shared_b, pid), "trapping target survives");
    }
    assert_eq!(wire_exits + coarsened, STORM as usize);
    if coarsened > 0 {
        // Overflow path: the pinned generation is down on A immediately, and
        // B converges via EOF within a bounded window (DC-1 both-sides).
        assert!(
            shared_a
                .distribution_connections
                .get_connection(node_b_on_a)
                .is_none(),
            "overflow must purge the pinned connection on the sending side"
        );
        let eof_deadline = Instant::now() + Duration::from_secs(30);
        while shared_b
            .distribution_connections
            .get_connection(node_a_on_b)
            .is_some()
        {
            assert!(
                Instant::now() < eof_deadline,
                "B never observed the downed pair via EOF"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// T-8 (ruling 8, the accepted cross-peer blast radius, pinned): with the
/// shared control lane wedged full behind a never-reading peer, a control to
/// a HEALTHY peer overflows and downs the healthy pin — and the inline hook
/// converts the caller's links to `noconnection` before the facility call
/// returns. Correctness holds (no lost signal, DC-1/DC-3); the healthy pair
/// is merely down pending redial — an availability blip.
#[test]
fn control_overflow_to_healthy_peer_converges_to_noconnection_blip() {
    let shared = make_shared_state_with_dist_sender();
    register_scheduler_connection_subscriber(&shared);
    let sender = shared.dist_sender.as_ref().expect("dist sender");
    let handle = sender.handle();
    let _context = handle.enter();

    let wedged_node = shared.atom_table.intern("wedged@test");
    let healthy_node = shared.atom_table.intern("healthy@test");
    // Held but NEVER read: writes to it park once the kernel buffers fill.
    let wedged_peer = install_peer(&shared, wedged_node);
    let _healthy_peer = install_peer(&shared, healthy_node);
    let wedged = shared
        .distribution_connections
        .get_connection(wedged_node)
        .expect("wedged connection is in the table");

    let watcher = insert_process(&shared, 1);
    set_trap_exit(&shared, watcher, true);
    let healthy_remote = RemotePid {
        node: healthy_node,
        pid_number: 9,
        serial: 0,
    };
    add_remote_link(&shared, watcher, healthy_remote);

    // Park the drain: one oversized control to the never-reading peer blocks
    // `write_all` until WRITE_TIMEOUT (5 s), and the lane fills behind it.
    let mut big = vec![0_u8; 16 * 1024 * 1024];
    big[0] = 1;
    let control_len = u32::try_from(big.len()).expect("control fits u32");
    let mut big_frame = Vec::with_capacity(8 + big.len());
    big_frame.extend_from_slice(&control_len.to_be_bytes());
    big_frame.extend_from_slice(&0_u32.to_be_bytes());
    big_frame.extend_from_slice(&big);
    sender
        .enqueue_control(ControlOutbound {
            connection: Arc::clone(&wedged),
            frame: Arc::from(big_frame.into_boxed_slice()),
        })
        .expect("first control accepted into an empty lane");
    // The drain has begun (and therefore parked on) the oversized write once
    // the peer side observes its first byte — everything below happens well
    // inside the 5 s window during which the drain frees no lane slot.
    wedged_peer
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("peer read timeout");
    let mut probe = [0_u8; 1];
    assert_eq!(
        wedged_peer
            .peek(&mut probe)
            .expect("drain begins the wedged write"),
        1
    );

    // Fill the lane behind the parked write. The first Overflow marks the
    // WEDGED pin down (its own DC-1(b)) and proves the lane is full.
    let filler: Arc<[u8]> = Arc::from(vec![0_u8; 8].into_boxed_slice());
    let mut overflowed = false;
    for _ in 0..=crate::distribution::sender::DIST_CONTROL_QUEUE_CAP {
        match sender.enqueue_control(ControlOutbound {
            connection: Arc::clone(&wedged),
            frame: Arc::clone(&filler),
        }) {
            Ok(()) => {}
            Err(_) => {
                overflowed = true;
                break;
            }
        }
    }
    assert!(
        overflowed,
        "filling behind a parked write must fill the lane"
    );
    assert!(wedged.is_down(), "overflow downs its own pinned connection");
    assert_eq!(
        mailbox_message_count(&shared, watcher),
        0,
        "the healthy link is untouched so far"
    );

    // The accepted-risk moment: a control to the HEALTHY peer hits the full
    // shared lane. INV-SYNC on the hook chain means the noconnection is in
    // the watcher's mailbox before exit_remote returns.
    let facility = control_facility(&shared);
    assert_eq!(
        facility.exit_remote(watcher, healthy_remote, ExitReason::Error),
        Ok(()),
        "EXIT2 stays best-effort Ok even when it overflows"
    );

    let tuple = read_mailbox_tuple(&shared, watcher).expect("noconnection EXIT");
    assert_noconnection_exit(&tuple, healthy_remote);
    assert_eq!(
        mailbox_message_count(&shared, watcher),
        1,
        "blip, not a storm"
    );
    assert!(is_alive(&shared, watcher), "trapping watcher survives");
    assert!(
        shared
            .distribution_connections
            .get_connection(healthy_node)
            .is_none(),
        "the healthy pair is down pending redial"
    );
}

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

/// A nonzero embedder-supplied `RemotePid.serial` is normalized to the wire
/// link identity (serial 0) at establishment, so the peer's later EXIT —
/// whose `from` is always minted with serial 0 — hits the DC-4 equality gate
/// instead of silently no-oping and losing the death signal until node-down.
#[test]
fn nonzero_embedder_serial_cannot_dodge_the_wire_exit_gate() {
    let shared = make_shared_state_with_dist_sender();
    let handle = shared.dist_sender.as_ref().expect("dist sender").handle();
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
    let handle = shared.dist_sender.as_ref().expect("dist sender").handle();
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
        .distribution_connections
        .get_connection(peer_node)
        .expect("the connection must survive the refused controls");
    assert!(
        !connection.is_down(),
        "an unencodable pid is refused at the send boundary, not converted \
         into a whole-connection teardown"
    );
    drop(peer);
}
