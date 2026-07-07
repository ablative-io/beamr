//! Connection-events end-to-end tests over real loopback distribution links.
//!
//! Exercises the generation-tagged connection-event hub
//! (docs/CONN-EVENTS-HOOK-SPEC.md) at the full-scheduler level, in the
//! `pg_distribution_e2e` harness shape: in-process Schedulers, a
//! `DynamicResolver`, loopback TCP, bounded `eventually` polling — plus the
//! HS-5 hard wall-clock watchdog (60s, separate thread + `recv_timeout`, as in
//! `distribution_mesh_handshake`) on every multi-node scenario.
//!
//! Covered here (spec §8, commit 4):
//! - **Generation monotonicity across reconnect**, with the INV-SYNC
//!   observable: `Down(g1, ManualDisconnect)` is delivered before
//!   `disconnect_node` returns — no polling between the call and the assert.
//! - **Peer-bounce vs link-blip** discrimination: a restarted peer VM (same
//!   node name, new creation) surfaces as a changed `peer_creation` on the
//!   next Up, while a mere link blip keeps it identical.
//! - **H1-closure regression**: the pg purge for generation g runs before
//!   generation g+1's read loop exists, so fresh joins sent on g+1 survive
//!   and are never wiped by the stale purge.
//! - **Multi-subscriber**: an embedder-style probe alongside the scheduler's
//!   composed subscriber, proving pg semantics are unchanged in its presence
//!   and that the probe observes post-purge pg state at Down delivery
//!   (INV-SCHED-FIRST observable).
//!
//! INV-FRAME-ORDER is pinned at the unit level
//! (`connection_events_tests::inv_frame_order_up_delivered_before_first_inbound_frame`)
//! and is not repeated here.

#![cfg(feature = "net")]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use beamr::atom::{Atom, AtomTable};
use beamr::distribution::connection_events::{ConnectionDownReason, ConnectionEvent};
use beamr::distribution::pg::RemoteMember;
use beamr::distribution::resolver::{NodeResolver, ResolveError, ResolveFuture};
use beamr::distribution::{ConnectionManager, DEFAULT_COOKIE, DistributionConfig};
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;
use beamr::scheduler::{Scheduler, SchedulerConfig};

#[derive(Default)]
struct DynamicResolver {
    nodes: Mutex<HashMap<String, SocketAddr>>,
}

impl DynamicResolver {
    fn insert(&self, name: &str, addr: SocketAddr) {
        self.nodes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(name.to_owned(), addr);
    }
}

impl NodeResolver for DynamicResolver {
    fn resolve<'a>(&'a self, name: &'a str) -> ResolveFuture<'a> {
        let result = self
            .nodes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(name)
            .copied()
            .ok_or(ResolveError::NotFound);
        Box::pin(async move { result })
    }
}

/// One in-process scheduler node with real distribution wiring. `creation` is
/// the local VM incarnation advertised in the handshake — the value a peer
/// surfaces as `NodeUp::peer_creation`.
fn scheduler(
    node_name: &str,
    creation: u32,
    resolver: Arc<DynamicResolver>,
    atom_table: Arc<AtomTable>,
) -> Scheduler {
    let bif_registry = Arc::new(BifRegistryImpl::new());
    let module_registry = Arc::new(ModuleRegistry::new());
    Scheduler::with_code_server_and_policy(
        SchedulerConfig {
            thread_count: Some(1),
            node_name: Some(node_name.to_owned()),
            creation: Some(creation),
            distribution: Some(DistributionConfig {
                resolver,
                cookie: DEFAULT_COOKIE.to_owned(),
            }),
            ..SchedulerConfig::default()
        },
        module_registry,
        atom_table,
        bif_registry,
        Arc::new(beamr::native::AllCapabilitiesPolicy),
    )
    .expect("scheduler starts")
}

