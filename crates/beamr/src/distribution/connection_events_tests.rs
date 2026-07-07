//! Tests for the connection-event hub: dispatch mechanics (ordering,
//! reentrancy, the blocking gate), generation assignment, the
//! `register_connection` emission arms, and the `ConnectionManager` event API.

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::atom::{Atom, AtomTable};
use crate::distribution::connection::{ConnectionDownReason, ConnectionManager, DistConnection};
use crate::distribution::connection_events::{
    ConnectionEvent, ConnectionEventHub, ConnectionGeneration,
};
use crate::distribution::resolver::StaticResolver;

type EventLog = Arc<Mutex<Vec<ConnectionEvent>>>;

fn new_event_log() -> EventLog {
    Arc::new(Mutex::new(Vec::new()))
}

fn snapshot(log: &EventLog) -> Vec<ConnectionEvent> {
    log.lock().expect("event log lock").clone()
}

fn push_event(log: &EventLog, event: ConnectionEvent) {
    log.lock().expect("event log lock").push(event);
}

fn manager_named(local_name: &str) -> ConnectionManager {
    ConnectionManager::new(
        Arc::new(AtomTable::with_common_atoms()),
        Arc::new(StaticResolver::new(HashMap::new())),
        "test-cookie",
        local_name,
        1,
    )
}

/// A connected localhost socket pair: (server end for the manager, client end
/// held by the test as the "peer", the server's address).
fn socket_pair() -> (std::net::TcpStream, std::net::TcpStream, SocketAddr) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind socket pair listener");
    let addr = listener.local_addr().expect("socket pair local addr");
    let client = std::net::TcpStream::connect(addr).expect("connect socket pair client");
    let (server, _) = listener.accept().expect("accept socket pair server");
    (server, client, addr)
}

/// Install a handshake-less test connection for `name`. Returns the installed
/// connection and the peer-side socket (kept open by the caller so the read
/// loop does not immediately observe EOF). Must run inside a tokio runtime
/// context.
fn install(manager: &ConnectionManager, name: &str) -> (Arc<DistConnection>, std::net::TcpStream) {
    let node = manager.atom_table().intern(name);
    let (server, client, addr) = socket_pair();
    let connection = manager
        .register_test_connection(node, addr, server)
        .expect("register test connection");
    (connection, client)
}

fn generation(raw: u64) -> ConnectionGeneration {
    ConnectionGeneration::from_raw(raw)
}

// ---------------------------------------------------------------------------
// Hub units (no sockets)
// ---------------------------------------------------------------------------

#[test]
fn subscribers_fire_in_registration_order() {
    let hub = ConnectionEventHub::new();
    let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::clone(&order);
    hub.subscribe(move |_| first.lock().expect("order lock").push("first"));
    let second = Arc::clone(&order);
    hub.subscribe(move |_| second.lock().expect("order lock").push("second"));

    hub.enqueue(ConnectionEvent::up(Atom::OK, generation(1), 7));
    hub.dispatch();

    assert_eq!(*order.lock().expect("order lock"), vec!["first", "second"]);
}

#[test]
fn unsubscribe_returns_true_then_false_and_stops_delivery() {
    let hub = ConnectionEventHub::new();
    let log = new_event_log();
    let log_for_subscriber = Arc::clone(&log);
    let id = hub.subscribe(move |event| push_event(&log_for_subscriber, event));

    hub.enqueue(ConnectionEvent::up(Atom::OK, generation(1), 0));
    hub.dispatch();
    assert_eq!(snapshot(&log).len(), 1);

    assert!(hub.unsubscribe(id), "first unsubscribe must report removal");
    assert!(
        !hub.unsubscribe(id),
        "second unsubscribe must report unknown id"
    );

    hub.enqueue(ConnectionEvent::down(
        Atom::OK,
        generation(1),
        ConnectionDownReason::PeerClosed,
    ));
    hub.dispatch();
    assert_eq!(
        snapshot(&log).len(),
        1,
        "an unsubscribed callback must receive no further events"
    );
}

#[test]
fn legacy_slot_fires_last_down_only_with_legacy_shape() {
    let hub = ConnectionEventHub::new();
    let order: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let subscriber_order = Arc::clone(&order);
    hub.subscribe(move |event| {
        let tag = match event {
            ConnectionEvent::Up(_) => "sub:up",
            ConnectionEvent::Down(_) => "sub:down",
        };
        subscriber_order
            .lock()
            .expect("order lock")
            .push(tag.to_owned());
    });
    let legacy_order = Arc::clone(&order);
    hub.legacy_down_hook().register(move |event| {
        assert_eq!(event.node, Atom::OK, "legacy event carries the node");
        assert_eq!(
            event.reason,
            ConnectionDownReason::HeartbeatTimeout,
            "legacy event carries the reason"
        );
        legacy_order
            .lock()
            .expect("order lock")
            .push("legacy:down".to_owned());
    });

    hub.enqueue(ConnectionEvent::up(Atom::OK, generation(1), 0));
    hub.enqueue(ConnectionEvent::down(
        Atom::OK,
        generation(1),
        ConnectionDownReason::HeartbeatTimeout,
    ));
    hub.dispatch();

    assert_eq!(
        *order.lock().expect("order lock"),
        vec!["sub:up", "sub:down", "legacy:down"],
        "legacy slot fires LAST and only for Down"
    );
}

#[test]
fn reentrant_dispatch_from_callback_delivers_after_current_callback() {
    let hub = Arc::new(ConnectionEventHub::new());
    let order: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let hub_for_subscriber = Arc::clone(&hub);
    let order_for_subscriber = Arc::clone(&order);
    hub.subscribe(move |event| match event {
        ConnectionEvent::Up(_) => {
            // Trigger a nested transition from inside the callback: the nested
            // dispatch must return immediately (owner-thread reentrancy), and
            // the outer drain delivers the new event AFTER this callback.
            hub_for_subscriber.enqueue(ConnectionEvent::down(
                Atom::OK,
                generation(1),
                ConnectionDownReason::PeerClosed,
            ));
            hub_for_subscriber.dispatch();
            order_for_subscriber
                .lock()
                .expect("order lock")
                .push("up-callback-end".to_owned());
        }
        ConnectionEvent::Down(_) => {
            order_for_subscriber
                .lock()
                .expect("order lock")
                .push("down".to_owned());
        }
    });

    hub.enqueue(ConnectionEvent::up(Atom::OK, generation(1), 0));
    hub.dispatch();

    // The nested event was delivered after the enqueuing callback returned,
    // before the outermost dispatch returned — and with no deadlock.
    assert_eq!(
        *order.lock().expect("order lock"),
        vec!["up-callback-end".to_owned(), "down".to_owned()]
    );
}

