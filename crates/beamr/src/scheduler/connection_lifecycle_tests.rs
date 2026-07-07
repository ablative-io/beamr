//! Tests for the scheduler's composed connection-event subscriber: node-death
//! cleanup (pg purge, then noconnection delivery) fired through the REAL hook
//! chain, redelivery idempotence at the subscriber seam, and the
//! Executing-slot noconnection variants (including the D9 ruling: a tombstoned
//! non-trap target is cleaned up at store-back — deferred, not skipped).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::connection_lifecycle::{
    handle_connection_event, register_scheduler_connection_subscriber,
};
use super::supervision_tests::{
    add_remote_link, insert_process, is_alive, make_executing, make_shared_state,
    read_mailbox_tuple, set_trap_exit,
};
use super::*;
use crate::atom::Atom;
use crate::distribution::connection::ConnectionDownReason;
use crate::distribution::connection_events::{ConnectionEvent, ConnectionGeneration};
use crate::process::RemotePid;
use crate::scheduler::execution::{cleanup_if_tombstoned_after_store, store_runnable_process};
use crate::term::boxed::ExternalPid;

/// A connected localhost socket pair: (server end for the manager, client end
/// held by the test as the "peer", the server's address).
pub(super) fn socket_pair() -> (std::net::TcpStream, std::net::TcpStream, SocketAddr) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind socket pair listener");
    let addr = listener.local_addr().expect("socket pair local addr");
    let client = std::net::TcpStream::connect(addr).expect("connect socket pair client");
    let (server, _) = listener.accept().expect("accept socket pair server");
    (server, client, addr)
}

/// Build a runtime and hand its handle to the shared state's connection
/// manager, so `register_test_connection` can spawn real read lifecycles.
/// Callers must also `enter()` the returned runtime before installing.
fn runtime_backing(shared: &SharedState) -> tokio::runtime::Runtime {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build test runtime");
    shared
        .distribution_connections
        .set_runtime_handle(runtime.handle().clone());
    runtime
}

/// Install a handshake-less real-socket connection for `node`; returns the
/// peer-side socket (kept open by the caller so the read loop does not
/// immediately observe EOF). Must run inside a tokio runtime context.
pub(super) fn install_peer(shared: &SharedState, node: Atom) -> std::net::TcpStream {
    let (server, client, addr) = socket_pair();
    let _connection = shared
        .distribution_connections
        .register_test_connection(node, addr, server)
        .expect("register test connection");
    client
}

pub(super) fn mailbox_message_count(shared: &SharedState, pid: u64) -> usize {
    let entry = shared
        .process_bodies
        .get(&pid)
        .unwrap_or_else(|| panic!("process {pid} exists"));
    let mut slot = lock_or_recover(&entry);
    let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
        panic!("process {pid} is present");
    };
    process.mailbox_mut().drain_arrival();
    process.mailbox().message_count()
}

fn noconnection_down(node: Atom) -> ConnectionEvent {
    ConnectionEvent::down(
        node,
        ConnectionGeneration::from_raw(1),
        ConnectionDownReason::PeerClosed,
    )
}

/// Assert `tuple` is `{'EXIT', ExternalPid(remote), noconnection}`.
pub(super) fn assert_noconnection_exit(tuple: &[Term], remote: RemotePid) {
    assert_eq!(tuple.len(), 3, "remote EXIT message is a 3-tuple");
    assert_eq!(tuple[0], Term::atom(Atom::EXIT));
    let source = ExternalPid::new(tuple[1]).expect("remote source pid");
    assert_eq!(source.node(), Some(remote.node));
    assert_eq!(source.pid_number(), remote.pid_number);
    assert_eq!(source.serial(), remote.serial);
    assert_eq!(tuple[2], Term::atom(Atom::NOCONNECTION));
}