/// Poll `predicate` for up to ~1s, sleeping between attempts.
async fn eventually(mut predicate: impl FnMut() -> bool) -> bool {
    for _ in 0..100 {
        if predicate() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    predicate()
}

/// HS-5 watchdog discipline: run `scenario` on a worker thread and fail hard
/// when it does not complete within 60s, so a hung multi-node scenario (a
/// blocked dispatch, a handshake that never returns) fails the test instead
/// of parking the whole test binary.
fn run_under_watchdog(name: &str, scenario: impl FnOnce() + Send + 'static) {
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        scenario();
        let _ = done_tx.send(());
    });
    match done_rx.recv_timeout(Duration::from_secs(60)) {
        Ok(()) => worker
            .join()
            .expect("watchdogged scenario thread should not panic"),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            // The worker panicked before signalling: propagate its panic so the
            // real assertion failure is reported, not a watchdog message.
            if let Err(payload) = worker.join() {
                std::panic::resume_unwind(payload);
            }
        }
        Err(mpsc::RecvTimeoutError::Timeout) => panic!(
            "{name}: multi-node scenario did not complete within the 60s watchdog \
             (a connect, dispatch, or drain never returned)"
        ),
    }
}

/// Drive an async scenario body on a dedicated current-thread runtime (the
/// watchdog worker thread owns it; the schedulers under test each own their
/// distribution runtimes independently, as in production).
fn block_on_scenario<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("scenario runtime builds")
        .block_on(future)
}

/// Shared recorder for an embedder-style probe subscriber.
type EventLog = Arc<Mutex<Vec<ConnectionEvent>>>;

/// Subscribe an embedder-style recording probe: every hub event, in delivery
/// order, appended to the returned log.
fn subscribe_probe(manager: &ConnectionManager) -> EventLog {
    let log: EventLog = Arc::new(Mutex::new(Vec::new()));
    let probe_log = Arc::clone(&log);
    manager.subscribe_connection_events(move |event| {
        probe_log
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(event);
    });
    log
}

/// `(generation, peer_creation)` for every Up delivered for `node`, in order.
fn up_events(log: &EventLog, node: Atom) -> Vec<(u64, u32)> {
    log.lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
        .filter_map(|event| match event {
            ConnectionEvent::Up(up) if up.node == node => {
                Some((up.generation.get(), up.peer_creation))
            }
            _ => None,
        })
        .collect()
}

/// `(generation, reason)` for every Down delivered for `node`, in order.
fn down_events(log: &EventLog, node: Atom) -> Vec<(u64, ConnectionDownReason)> {
    log.lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
        .filter_map(|event| match event {
            ConnectionEvent::Down(down) if down.node == node => {
                Some((down.generation.get(), down.reason))
            }
            _ => None,
        })
        .collect()
}

/// Set a `(Mutex<bool>, Condvar)` gate and wake its waiters.
fn open_gate(gate: &(Mutex<bool>, Condvar)) {
    let (flag, condvar) = gate;
    *flag.lock().unwrap_or_else(|error| error.into_inner()) = true;
    condvar.notify_all();
}

/// Block until a `(Mutex<bool>, Condvar)` gate opens or `timeout` elapses;
/// returns whether the gate opened.
fn await_gate(gate: &(Mutex<bool>, Condvar), timeout: Duration) -> bool {
    let (flag, condvar) = gate;
    let guard = flag.lock().unwrap_or_else(|error| error.into_inner());
    let (guard, _result) = condvar
        .wait_timeout_while(guard, timeout, |opened| !*opened)
        .unwrap_or_else(|error| error.into_inner());
    *guard
}