#[test]
fn subscriber_may_unsubscribe_itself_mid_callback() {
    let hub = Arc::new(ConnectionEventHub::new());
    let log = new_event_log();

    let hub_for_subscriber = Arc::clone(&hub);
    let log_for_subscriber = Arc::clone(&log);
    let id_slot: Arc<Mutex<Option<crate::distribution::connection_events::SubscriberId>>> =
        Arc::new(Mutex::new(None));
    let id_for_subscriber = Arc::clone(&id_slot);
    let id = hub.subscribe(move |event| {
        push_event(&log_for_subscriber, event);
        if let Some(id) = *id_for_subscriber.lock().expect("id lock") {
            assert!(
                hub_for_subscriber.unsubscribe(id),
                "self-unsubscribe from inside a callback must succeed"
            );
        }
    });
    *id_slot.lock().expect("id lock") = Some(id);

    hub.enqueue(ConnectionEvent::up(Atom::OK, generation(1), 0));
    hub.dispatch();
    hub.enqueue(ConnectionEvent::up(Atom::OK, generation(2), 0));
    hub.dispatch();

    assert_eq!(
        snapshot(&log).len(),
        1,
        "a self-unsubscribed callback must not see later events"
    );
}

#[test]
fn panicking_subscriber_does_not_wedge_dispatch() {
    let hub = Arc::new(ConnectionEventHub::new());
    let log = new_event_log();

    // Recorder first, so it observes the event even though the panicker
    // aborts the rest of that event's chain.
    let log_for_recorder = Arc::clone(&log);
    hub.subscribe(move |event| push_event(&log_for_recorder, event));
    let panicked = Arc::new(AtomicBool::new(false));
    let panicked_for_subscriber = Arc::clone(&panicked);
    hub.subscribe(move |_| {
        if !panicked_for_subscriber.swap(true, Ordering::SeqCst) {
            panic!("test-only subscriber panic");
        }
    });

    hub.enqueue(ConnectionEvent::up(Atom::OK, generation(1), 0));
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| hub.dispatch()));
    assert!(outcome.is_err(), "the subscriber panic must propagate");

    // The gate mutex is now poisoned and the panic unwound mid-drain. A later
    // event on the SAME thread must still be delivered: poisoned-lock recovery
    // plus the RAII owner reset (a stale owner record would make this dispatch
    // return immediately without draining).
    hub.enqueue(ConnectionEvent::up(Atom::OK, generation(2), 0));
    hub.dispatch();

    let events = snapshot(&log);
    assert_eq!(
        events.len(),
        2,
        "events after a panicked pass still deliver"
    );
    assert_eq!(events[1].generation(), generation(2));
}

#[test]
fn generations_start_at_one_and_increase_per_peer_independently() {
    let hub = ConnectionEventHub::new();
    assert_eq!(hub.last_generation(Atom::OK), None);
    assert_eq!(hub.next_generation(Atom::OK), generation(1));
    assert_eq!(hub.next_generation(Atom::OK), generation(2));
    assert_eq!(hub.next_generation(Atom::ERROR), generation(1));
    assert_eq!(hub.last_generation(Atom::OK), Some(generation(2)));
    assert_eq!(hub.last_generation(Atom::ERROR), Some(generation(1)));
}

// ---------------------------------------------------------------------------
// Manager-level: emission arms, generations, visibility
// ---------------------------------------------------------------------------

#[tokio::test]
async fn generations_strictly_increase_across_redial() {
    let manager = manager_named("local@127.0.0.1");
    let node = manager.atom_table().intern("peer@127.0.0.1");
    let log = new_event_log();
    let log_for_subscriber = Arc::clone(&log);
    manager.subscribe_connection_events(move |event| push_event(&log_for_subscriber, event));

    let (first, _peer_first) = install(&manager, "peer@127.0.0.1");
    assert_eq!(first.generation(), generation(1));
    assert!(manager.disconnect_node(node));
    let (second, _peer_second) = install(&manager, "peer@127.0.0.1");
    assert_eq!(second.generation(), generation(2));
    assert!(second.generation() > first.generation());
    assert_eq!(manager.last_peer_generation(node), Some(generation(2)));

    let events = snapshot(&log);
    assert_eq!(
        events,
        vec![
            ConnectionEvent::up(node, generation(1), 0),
            ConnectionEvent::down(node, generation(1), ConnectionDownReason::ManualDisconnect),
            ConnectionEvent::up(node, generation(2), 0),
        ],
        "per-node delivery is Up(g1) Down(g1) Up(g2) with strictly increasing generations"
    );
}

#[tokio::test]
async fn hs4_down_but_unreaped_incumbent_emits_down_then_up_exactly_once() {
    let manager = manager_named("local@127.0.0.1");
    let node = manager.atom_table().intern("peer@127.0.0.1");

    let (first, _peer_first) = install(&manager, "peer@127.0.0.1");
    let log = new_event_log();
    let log_for_subscriber = Arc::clone(&log);
    manager.subscribe_connection_events(move |event| push_event(&log_for_subscriber, event));

    // The HS-4 window: down flag flipped, entry NOT reaped. Pre-hub, the
    // replaced incumbent's Down notification was lost forever.
    first.force_down_without_reap();
    assert!(
        manager.get_connection(node).is_some(),
        "the down entry must still be in the table for the HS-4 arm"
    );

    let (second, _peer_second) = install(&manager, "peer@127.0.0.1");

    let events = snapshot(&log);
    assert_eq!(
        events,
        vec![
            // Reason is the documented ReadError fallback: the test flipped the
            // flag directly, so no reason was recorded by mark_down.
            ConnectionEvent::down(node, generation(1), ConnectionDownReason::ReadError),
            ConnectionEvent::up(node, generation(2), 0),
        ],
        "the replaced down incumbent's session closes exactly once, before the new Up"
    );
    assert_eq!(second.generation(), generation(2));
    assert_eq!(manager.connection_count(), 1);
}