/// Scenario 4: node death delivers noconnection via the REAL hook chain — a
/// dropped peer socket EOFs the read loop, `mark_down` fires the hub, and the
/// composed scheduler subscriber applies the supervision outcomes: the
/// trapping process receives `{'EXIT', ExternalPid, noconnection}`, the
/// non-trapping process dies, and a process linked to a DIFFERENT node is
/// untouched. (The direct-call test in supervision_tests stays as the
/// applier-level contract; this pins the wiring.)
#[test]
fn node_death_via_real_hook_delivers_noconnection_outcomes() {
    let shared = make_shared_state();
    register_scheduler_connection_subscriber(&shared);
    let runtime = runtime_backing(&shared);
    let _context = runtime.enter();

    let node = shared.atom_table.intern("peer@127.0.0.1");
    let other_node = shared.atom_table.intern("other@127.0.0.1");
    let trapping = insert_process(&shared, 1);
    let non_trapping = insert_process(&shared, 2);
    let other = insert_process(&shared, 3);
    set_trap_exit(&shared, trapping, true);
    let remote_a = RemotePid {
        node,
        pid_number: 10,
        serial: 0,
    };
    add_remote_link(&shared, trapping, remote_a);
    add_remote_link(
        &shared,
        non_trapping,
        RemotePid {
            node,
            pid_number: 11,
            serial: 0,
        },
    );
    add_remote_link(
        &shared,
        other,
        RemotePid {
            node: other_node,
            pid_number: 12,
            serial: 0,
        },
    );

    let peer = install_peer(&shared, node);
    // Drop the peer socket: the read loop observes EOF, marks the connection
    // down, and the hub dispatches Down to the composed subscriber on the
    // runtime thread.
    drop(peer);

    let deadline = Instant::now() + Duration::from_secs(10);
    while is_alive(&shared, non_trapping) || read_mailbox_tuple(&shared, trapping).is_none() {
        assert!(
            Instant::now() < deadline,
            "node death outcomes never arrived via the real hook"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let tuple = read_mailbox_tuple(&shared, trapping).expect("trapping noconnection EXIT");
    assert_noconnection_exit(&tuple, remote_a);
    assert!(is_alive(&shared, trapping), "trapping process survives");
    assert!(
        !is_alive(&shared, non_trapping),
        "non-trapping process dies of noconnection"
    );
    assert!(
        is_alive(&shared, other),
        "a process linked only to a different node is untouched"
    );
}

/// Scenario 5 (structural): pg purge strictly precedes noconnection delivery
/// inside the composed subscriber, so a probe subscriber registered AFTER the
/// scheduler's — the earliest an embedder can observe the Down — already sees
/// the dead node's pg members gone AND the noconnection tuple in the trap
/// target's mailbox. Deterministic: purge and delivery are sequential
/// statements, and INV-SYNC means `disconnect_node` returning implies the
/// whole chain ran (no polling).
#[test]
fn probe_after_scheduler_observes_post_purge_state_and_delivered_noconnection() {
    let shared = make_shared_state();
    register_scheduler_connection_subscriber(&shared);
    let runtime = runtime_backing(&shared);
    let _context = runtime.enter();

    let node = shared.atom_table.intern("peer@127.0.0.1");
    let scope = shared.atom_table.intern("pg");
    let group = shared.atom_table.intern("workers");
    shared
        .pg_registry
        .apply_remote_join(scope, group, node, 55, 0);

    let target = insert_process(&shared, 1);
    set_trap_exit(&shared, target, true);
    add_remote_link(
        &shared,
        target,
        RemotePid {
            node,
            pid_number: 10,
            serial: 0,
        },
    );

    let peer = install_peer(&shared, node);

    // Probe registered AFTER the scheduler's composed subscriber (Weak per
    // INV-SUB-DISCIPLINE: the closure is stored inside the manager the shared
    // state owns).
    let observed: Arc<Mutex<Option<(usize, bool)>>> = Arc::new(Mutex::new(None));
    let observed_for_probe = Arc::clone(&observed);
    let weak = Arc::downgrade(&shared);
    shared
        .distribution_connections
        .subscribe_connection_events(move |event| {
            let ConnectionEvent::Down(_) = event else {
                return;
            };
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let members = shared.pg_registry.remote_members(scope, group).len();
            let delivered = read_mailbox_tuple(&shared, target)
                .is_some_and(|tuple| tuple[2] == Term::atom(Atom::NOCONNECTION));
            *observed_for_probe.lock().expect("observed lock") = Some((members, delivered));
        });

    assert!(shared.distribution_connections.disconnect_node(node));
    drop(peer);

    let (members, delivered) = observed
        .lock()
        .expect("observed lock")
        .expect("the probe subscriber ran before disconnect_node returned");
    assert_eq!(
        members, 0,
        "the probe must never observe the dead node's pg members"
    );
    assert!(
        delivered,
        "the noconnection EXIT must already be in the trap target's mailbox"
    );
}

/// Scenario 7, R2 regression (scheduler half): a legacy
/// `register_connection_down` registrant added AFTER the scheduler's composed
/// subscriber (the earliest an embedder can register) still fires — AND both
/// internal effects, pg purge and noconnection delivery, still happen.
/// Pre-hub, the pg purge lived in the replace-on-register slot, so this exact
/// registration silently evicted the scheduler's node-death cleanup.
#[test]
fn r2_late_legacy_registrant_leaves_purge_and_noconnection_intact() {
    let shared = make_shared_state();
    register_scheduler_connection_subscriber(&shared);
    let runtime = runtime_backing(&shared);
    let _context = runtime.enter();

    let node = shared.atom_table.intern("peer@127.0.0.1");
    let scope = shared.atom_table.intern("pg");
    let group = shared.atom_table.intern("workers");
    shared
        .pg_registry
        .apply_remote_join(scope, group, node, 55, 0);

    let target = insert_process(&shared, 1);
    set_trap_exit(&shared, target, true);
    let remote = RemotePid {
        node,
        pid_number: 10,
        serial: 0,
    };
    add_remote_link(&shared, target, remote);

    // The R2 hazard: a legacy registration landing after construction.
    let legacy_seen: Arc<Mutex<Vec<(Atom, ConnectionDownReason)>>> =
        Arc::new(Mutex::new(Vec::new()));
    let legacy_seen_for_slot = Arc::clone(&legacy_seen);
    shared
        .distribution_connections
        .register_connection_down(move |event| {
            legacy_seen_for_slot
                .lock()
                .expect("legacy log lock")
                .push((event.node, event.reason));
        });

    let peer = install_peer(&shared, node);
    assert!(shared.distribution_connections.disconnect_node(node));
    drop(peer);

    // INV-SYNC: every effect is observable the moment disconnect_node returns.
    assert_eq!(
        *legacy_seen.lock().expect("legacy log lock"),
        vec![(node, ConnectionDownReason::ManualDisconnect)],
        "the late legacy registrant still fires, 0.11 shape"
    );
    assert!(
        shared.pg_registry.remote_members(scope, group).is_empty(),
        "pg purge still ran despite the legacy registration"
    );
    let tuple = read_mailbox_tuple(&shared, target).expect("noconnection EXIT still delivered");
    assert_noconnection_exit(&tuple, remote);
    assert!(is_alive(&shared, target), "trapping target survives");
}

/// Scenario 6ii: redelivery idempotence at the subscriber seam. Driving
/// `handle_connection_event(Down)` twice for one node produces exactly one
/// noconnection EXIT — the applier removes the remote link on first delivery,
/// so the second pass finds nothing to signal.
#[test]
fn redelivered_down_produces_exactly_one_noconnection_exit() {
    let shared = make_shared_state();
    let node = Atom::OK;
    let target = insert_process(&shared, 1);
    set_trap_exit(&shared, target, true);
    let remote = RemotePid {
        node,
        pid_number: 10,
        serial: 0,
    };
    add_remote_link(&shared, target, remote);

    handle_connection_event(&shared, noconnection_down(node));
    handle_connection_event(&shared, noconnection_down(node));

    assert_eq!(
        mailbox_message_count(&shared, target),
        1,
        "a redelivered Down must not duplicate the noconnection EXIT"
    );
    let tuple = read_mailbox_tuple(&shared, target).expect("noconnection EXIT");
    assert_noconnection_exit(&tuple, remote);
    assert!(is_alive(&shared, target));
}

/// Scenario 8, trapping Executing target: the noconnection signal parks in
/// `pending_exit_messages` while the slot is Executing and is drained into the
/// mailbox at store-back (parity with
/// `monitor_down_for_executing_watcher_is_delivered_on_store_back`).
#[test]
fn noconnection_to_trapping_executing_target_is_delivered_on_store_back() {
    let shared = make_shared_state();
    let node = Atom::OK;
    let target = insert_process(&shared, 1);
    set_trap_exit(&shared, target, true);
    let remote = RemotePid {
        node,
        pid_number: 10,
        serial: 0,
    };
    add_remote_link(&shared, target, remote);
    let process = make_executing(&shared, target);

    handle_connection_event(&shared, noconnection_down(node));

    assert!(
        read_mailbox_tuple(&shared, target).is_none(),
        "while Executing, the signal is parked in metadata, not the mailbox"
    );

    store_runnable_process(&shared, process);

    let tuple = read_mailbox_tuple(&shared, target).expect("noconnection EXIT after store-back");
    assert_noconnection_exit(&tuple, remote);
    assert!(is_alive(&shared, target), "trapping target survives");
}

/// Scenario 8, non-trapping Executing target — the D9-ruling test. The
/// Executing non-trap arm only inserts an exit tombstone, which is NOT a
/// latent bug: an Executing slot always has a worker mid-slice that must store
/// back, and every store-back outcome arm re-checks the tombstone. Pin it:
/// the target is still alive after delivery (deferred), and the store-back
/// tombstone check runs the full cleanup (not skipped).
#[test]
fn d9_non_trapping_executing_target_is_tombstoned_then_fully_cleaned_at_store_back() {
    let shared = make_shared_state();
    let node = Atom::OK;
    let target = insert_process(&shared, 1);
    add_remote_link(
        &shared,
        target,
        RemotePid {
            node,
            pid_number: 10,
            serial: 0,
        },
    );
    let process = make_executing(&shared, target);

    handle_connection_event(&shared, noconnection_down(node));

    assert!(
        is_alive(&shared, target),
        "delivery is deferred while Executing: only the tombstone is placed"
    );

    // The worker's slice ends: store back, then the tombstone re-check.
    store_runnable_process(&shared, process);
    assert!(
        cleanup_if_tombstoned_after_store(&shared, target),
        "the tombstone must be observed at store-back"
    );

    assert!(
        !is_alive(&shared, target),
        "full cleanup removed the target"
    );
    assert!(
        shared.process_bodies.get(&target).is_none(),
        "full cleanup removed the process body slot"
    );
}