/// Generation monotonicity across a reconnect, and the INV-SYNC observable:
/// `Up(g1)` is delivered when A's link installs on B; `Down(g1,
/// ManualDisconnect)` is delivered BEFORE `disconnect_node` returns (asserted
/// with no polling); the re-dial opens `Up(g2)` with `g2 > g1`; and across
/// this link blip — the peer VM never restarted — `peer_creation` is
/// identical on both Ups.
#[test]
fn generation_monotonic_across_reconnect_and_down_synchronous_with_disconnect() {
    run_under_watchdog("generation monotonicity across reconnect", || {
        block_on_scenario(async {
            let resolver = Arc::new(DynamicResolver::default());
            let atom_table = Arc::new(AtomTable::with_common_atoms());
            let node_a = scheduler(
                "a@127.0.0.1",
                41,
                Arc::clone(&resolver),
                Arc::clone(&atom_table),
            );
            let node_b = scheduler(
                "b@127.0.0.1",
                7,
                Arc::clone(&resolver),
                Arc::clone(&atom_table),
            );
            let listen_b = node_b
                .distribution_connections()
                .listen("127.0.0.1:0".parse().expect("listen address parses"))
                .await
                .expect("node B listens");
            resolver.insert("b@127.0.0.1", listen_b.local_addr());
            let a_atom = node_a.local_node().name;

            // Embedder-style probe on B records every hub event for A.
            let log = subscribe_probe(&node_b.distribution_connections());

            node_a
                .distribution_connections()
                .connect("b@127.0.0.1")
                .await
                .expect("A connects to B");
            assert!(
                eventually(|| !up_events(&log, a_atom).is_empty()).await,
                "B delivers Up(g1) after accepting A"
            );
            let (g1, creation1) = up_events(&log, a_atom)[0];
            assert_eq!(creation1, 41, "Up carries A's handshake creation");

            // INV-SYNC observable: when disconnect_node returns, the Down it
            // caused has already been delivered to every subscriber — assert
            // immediately, with no `eventually`.
            assert!(
                node_b.distribution_connections().disconnect_node(a_atom),
                "B disconnects A"
            );
            assert_eq!(
                down_events(&log, a_atom),
                vec![(g1, ConnectionDownReason::ManualDisconnect)],
                "Down(g1, ManualDisconnect) is delivered before disconnect_node returns"
            );

            // Reconnect: the next session opens a strictly greater generation.
            node_a
                .distribution_connections()
                .connect("b@127.0.0.1")
                .await
                .expect("A re-dials B");
            assert!(
                eventually(|| up_events(&log, a_atom).len() >= 2).await,
                "B delivers Up(g2) after the re-dial"
            );
            let ups = up_events(&log, a_atom);
            assert_eq!(ups.len(), 2, "exactly one Up per session: {ups:?}");
            let (g2, creation2) = ups[1];
            assert!(
                g2 > g1,
                "generations are strictly monotonic per peer (g1={g1}, g2={g2})"
            );
            assert_eq!(
                creation2, creation1,
                "a link blip keeps peer_creation — only a peer restart changes it"
            );
            assert_eq!(
                down_events(&log, a_atom).len(),
                1,
                "exactly one Down per closed session"
            );

            listen_b.shutdown();
            node_a.shutdown();
            node_b.shutdown();
        });
    });
}