#[tokio::test]
async fn displacement_inherits_generation_and_emits_no_events() {
    // local > alpha, so the canonical direction for the pair is Outbound and
    // the Inbound incumbent installed by the test helper is non-canonical:
    // a second install displaces it while it is still live.
    let manager = manager_named("local@127.0.0.1");
    let node = manager.atom_table().intern("alpha@127.0.0.1");

    let (first, _peer_first) = install(&manager, "alpha@127.0.0.1");
    assert!(!first.is_down());
    let log = new_event_log();
    let log_for_subscriber = Arc::clone(&log);
    manager.subscribe_connection_events(move |event| push_event(&log_for_subscriber, event));

    let (second, _peer_second) = install(&manager, "alpha@127.0.0.1");

    assert!(
        !Arc::ptr_eq(&first, &second),
        "the newcomer must displace the live non-canonical incumbent"
    );
    assert!(first.is_down(), "the displaced socket is retired");
    assert_eq!(
        second.generation(),
        first.generation(),
        "displacement continues the same logical session: the generation is inherited"
    );
    assert!(
        snapshot(&log).is_empty(),
        "socket displacement within one session emits no events"
    );
    assert_eq!(manager.connection_count(), 1);
    let survivor = manager
        .get_connection(node)
        .expect("survivor stays installed");
    assert!(Arc::ptr_eq(&survivor, &second));
    assert_eq!(survivor.generation(), second.generation());
}

/// A restarted peer whose re-dial lands while the stale incumbent still looks
/// live (no FIN reached us; heartbeat deadline not yet hit) is a session
/// boundary, not a socket swap: the differing handshake `peer_creation` is
/// the discriminator, and swallowing it would leave the dead incarnation's
/// cleanup (pg purge, noconnection delivery) unfired forever.
#[tokio::test]
async fn live_displacement_by_new_peer_creation_emits_down_then_up() {
    // local > alpha, so the Inbound incumbent installed by the test helper is
    // non-canonical: displaceable while still live.
    let manager = manager_named("local@127.0.0.1");
    let node = manager.atom_table().intern("alpha@127.0.0.1");

    let (server_first, _peer_first, addr_first) = socket_pair();
    let first = manager
        .register_test_connection_with_creation(node, addr_first, server_first, 41)
        .expect("install first incarnation");
    assert!(!first.is_down(), "the stale incumbent still looks live");
    let log = new_event_log();
    let log_for_subscriber = Arc::clone(&log);
    manager.subscribe_connection_events(move |event| push_event(&log_for_subscriber, event));

    let (server_second, _peer_second, addr_second) = socket_pair();
    let second = manager
        .register_test_connection_with_creation(node, addr_second, server_second, 42)
        .expect("install restarted incarnation");

    assert_eq!(
        snapshot(&log),
        vec![
            // The stale link never reported a failure, so the closed session
            // carries the same ReadError the displaced socket is retired with.
            ConnectionEvent::down(node, generation(1), ConnectionDownReason::ReadError),
            ConnectionEvent::up(node, generation(2), 42),
        ],
        "a changed peer_creation across a live displacement is a peer bounce: \
         Down(g_old) then Up(g_new), exactly once each"
    );
    assert!(first.is_down(), "the displaced stale socket is retired");
    assert_eq!(
        second.generation(),
        generation(2),
        "the restarted incarnation opens a NEW session"
    );
    assert_eq!(second.peer_creation(), 42);
    assert_eq!(manager.connection_count(), 1);
    assert_eq!(manager.last_peer_generation(node), Some(generation(2)));
}

/// The complement: a live displacement by the SAME nonzero incarnation (a
/// simultaneous-connect socket swap) stays invisible — generation inherited,
/// zero events — so the bounce discriminator fires only on a real restart.
#[tokio::test]
async fn live_displacement_same_nonzero_peer_creation_inherits_and_emits_nothing() {
    let manager = manager_named("local@127.0.0.1");
    let node = manager.atom_table().intern("alpha@127.0.0.1");

    let (server_first, _peer_first, addr_first) = socket_pair();
    let first = manager
        .register_test_connection_with_creation(node, addr_first, server_first, 41)
        .expect("install first socket");
    let log = new_event_log();
    let log_for_subscriber = Arc::clone(&log);
    manager.subscribe_connection_events(move |event| push_event(&log_for_subscriber, event));

    let (server_second, _peer_second, addr_second) = socket_pair();
    let second = manager
        .register_test_connection_with_creation(node, addr_second, server_second, 41)
        .expect("install same-incarnation socket");

    assert!(
        snapshot(&log).is_empty(),
        "same-incarnation socket displacement is one logical session: no events"
    );
    assert_eq!(second.generation(), first.generation());
    assert_eq!(manager.last_peer_generation(node), Some(generation(1)));
}

#[tokio::test]
async fn inv_up_visibility_up_callback_observes_installed_connection() {
    let manager = manager_named("local@127.0.0.1");
    let observed: Arc<Mutex<Option<Arc<DistConnection>>>> = Arc::new(Mutex::new(None));

    let manager_for_subscriber = manager.clone();
    let observed_for_subscriber = Arc::clone(&observed);
    manager.subscribe_connection_events(move |event| {
        if let ConnectionEvent::Up(up) = event {
            let connection = manager_for_subscriber.get_connection(up.node);
            let connection = connection.expect("Up(g) callback must observe the g connection");
            assert_eq!(connection.generation(), up.generation);
            assert_eq!(connection.peer_creation(), up.peer_creation);
            *observed_for_subscriber.lock().expect("observed lock") = Some(connection);
        }
    });

    let (installed, mut peer) = install(&manager, "peer@127.0.0.1");
    let observed = observed
        .lock()
        .expect("observed lock")
        .take()
        .expect("the Up callback ran and observed a connection");
    assert!(Arc::ptr_eq(&observed, &installed));

    // The handle obtained INSIDE the callback is usable: a frame written on it
    // reaches the peer. (The write itself happens after dispatch, per
    // INV-SUB-DISCIPLINE — callbacks must not do socket I/O.)
    observed
        .write_raw(&[0_u8; 8])
        .await
        .expect("connection observed in the Up callback must be writable");
    peer.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set peer read timeout");
    let mut header = [0_u8; 8];
    peer.read_exact(&mut header)
        .expect("peer must receive the frame written on the observed handle");
}

#[tokio::test]
async fn inv_down_visibility_down_callback_never_observes_closed_generation() {
    let manager = manager_named("local@127.0.0.1");
    let manager_for_subscriber = manager.clone();
    let violations: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let violations_for_subscriber = Arc::clone(&violations);
    manager.subscribe_connection_events(move |event| {
        if let ConnectionEvent::Down(down) = event
            && let Some(connection) = manager_for_subscriber.get_connection(down.node)
            && connection.generation() == down.generation
        {
            violations_for_subscriber
                .lock()
                .expect("violations lock")
                .push(format!(
                    "Down({}) callback observed the closed generation",
                    down.generation.get()
                ));
        }
    });

    let node = manager.atom_table().intern("peer@127.0.0.1");
    for _ in 0..10 {
        let (_connection, _peer) = install(&manager, "peer@127.0.0.1");
        assert!(manager.disconnect_node(node));
    }
    assert_eq!(
        *violations.lock().expect("violations lock"),
        Vec::<String>::new()
    );
}

/// INV-DOWN-VISIBILITY under concurrent dispatch (pins the shard-lock
/// dependency): while one thread performs the removal, a subscriber running on
/// whichever thread holds the dispatch gate must never observe the closed
/// generation still installed.
#[test]
fn inv_down_visibility_holds_under_concurrent_dispatch() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build test runtime");
    let manager = manager_named("local@127.0.0.1");
    manager.set_runtime_handle(runtime.handle().clone());
    let _context = runtime.enter();

    let manager_for_subscriber = manager.clone();
    let violations: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let violations_for_subscriber = Arc::clone(&violations);
    manager.subscribe_connection_events(move |event| {
        if let ConnectionEvent::Down(down) = event
            && let Some(connection) = manager_for_subscriber.get_connection(down.node)
            && connection.generation() == down.generation
        {
            violations_for_subscriber
                .lock()
                .expect("violations lock")
                .push(format!(
                    "Down({}) observed generation {} still installed",
                    down.node.index(),
                    down.generation.get()
                ));
        }
    });

    let node_x = manager.atom_table().intern("xnode@127.0.0.1");
    let node_y = manager.atom_table().intern("ynode@127.0.0.1");
    for _ in 0..25 {
        let (_connection_x, _peer_x) = install(&manager, "xnode@127.0.0.1");
        let (_connection_y, _peer_y) = install(&manager, "ynode@127.0.0.1");
        let manager_x = manager.clone();
        let manager_y = manager.clone();
        let disconnect_x = std::thread::spawn(move || manager_x.disconnect_node(node_x));
        let disconnect_y = std::thread::spawn(move || manager_y.disconnect_node(node_y));
        assert!(disconnect_x.join().expect("x disconnect thread"));
        assert!(disconnect_y.join().expect("y disconnect thread"));
    }
    assert_eq!(
        *violations.lock().expect("violations lock"),
        Vec::<String>::new()
    );
}

// ---------------------------------------------------------------------------
// INV-SYNC and the blocking gate
// ---------------------------------------------------------------------------

/// The blocking gate makes delivery synchronous even under contention: thread
/// B's `disconnect_node` must not return until B's Down was delivered — even
/// though thread A holds the dispatch gate in a deliberately slow subscriber,
/// so it is A that ends up delivering B's event. No `eventually` polling.
#[test]
fn inv_sync_disconnect_return_implies_delivery_under_contention() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build test runtime");
    let manager = manager_named("local@127.0.0.1");
    manager.set_runtime_handle(runtime.handle().clone());
    let _context = runtime.enter();

    let node_x = manager.atom_table().intern("xnode@127.0.0.1");
    let node_y = manager.atom_table().intern("ynode@127.0.0.1");
    let (_connection_x, _peer_x) = install(&manager, "xnode@127.0.0.1");
    let (_connection_y, _peer_y) = install(&manager, "ynode@127.0.0.1");

    let log = new_event_log();
    let log_for_subscriber = Arc::clone(&log);
    let slow_entered = Arc::new(AtomicBool::new(false));
    let slow_entered_for_subscriber = Arc::clone(&slow_entered);
    manager.subscribe_connection_events(move |event| {
        if let ConnectionEvent::Down(down) = event
            && down.node == node_x
        {
            // Deliberately slow subscriber holding the dispatch gate.
            slow_entered_for_subscriber.store(true, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(300));
        }
        push_event(&log_for_subscriber, event);
    });

    let manager_for_x = manager.clone();
    let downer = std::thread::spawn(move || manager_for_x.disconnect_node(node_x));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !slow_entered.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "slow subscriber never entered");
        std::thread::sleep(Duration::from_millis(5));
    }

    // Thread A (downer) is inside the slow callback holding the gate. This
    // call must BLOCK until Down(y) has been delivered (by A's drain loop).
    assert!(manager.disconnect_node(node_y));
    let delivered: Vec<_> = snapshot(&log)
        .iter()
        .filter_map(|event| match event {
            ConnectionEvent::Down(down) => Some(down.node),
            ConnectionEvent::Up(_) => None,
        })
        .collect();
    assert!(
        delivered.contains(&node_y),
        "disconnect_node returned before its Down was delivered (INV-SYNC broken): {delivered:?}"
    );
    assert!(downer.join().expect("downer thread"));
}