/// Peer-bounce vs link-blip: restarting the peer scheduler under the same
/// node name but a different creation is visible as a changed
/// `peer_creation` on the next Up — the discriminator generations alone
/// cannot provide (the blip half is pinned by
/// `generation_monotonic_across_reconnect_and_down_synchronous_with_disconnect`,
/// where `peer_creation` stays identical).
#[test]
fn peer_bounce_changes_peer_creation_on_next_up() {
    run_under_watchdog("peer bounce vs blip via peer_creation", || {
        block_on_scenario(async {
            let resolver = Arc::new(DynamicResolver::default());
            let atom_table = Arc::new(AtomTable::with_common_atoms());
            let node_a = scheduler(
                "a@127.0.0.1",
                41,
                Arc::clone(&resolver),
                Arc::clone(&atom_table),
            );
            let node_b = scheduler(
                "b@127.0.0.1",
                7,
                Arc::clone(&resolver),
                Arc::clone(&atom_table),
            );
            let listen_b = node_b
                .distribution_connections()
                .listen("127.0.0.1:0".parse().expect("listen address parses"))
                .await
                .expect("node B listens");
            resolver.insert("b@127.0.0.1", listen_b.local_addr());
            let a_atom = node_a.local_node().name;

            let log = subscribe_probe(&node_b.distribution_connections());

            node_a
                .distribution_connections()
                .connect("b@127.0.0.1")
                .await
                .expect("A connects to B");
            assert!(
                eventually(|| !up_events(&log, a_atom).is_empty()).await,
                "B delivers Up(g1) for the first A incarnation"
            );
            let (g1, creation1) = up_events(&log, a_atom)[0];
            assert_eq!(creation1, 41, "first Up carries the first incarnation");

            // Close the session, then restart A as a NEW VM incarnation: same
            // node name, different creation.
            assert!(
                node_b.distribution_connections().disconnect_node(a_atom),
                "B disconnects A"
            );
            node_a.shutdown();
            let node_a_restarted = scheduler(
                "a@127.0.0.1",
                42,
                Arc::clone(&resolver),
                Arc::clone(&atom_table),
            );
            node_a_restarted
                .distribution_connections()
                .connect("b@127.0.0.1")
                .await
                .expect("restarted A dials B");

            assert!(
                eventually(|| up_events(&log, a_atom).len() >= 2).await,
                "B delivers Up(g2) for the restarted A"
            );
            let ups = up_events(&log, a_atom);
            assert_eq!(ups.len(), 2, "exactly one Up per session: {ups:?}");
            let (g2, creation2) = ups[1];
            assert!(
                g2 > g1,
                "the bounce opens a strictly greater generation (g1={g1}, g2={g2})"
            );
            assert_eq!(
                creation2, 42,
                "the post-bounce Up carries the restarted peer's creation"
            );
            assert_ne!(
                creation2, creation1,
                "a peer restart is visible as a peer_creation change on the next Up"
            );

            // Full per-node history: Up(g1) Down(g1) Up(g2) — INV-ALTERNATION
            // at the e2e level.
            let kinds: Vec<bool> = log
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .iter()
                .filter(|event| event.node() == a_atom)
                .map(|event| event.down_reason().is_some())
                .collect();
            assert_eq!(
                kinds,
                vec![false, true, false],
                "delivered history for A is exactly Up, Down, Up"
            );

            listen_b.shutdown();
            node_a_restarted.shutdown();
            node_b.shutdown();
        });
    });
}