#[tokio::test]
async fn callback_may_disconnect_and_subscribe_reentrantly_on_same_manager() {
    let manager = manager_named("local@127.0.0.1");
    let node_x = manager.atom_table().intern("xnode@127.0.0.1");
    let node_y = manager.atom_table().intern("ynode@127.0.0.1");
    let (_connection_x, _peer_x) = install(&manager, "xnode@127.0.0.1");
    let (_connection_y, _peer_y) = install(&manager, "ynode@127.0.0.1");

    let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    let late_log = new_event_log();
    let manager_for_subscriber = manager.clone();
    let order_for_subscriber = Arc::clone(&order);
    let late_log_for_subscriber = Arc::clone(&late_log);
    manager.subscribe_connection_events(move |event| {
        let ConnectionEvent::Down(down) = event else {
            return;
        };
        if down.node == node_x {
            order_for_subscriber
                .lock()
                .expect("order lock")
                .push("x-down-start");
            // Reentrant transition: enqueued now, delivered AFTER this
            // callback returns (owner-thread reentrancy; no deadlock).
            assert!(manager_for_subscriber.disconnect_node(node_y));
            // Mid-dispatch subscribe: must not see the in-flight Down(x)
            // (snapshot semantics) but must see the pending Down(y).
            let late_log_for_late = Arc::clone(&late_log_for_subscriber);
            manager_for_subscriber
                .subscribe_connection_events(move |event| push_event(&late_log_for_late, event));
            order_for_subscriber
                .lock()
                .expect("order lock")
                .push("x-down-end");
        } else {
            order_for_subscriber
                .lock()
                .expect("order lock")
                .push("y-down");
        }
    });

    assert!(manager.disconnect_node(node_x));

    // INV-SYNC: by the time disconnect_node(x) returned, BOTH downs (including
    // the one triggered from inside the callback) were delivered, in order.
    assert_eq!(
        *order.lock().expect("order lock"),
        vec!["x-down-start", "x-down-end", "y-down"]
    );
    let late_events = snapshot(&late_log);
    assert_eq!(
        late_events,
        vec![ConnectionEvent::down(
            node_y,
            generation(1),
            ConnectionDownReason::ManualDisconnect
        )],
        "a mid-dispatch subscriber misses the in-flight event but sees the next one"
    );
}

// ---------------------------------------------------------------------------
// Exactly-once-per-generation stress
// ---------------------------------------------------------------------------

#[test]
fn exactly_once_per_generation_under_racing_downs_and_redials() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build test runtime");
    // zeta > local: the canonical direction is Inbound, so a re-dial racing a
    // still-live incumbent benignly loses instead of displacing (keeps the
    // stress on the down/redial race, not the displacement path).
    let manager = manager_named("local@127.0.0.1");
    manager.set_runtime_handle(runtime.handle().clone());
    let _context = runtime.enter();
    let node = manager.atom_table().intern("zeta@127.0.0.1");

    let log = new_event_log();
    let log_for_subscriber = Arc::clone(&log);
    manager.subscribe_connection_events(move |event| push_event(&log_for_subscriber, event));

    let mut peers = Vec::new();
    let mut previous: Option<Arc<DistConnection>> = None;
    for _ in 0..40 {
        let mut racers = Vec::new();
        if let Some(connection) = previous.take() {
            // Two racing down paths (mixed reasons) against the re-dial below.
            let for_write_timeout = Arc::clone(&connection);
            racers.push(std::thread::spawn(move || {
                for_write_timeout.mark_down_write_timeout();
            }));
            let manager_for_disconnect = manager.clone();
            racers.push(std::thread::spawn(move || {
                manager_for_disconnect.disconnect_node(node);
            }));
        }
        let (server, client, addr) = socket_pair();
        let installed = manager
            .register_test_connection(node, addr, server)
            .expect("racing re-dial install");
        peers.push(client);
        for racer in racers {
            racer.join().expect("racer thread must not panic");
        }
        previous = Some(installed);
    }
    // Close the final session so every opened generation also closed.
    manager.disconnect_node(node);

    // One global order, alternating Up(g)/Down(g) with strictly increasing
    // generations and exactly one Up and one Down per generation.
    let events = snapshot(&log);
    assert!(!events.is_empty());
    let mut open: Option<ConnectionGeneration> = None;
    let mut last_closed: Option<ConnectionGeneration> = None;
    for event in &events {
        match event {
            ConnectionEvent::Up(up) => {
                assert_eq!(
                    open,
                    None,
                    "Up({}) delivered while generation {open:?} still open",
                    up.generation.get()
                );
                if let Some(closed) = last_closed {
                    assert!(
                        up.generation > closed,
                        "generations must strictly increase: Up({}) after Down({})",
                        up.generation.get(),
                        closed.get()
                    );
                }
                open = Some(up.generation);
            }
            ConnectionEvent::Down(down) => {
                assert_eq!(
                    open,
                    Some(down.generation),
                    "Down({}) must close the currently open generation {open:?}",
                    down.generation.get()
                );
                last_closed = Some(down.generation);
                open = None;
            }
        }
    }
    assert_eq!(open, None, "every opened generation must have closed");
    let ups = events
        .iter()
        .filter(|event| matches!(event, ConnectionEvent::Up(_)))
        .count();
    let downs = events.len() - ups;
    assert_eq!(ups, downs, "exactly one Down per Up");
    assert_eq!(
        manager
            .last_peer_generation(node)
            .map(ConnectionGeneration::get),
        Some(ups as u64),
        "generations are assigned densely from 1"
    );
}

// ---------------------------------------------------------------------------
// R2 regression + legacy compatibility
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r2_legacy_registration_no_longer_evicts_hub_subscribers() {
    let manager = manager_named("local@127.0.0.1");
    let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

    // Hub subscriber registered first (as the scheduler's pg-purge is, at
    // construction). Pre-hub, the pg purge lived in the legacy slot and any
    // later legacy registrant REPLACED it silently.
    let hub_order = Arc::clone(&order);
    manager.subscribe_connection_events(move |event| {
        if matches!(event, ConnectionEvent::Down(_)) {
            hub_order.lock().expect("order lock").push("hub");
        }
    });
    let first_legacy_order = Arc::clone(&order);
    manager.register_connection_down(move |_| {
        first_legacy_order
            .lock()
            .expect("order lock")
            .push("legacy-first");
    });
    // Replace-on-register semantics of the legacy slot are retained…
    let second_legacy_order = Arc::clone(&order);
    manager.register_connection_down(move |_| {
        second_legacy_order
            .lock()
            .expect("order lock")
            .push("legacy-second");
    });

    let node = manager.atom_table().intern("peer@127.0.0.1");
    let (_connection, _peer) = install(&manager, "peer@127.0.0.1");
    assert!(manager.disconnect_node(node));

    // …but the hub subscriber can no longer be evicted by a legacy registrant,
    // and the legacy slot fires last.
    assert_eq!(
        *order.lock().expect("order lock"),
        vec!["hub", "legacy-second"]
    );
}

// ---------------------------------------------------------------------------
// connected_peers / last_peer_generation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn connected_peers_excludes_down_links_and_last_generation_survives_them() {
    let manager = manager_named("local@127.0.0.1");
    let node_a = manager.atom_table().intern("anode@127.0.0.1");
    let node_b = manager.atom_table().intern("bnode@127.0.0.1");
    let unknown = manager.atom_table().intern("never@127.0.0.1");

    let (connection_a, _peer_a) = install(&manager, "anode@127.0.0.1");
    let (_connection_b, _peer_b) = install(&manager, "bnode@127.0.0.1");

    let mut rows = manager.connected_peers();
    rows.sort_by_key(|row| row.node.index());
    let mut expected_nodes = vec![node_a, node_b];
    expected_nodes.sort_by_key(|node| node.index());
    assert_eq!(
        rows.iter().map(|row| row.node).collect::<Vec<_>>(),
        expected_nodes
    );
    for row in &rows {
        assert_eq!(row.generation, generation(1));
        assert_eq!(row.peer_creation, 0);
    }

    // A down-but-unreaped link is not a connected peer, but its generation
    // history remains queryable.
    connection_a.force_down_without_reap();
    let rows = manager.connected_peers();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].node, node_b);
    assert_eq!(manager.last_peer_generation(node_a), Some(generation(1)));
    assert_eq!(manager.last_peer_generation(unknown), None);
}

/// Late-subscriber reconciliation recipe: subscribe FIRST, then snapshot, then
/// per peer keep the highest generation — the generation is the dedupe key, so
/// a session reported by both the snapshot and an event is never counted twice.
#[tokio::test]
async fn late_subscriber_recipe_reconciles_without_double_counting() {
    let manager = manager_named("local@127.0.0.1");
    let node = manager.atom_table().intern("peer@127.0.0.1");
    let (_connection, _peer) = install(&manager, "peer@127.0.0.1");

    // Late consumer: subscribe first…
    let log = new_event_log();
    let log_for_subscriber = Arc::clone(&log);
    manager.subscribe_connection_events(move |event| push_event(&log_for_subscriber, event));
    // …then snapshot.
    let rows = manager.connected_peers();

    // A transition racing the reconciliation shows up as events.
    assert!(manager.disconnect_node(node));
    let (_second, _peer_second) = install(&manager, "peer@127.0.0.1");

    // Reconcile: max generation per peer across snapshot rows and Up events.
    let mut in_force: HashMap<Atom, ConnectionGeneration> = HashMap::new();
    for row in rows {
        let slot = in_force.entry(row.node).or_insert(row.generation);
        *slot = (*slot).max(row.generation);
    }
    for event in snapshot(&log) {
        match event {
            ConnectionEvent::Up(up) => {
                let slot = in_force.entry(up.node).or_insert(up.generation);
                *slot = (*slot).max(up.generation);
            }
            ConnectionEvent::Down(down) => {
                if in_force.get(&down.node) == Some(&down.generation) {
                    in_force.remove(&down.node);
                }
            }
        }
    }
    assert_eq!(in_force.len(), 1, "one peer, counted exactly once");
    assert_eq!(in_force.get(&node), Some(&generation(2)));
    assert_eq!(manager.last_peer_generation(node), Some(generation(2)));
}

// ---------------------------------------------------------------------------
// INV-FRAME-ORDER
// ---------------------------------------------------------------------------

/// No inbound frame from the generation-g socket reaches the control-frame
/// handler before Up(g) delivery completes: the frame bytes are already
/// buffered in the socket BEFORE the install, and the Up subscriber dawdles —
/// if the read loop were spawned before dispatch, "frame" would win the race.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv_frame_order_up_delivered_before_first_inbound_frame() {
    let manager = manager_named("local@127.0.0.1");
    let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

    let frame_order = Arc::clone(&order);
    manager.register_control_frame_handler(move |_control, _payload| {
        frame_order.lock().expect("order lock").push("frame");
    });
    let up_order = Arc::clone(&order);
    manager.subscribe_connection_events(move |event| {
        if matches!(event, ConnectionEvent::Up(_)) {
            // Widen the race window: a prematurely spawned read loop would
            // deliver the pre-buffered frame while we sleep.
            std::thread::sleep(Duration::from_millis(100));
            up_order.lock().expect("order lock").push("up");
        }
    });

    let node = manager.atom_table().intern("peer@127.0.0.1");
    let (server, mut client, addr) = socket_pair();
    // Pre-buffer a data frame (control_len = 2, payload_len = 0, 2 bytes of
    // control) so it is readable the instant the read loop exists.
    client
        .write_all(&[0, 0, 0, 2, 0, 0, 0, 0, 1, 2])
        .expect("pre-buffer inbound frame");
    let _connection = manager
        .register_test_connection(node, addr, server)
        .expect("install with pre-buffered frame");

    let deadline = Instant::now() + Duration::from_secs(10);
    while !order.lock().expect("order lock").contains(&"frame") {
        assert!(
            Instant::now() < deadline,
            "the pre-buffered frame was never delivered"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        *order.lock().expect("order lock"),
        vec!["up", "frame"],
        "Up(g) delivery must complete before the first generation-g frame"
    );
}

// ---------------------------------------------------------------------------
// Canonical-arm peer-bounce boundary
// ---------------------------------------------------------------------------