/// H1-closure regression: with the Down(g) drain deliberately stalled INSIDE
/// a probe subscriber (after the scheduler's composed subscriber already
/// purged), the peer re-dials — installing generation g+1 — and immediately
/// joins a fresh pg member on the new session. Because g+1's read loop is
/// spawned only after its installer's dispatch returns, the fresh join
/// cannot be applied while the stale drain is in flight, so the purge for g
/// provably ran before g+1's read loop existed and the g+1 join survives in
/// the final pg state.
#[test]
fn h1_regression_purge_for_old_generation_precedes_new_generations_read_loop() {
    run_under_watchdog("H1 closure: stale purge vs fresh joins", || {
        block_on_scenario(async {
            let resolver = Arc::new(DynamicResolver::default());
            let atom_table = Arc::new(AtomTable::with_common_atoms());
            let node_a = scheduler(
                "a@127.0.0.1",
                41,
                Arc::clone(&resolver),
                Arc::clone(&atom_table),
            );
            let node_b = scheduler(
                "b@127.0.0.1",
                7,
                Arc::clone(&resolver),
                Arc::clone(&atom_table),
            );
            let listen_b = node_b
                .distribution_connections()
                .listen("127.0.0.1:0".parse().expect("listen address parses"))
                .await
                .expect("node B listens");
            resolver.insert("b@127.0.0.1", listen_b.local_addr());
            let a_atom = node_a.local_node().name;

            let scope = node_a.pg_registry().default_scope();
            let group = atom_table.intern("h1_closure");
            let registry_b = node_b.pg_registry();

            // Recording probe first, so the event history is pinned; the
            // staller below registers after it (and both after the
            // scheduler's composed subscriber, registered at construction).
            let log = subscribe_probe(&node_b.distribution_connections());

            // Stalling probe: parks the FIRST Down-for-A drain until released.
            // This deliberately violates INV-SUB-DISCIPLINE (a test
            // instrument, like the spec's "deliberately slow subscriber") to
            // hold the dispatch gate open across the peer's re-dial.
            let entered_down = Arc::new((Mutex::new(false), Condvar::new()));
            let release = Arc::new((Mutex::new(false), Condvar::new()));
            let stalled_once = Arc::new(AtomicBool::new(false));
            let staller_entered = Arc::clone(&entered_down);
            let staller_release = Arc::clone(&release);
            let staller_once = Arc::clone(&stalled_once);
            node_b
                .distribution_connections()
                .subscribe_connection_events(move |event| {
                    if event.node() == a_atom
                        && event.down_reason().is_some()
                        && !staller_once.swap(true, Ordering::SeqCst)
                    {
                        open_gate(&staller_entered);
                        assert!(
                            await_gate(&staller_release, Duration::from_secs(50)),
                            "the stalled drain is always released"
                        );
                    }
                });

            // Session g1: A connects and joins a member; B observes it.
            node_a
                .distribution_connections()
                .connect("b@127.0.0.1")
                .await
                .expect("A connects to B");
            let pre_down_pid = 100_u64;
            node_a.pg_registry().join(scope, group, pre_down_pid);
            let pre_down_member = RemoteMember {
                node: a_atom,
                pid_number: pre_down_pid,
                serial: 0,
            };
            assert!(
                eventually(|| registry_b
                    .remote_members(scope, group)
                    .contains(&pre_down_member))
                .await,
                "B observes A's member while generation g1 is up"
            );

            // Tear the link down from another thread; its dispatch stalls in
            // the probe with the gate held.
            let b_connections = node_b.distribution_connections();
            let disconnector = thread::spawn(move || {
                assert!(b_connections.disconnect_node(a_atom), "B disconnects A");
            });
            assert!(
                await_gate(&entered_down, Duration::from_secs(30)),
                "the Down(g1) drain reaches the stalling probe"
            );
            // The scheduler's composed subscriber registered first, so by the
            // time the probe stalls the drain, the purge for g1 already ran.
            assert!(
                registry_b.remote_members(scope, group).is_empty(),
                "the pg purge for generation g1 ran before the probe stalled the drain"
            );

            // While the Down(g1) drain is stalled, A re-dials — B installs
            // generation g2 and enqueues Up(g2), but its dispatch blocks on
            // the gate — and A immediately joins a fresh member on g2.
            node_a
                .distribution_connections()
                .connect("b@127.0.0.1")
                .await
                .expect("A re-dials B while the drain is stalled");
            let rejoin_pid = 200_u64;
            node_a.pg_registry().join(scope, group, rejoin_pid);
            let rejoin_member = RemoteMember {
                node: a_atom,
                pid_number: rejoin_pid,
                serial: 0,
            };

            // g2's read loop must not exist yet (its installer's dispatch is
            // blocked on the gate), so the fresh join cannot have been
            // applied — and therefore cannot be wiped by the in-flight
            // Down(g1) processing.
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert!(
                !registry_b
                    .remote_members(scope, group)
                    .contains(&rejoin_member),
                "no g2 join is applied while the Down(g1) drain is still in flight"
            );

            open_gate(&release);
            disconnector
                .join()
                .expect("disconnect_node thread completes");

            // Drain finished: Up(g2) delivered, g2's read loop spawned, and
            // the fresh join applies WITHOUT being wiped by the (already
            // completed) purge for g1.
            assert!(
                eventually(|| registry_b
                    .remote_members(scope, group)
                    .contains(&rejoin_member))
                .await,
                "the g2 join survives: the purge for g1 ran before g2's read loop existed"
            );
            assert!(
                !registry_b
                    .remote_members(scope, group)
                    .contains(&pre_down_member),
                "the old generation's member stays purged"
            );

            // Event order pinned: Down(g1) precedes Up(g2), one of each.
            let downs = down_events(&log, a_atom);
            let ups = up_events(&log, a_atom);
            assert_eq!(
                downs,
                vec![(ups[0].0, ConnectionDownReason::ManualDisconnect)],
                "exactly one Down, for generation g1"
            );
            assert_eq!(ups.len(), 2, "exactly one Up per session: {ups:?}");
            assert!(
                ups[1].0 > ups[0].0,
                "the re-dial session's generation is strictly greater"
            );

            listen_b.shutdown();
            node_a.shutdown();
            node_b.shutdown();
        });
    });
}