/// Mirror of the displacement-arm bounce boundary: even when the LIVE
/// incumbent holds the canonical direction (the arm where newcomers lose), a
/// nonzero-creation mismatch proves it a stale incarnation — the peer
/// restarted — so the newcomer installs as a session boundary: Down(g_old)
/// then Up(g_new), and the stale socket is retired.
#[tokio::test]
async fn canonical_incumbent_displaced_by_new_peer_creation_emits_down_then_up() {
    // peer > local, so the canonical direction is Inbound and the Inbound
    // incumbent installed by the test helper is live AND canonical.
    let manager = manager_named("local@127.0.0.1");
    let node = manager.atom_table().intern("peer@127.0.0.1");

    let (server_first, _peer_first, addr_first) = socket_pair();
    let first = manager
        .register_test_connection_with_creation(node, addr_first, server_first, 41)
        .expect("install first incarnation");
    assert!(!first.is_down(), "the stale incumbent still looks live");
    let log = new_event_log();
    let log_for_subscriber = Arc::clone(&log);
    manager.subscribe_connection_events(move |event| push_event(&log_for_subscriber, event));

    let (server_second, _peer_second, addr_second) = socket_pair();
    let second = manager
        .register_test_connection_with_creation(node, addr_second, server_second, 42)
        .expect("install restarted incarnation");

    assert!(
        !Arc::ptr_eq(&first, &second),
        "the restarted peer's dial must displace the live canonical incumbent"
    );
    assert_eq!(
        snapshot(&log),
        vec![
            // The stale link never reported a failure, so the closed session
            // carries the same ReadError the displaced socket is retired with.
            ConnectionEvent::down(node, generation(1), ConnectionDownReason::ReadError),
            ConnectionEvent::up(node, generation(2), 42),
        ],
        "a creation mismatch is a session boundary even against a canonical incumbent"
    );
    assert!(first.is_down(), "the displaced stale socket is retired");
    assert_eq!(second.generation(), generation(2));
    assert_eq!(second.peer_creation(), 42);
    assert_eq!(manager.connection_count(), 1);
    let survivor = manager
        .get_connection(node)
        .expect("survivor stays installed");
    assert!(Arc::ptr_eq(&survivor, &second));
}

/// The complement: same-creation and sentinel-zero newcomers keep losing to a
/// live canonical incumbent exactly as before — no displacement, no events —
/// and a sentinel-zero INCUMBENT is equally non-discriminating.
#[tokio::test]
async fn canonical_incumbent_still_wins_against_same_or_sentinel_creation() {
    let manager = manager_named("local@127.0.0.1");
    let node = manager.atom_table().intern("peer@127.0.0.1");

    let (server_first, _peer_first, addr_first) = socket_pair();
    let first = manager
        .register_test_connection_with_creation(node, addr_first, server_first, 41)
        .expect("install canonical incumbent");
    let log = new_event_log();
    let log_for_subscriber = Arc::clone(&log);
    manager.subscribe_connection_events(move |event| push_event(&log_for_subscriber, event));

    // Same nonzero incarnation: a simultaneous-connect echo, not a bounce.
    let (server_same, _peer_same, addr_same) = socket_pair();
    let same = manager
        .register_test_connection_with_creation(node, addr_same, server_same, 41)
        .expect("register same-creation newcomer");
    assert!(Arc::ptr_eq(&same, &first), "same-creation newcomer loses");

    // Sentinel-zero newcomer: 0 never discriminates.
    let (server_zero, _peer_zero, addr_zero) = socket_pair();
    let zero = manager
        .register_test_connection_with_creation(node, addr_zero, server_zero, 0)
        .expect("register sentinel-creation newcomer");
    assert!(Arc::ptr_eq(&zero, &first), "sentinel-zero newcomer loses");

    assert!(
        !first.is_down(),
        "the canonical incumbent survives untouched"
    );
    assert!(snapshot(&log).is_empty(), "losing newcomers emit no events");
    assert_eq!(first.generation(), generation(1));

    // Sentinel-zero incumbent, nonzero newcomer: still no discriminator, so
    // the live canonical incumbent keeps winning.
    let node_other = manager.atom_table().intern("other@127.0.0.1");
    let (server_other, _peer_other, addr_other) = socket_pair();
    let sentinel_incumbent = manager
        .register_test_connection_with_creation(node_other, addr_other, server_other, 0)
        .expect("install sentinel-creation incumbent");
    let (server_late, _peer_late, addr_late) = socket_pair();
    let late = manager
        .register_test_connection_with_creation(node_other, addr_late, server_late, 42)
        .expect("register nonzero newcomer against sentinel incumbent");
    assert!(
        Arc::ptr_eq(&late, &sentinel_incumbent),
        "a nonzero newcomer cannot bounce a sentinel-creation incumbent"
    );
    assert!(!sentinel_incumbent.is_down());
    assert_eq!(manager.connection_count(), 2);
    assert_eq!(
        snapshot(&log),
        vec![ConnectionEvent::up(node_other, generation(1), 0)],
        "only the sentinel incumbent's own install emitted an event"
    );
}

// ---------------------------------------------------------------------------
// subscribe_connection_events_with_snapshot
// ---------------------------------------------------------------------------

#[tokio::test]
async fn snapshot_subscribe_delivers_synthetic_ups_before_any_subsequent_real_event() {
    let manager = manager_named("local@127.0.0.1");
    let node_a = manager.atom_table().intern("anode@127.0.0.1");
    let node_b = manager.atom_table().intern("bnode@127.0.0.1");
    let (_connection_a, _peer_a) = install(&manager, "anode@127.0.0.1");
    let (_connection_b, _peer_b) = install(&manager, "bnode@127.0.0.1");

    let log = new_event_log();
    let log_for_subscriber = Arc::clone(&log);
    let id = manager.subscribe_connection_events_with_snapshot(move |event| {
        push_event(&log_for_subscriber, event)
    });

    // The synthetic catch-up was delivered synchronously, before the
    // subscribe call returned.
    let catch_up = snapshot(&log);
    assert_eq!(catch_up.len(), 2, "one synthetic Up per live peer");
    let mut nodes: Vec<Atom> = catch_up.iter().map(ConnectionEvent::node).collect();
    nodes.sort_by_key(|node| node.index());
    let mut expected = vec![node_a, node_b];
    expected.sort_by_key(|node| node.index());
    assert_eq!(nodes, expected);
    for event in &catch_up {
        assert_eq!(
            *event,
            ConnectionEvent::up(event.node(), generation(1), 0),
            "each catch-up row is the peer's in-force NodeUp"
        );
    }

    assert!(manager.disconnect_node(node_a));
    let events = snapshot(&log);
    assert_eq!(events.len(), 3);
    assert_eq!(
        events[2],
        ConnectionEvent::down(
            node_a,
            generation(1),
            ConnectionDownReason::ManualDisconnect
        ),
        "real events land strictly after the synthetic catch-up"
    );
    assert!(
        manager.unsubscribe_connection_events(id),
        "the returned id identifies a live subscription"
    );
}

#[tokio::test]
async fn synthetic_catch_up_is_invisible_to_other_subscribers() {
    let manager = manager_named("local@127.0.0.1");
    let node = manager.atom_table().intern("peer@127.0.0.1");

    let early_log = new_event_log();
    let early_for_subscriber = Arc::clone(&early_log);
    manager.subscribe_connection_events(move |event| push_event(&early_for_subscriber, event));
    let (_connection, _peer) = install(&manager, "peer@127.0.0.1");
    assert_eq!(
        snapshot(&early_log),
        vec![ConnectionEvent::up(node, generation(1), 0)]
    );

    let late_log = new_event_log();
    let late_for_subscriber = Arc::clone(&late_log);
    manager.subscribe_connection_events_with_snapshot(move |event| {
        push_event(&late_for_subscriber, event)
    });

    assert_eq!(
        snapshot(&late_log),
        vec![ConnectionEvent::up(node, generation(1), 0)],
        "the late subscriber catches up on the in-force session"
    );
    assert_eq!(
        snapshot(&early_log),
        vec![ConnectionEvent::up(node, generation(1), 0)],
        "the synthetic Up is subscriber-local: nothing is replayed to others"
    );

    assert!(manager.disconnect_node(node));
    assert_eq!(snapshot(&early_log).len(), 2);
    assert_eq!(snapshot(&late_log).len(), 2);
}

/// A subscriber registering via the snapshot path DURING active churn misses
/// no session and double-sees none: its per-node stream starts at an Up
/// (synthetic or real), alternates Up/Down, and — because generations are
/// assigned densely — covers every generation from the first seen through the
/// last (a gap would be a missed session, a repeat a double-see).
#[test]
fn snapshot_subscriber_during_churn_misses_nothing_and_double_sees_nothing() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build test runtime");
    let manager = manager_named("local@127.0.0.1");
    manager.set_runtime_handle(runtime.handle().clone());
    let _context = runtime.enter();
    let node = manager.atom_table().intern("zeta@127.0.0.1");

    let barrier = Arc::new(std::sync::Barrier::new(2));
    let churn = {
        let manager = manager.clone();
        let barrier = Arc::clone(&barrier);
        let handle = runtime.handle().clone();
        std::thread::spawn(move || {
            let _context = handle.enter();
            // Keep peer sockets alive so EOF downs do not race the manual
            // disconnects.
            let mut peers = Vec::new();
            for iteration in 0..60 {
                if iteration == 10 {
                    // Release the main thread to subscribe mid-churn.
                    barrier.wait();
                }
                let (server, client, addr) = socket_pair();
                manager
                    .register_test_connection(node, addr, server)
                    .expect("churn install");
                peers.push(client);
                manager.disconnect_node(node);
            }
            peers
        })
    };

    barrier.wait();
    let log = new_event_log();
    let log_for_subscriber = Arc::clone(&log);
    manager.subscribe_connection_events_with_snapshot(move |event| {
        push_event(&log_for_subscriber, event)
    });
    let _peers = churn.join().expect("churn thread must not panic");
    // One final post-churn session guarantees the subscriber observed
    // something even if the subscription raced past the churn entirely.
    let (server, _client, addr) = socket_pair();
    manager
        .register_test_connection(node, addr, server)
        .expect("final install");
    manager.disconnect_node(node);

    let events = snapshot(&log);
    assert!(!events.is_empty());
    assert!(
        matches!(events.first(), Some(ConnectionEvent::Up(_))),
        "the stream must start with an Up (synthetic catch-up or the next real session)"
    );
    let mut open: Option<ConnectionGeneration> = None;
    let mut last_closed: Option<ConnectionGeneration> = None;
    for event in &events {
        assert_eq!(event.node(), node);
        match event {
            ConnectionEvent::Up(up) => {
                assert_eq!(
                    open,
                    None,
                    "Up({}) delivered while a generation is still open (double-see)",
                    up.generation.get()
                );
                if let Some(closed) = last_closed {
                    assert_eq!(
                        up.generation.get(),
                        closed.get() + 1,
                        "generations are dense: a gap is a missed session"
                    );
                }
                open = Some(up.generation);
            }
            ConnectionEvent::Down(down) => {
                assert_eq!(
                    open,
                    Some(down.generation),
                    "Down({}) must close the currently open generation",
                    down.generation.get()
                );
                last_closed = Some(down.generation);
                open = None;
            }
        }
    }
    assert_eq!(open, None, "the final session closed");
    assert_eq!(
        last_closed,
        manager.last_peer_generation(node),
        "the subscriber observed every session through the very last one"
    );
}

/// Calling the snapshot-subscribe path from INSIDE a subscriber callback on
/// the same manager must not deadlock: the reentrancy check registers the
/// subscription WITHOUT synthetic catch-up (the documented degradation), and
/// the subscription is live for subsequent real events.
#[tokio::test]
async fn reentrant_snapshot_subscribe_from_callback_registers_without_synthetics() {
    let manager = manager_named("local@127.0.0.1");
    let node_x = manager.atom_table().intern("xnode@127.0.0.1");
    let node_y = manager.atom_table().intern("ynode@127.0.0.1");
    let (_connection_x, _peer_x) = install(&manager, "xnode@127.0.0.1");
    let (_connection_y, _peer_y) = install(&manager, "ynode@127.0.0.1");

    let inner_log = new_event_log();
    let manager_for_subscriber = manager.clone();
    let inner_log_for_subscriber = Arc::clone(&inner_log);
    manager.subscribe_connection_events(move |event| {
        if let ConnectionEvent::Down(down) = event
            && down.node == node_x
        {
            // ynode is live: a non-reentrant call would deliver its synthetic
            // Up. Reentrantly, registration must return immediately (no gate
            // deadlock) and deliver nothing.
            let inner_log_for_inner = Arc::clone(&inner_log_for_subscriber);
            manager_for_subscriber.subscribe_connection_events_with_snapshot(move |event| {
                push_event(&inner_log_for_inner, event);
            });
        }
    });

    assert!(manager.disconnect_node(node_x));
    assert!(
        snapshot(&inner_log).is_empty(),
        "reentrant registration delivers no synthetic catch-up (ynode was live)"
    );

    assert!(manager.disconnect_node(node_y));
    assert_eq!(
        snapshot(&inner_log),
        vec![ConnectionEvent::down(
            node_y,
            generation(1),
            ConnectionDownReason::ManualDisconnect
        )],
        "the reentrant subscription is live for subsequent real events"
    );
}