/// Multi-subscriber e2e: an embedder-style probe attached alongside the
/// scheduler's composed subscriber leaves pg semantics unchanged (join
/// visible on the peer, purged on node-down — the
/// `pg_join_visible_on_peer_and_purged_on_node_down` contract), the purge is
/// synchronous with `disconnect_node` (INV-SYNC), and the probe observes
/// post-purge pg state at Down delivery (INV-SCHED-FIRST).
#[test]
fn embedder_probe_alongside_scheduler_subscriber_leaves_pg_semantics_unchanged() {
    run_under_watchdog("embedder probe alongside scheduler subscriber", || {
        block_on_scenario(async {
            let resolver = Arc::new(DynamicResolver::default());
            let atom_table = Arc::new(AtomTable::with_common_atoms());
            let node_a = scheduler(
                "a@127.0.0.1",
                41,
                Arc::clone(&resolver),
                Arc::clone(&atom_table),
            );
            let node_b = scheduler(
                "b@127.0.0.1",
                7,
                Arc::clone(&resolver),
                Arc::clone(&atom_table),
            );
            let listen_b = node_b
                .distribution_connections()
                .listen("127.0.0.1:0".parse().expect("listen address parses"))
                .await
                .expect("node B listens");
            resolver.insert("b@127.0.0.1", listen_b.local_addr());
            let a_atom = node_a.local_node().name;

            let scope = node_a.pg_registry().default_scope();
            let group = atom_table.intern("workers");
            let registry_b = node_b.pg_registry();

            // Embedder-style probe: records events AND, at each Down-for-A
            // delivery, whether B's pg view of A was already purged. Captures
            // a Weak registry handle per INV-SUB-DISCIPLINE.
            let log = subscribe_probe(&node_b.distribution_connections());
            let purged_at_down: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));
            let probe_purged = Arc::clone(&purged_at_down);
            let probe_registry = Arc::downgrade(&registry_b);
            node_b
                .distribution_connections()
                .subscribe_connection_events(move |event| {
                    if event.node() == a_atom
                        && event.down_reason().is_some()
                        && let Some(registry) = probe_registry.upgrade()
                    {
                        probe_purged
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .push(registry.remote_members(scope, group).is_empty());
                    }
                });

            // pg semantics with the probe attached: a join on A is visible on
            // B, exactly as without the probe.
            node_a
                .distribution_connections()
                .connect("b@127.0.0.1")
                .await
                .expect("A connects to B");
            let member_pid = 4242_u64;
            node_a.pg_registry().join(scope, group, member_pid);
            let expected = RemoteMember {
                node: a_atom,
                pid_number: member_pid,
                serial: 0,
            };
            assert!(
                eventually(|| registry_b.remote_members(scope, group).contains(&expected)).await,
                "B observes A's remote pg member with the probe attached"
            );
            assert!(
                eventually(|| !up_events(&log, a_atom).is_empty()).await,
                "the probe receives Up(g1)"
            );
            let (g1, _creation) = up_events(&log, a_atom)[0];

            // Node-down: the purge is synchronous with disconnect_node
            // (INV-SYNC) — assert with no polling — and unchanged in the
            // probe's presence.
            assert!(
                node_b.distribution_connections().disconnect_node(a_atom),
                "B disconnects A"
            );
            assert!(
                registry_b.remote_members(scope, group).is_empty(),
                "the pg purge completed before disconnect_node returned, probe attached"
            );
            assert_eq!(
                down_events(&log, a_atom),
                vec![(g1, ConnectionDownReason::ManualDisconnect)],
                "the probe receives exactly one Down(g1, ManualDisconnect)"
            );
            // INV-SCHED-FIRST observable: the scheduler's composed subscriber
            // (pg purge) ran before the probe saw the Down.
            assert_eq!(
                *purged_at_down
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()),
                vec![true],
                "the probe observes post-purge pg state at Down delivery"
            );

            listen_b.shutdown();
            node_a.shutdown();
            node_b.shutdown();
        });
    });
}
