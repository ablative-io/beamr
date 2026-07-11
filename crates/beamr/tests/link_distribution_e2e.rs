//! Cross-node supervision (LINK/EXIT) end-to-end tests over real loopback
//! distribution links, at the PUBLIC API only (`Scheduler::{link_remote,
//! exit_signal, set_trap_exit, process_namespace, enqueue_atom_message,
//! spawn_native}`), in the `pg_distribution_e2e` harness shape: in-process
//! Schedulers, a `DynamicResolver`, loopback TCP, bounded `eventually`
//! polling — plus the HS-5 hard wall-clock watchdog (60s, separate thread +
//! `recv_timeout`, as in `distribution_mesh_handshake`) on every multi-node
//! scenario (spec DIST-CONTROL-WIRE-SPEC.md §8, scenario 10).
//!
//! Covered here (§8 scenarios 1-6 + the 9-e2e-half; the in-crate halves live
//! in `scheduler/remote_supervision_tests.rs`):
//! - **1** cross-node link + abnormal exit kills the linked peer, both
//!   directions;
//! - **2** a `normal` remote exit never kills the linked peer;
//! - **3** Kill leaves the node pre-terminalized as `killed`, so a trapping
//!   remote peer survives it (raw `kill` on the wire would have killed it);
//! - **4** node death (peer's AcceptHandle + Scheduler dropped => read-loop
//!   EOF) delivers `noconnection` to every remote-linked process;
//! - **5** the pg purge is observable strictly before the `noconnection`
//!   delivery (order contract of the composed down-subscriber);
//! - **6** no double-fire, forward order: a wire EXIT that consumed the link
//!   is not re-signalled as `noconnection` by the node death that follows
//!   (DC-4). The reverse order — node death first, then a late wire EXIT —
//!   is not drivable from the public API (no way to inject a frame after the
//!   connection is gone) and is pinned in-crate
//!   (`backstop_then_late_wire_exit_delivers_exactly_one_signal`);
//! - **9 (e2e half)** a hostile (undecodable) frame written by a connected
//!   peer through the public `write_raw` is dropped without killing the read
//!   loop: both a subsequent pg frame and a subsequent LINK+EXIT deliver.
//!
//! Determinism notes the scenarios lean on (no sleeps as synchronization):
//! - `link_remote` records the LOCAL half-link and enqueues the wire LINK
//!   before returning, and `cleanup_exited_process` enqueues the wire EXIT
//!   (`propagate_exit`) BEFORE removing the process body — so observing
//!   `process_namespace(pid) == None` proves that process's EXIT control is
//!   already on the lane.
//! - The control lane is per-node FIFO (DC-5) and the receiving read loop
//!   applies frames serially, so a LINK enqueued before an EXIT is applied
//!   before it, and a "canary" EXIT enqueued after another EXIT proves the
//!   earlier one was applied once the canary's effect is visible.

#![cfg(feature = "net")]

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use beamr::atom::{Atom, AtomTable};
use beamr::distribution::connection::AcceptHandle;
use beamr::distribution::pg::RemoteMember;
use beamr::distribution::resolver::{NodeResolver, ResolveError, ResolveFuture};
use beamr::distribution::{DEFAULT_COOKIE, DistributionConfig};
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;
use beamr::process::{ExitReason, RemotePid};
use beamr::scheduler::{Scheduler, SchedulerConfig};
use beamr::{NativeContext, NativeHandler, NativeOutcome};

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

fn scheduler(
    node_name: &str,
    resolver: Arc<DynamicResolver>,
    atom_table: Arc<AtomTable>,
) -> Scheduler {
    let bif_registry = Arc::new(BifRegistryImpl::new());
    let module_registry = Arc::new(ModuleRegistry::new());
    Scheduler::with_code_server_and_policy(
        SchedulerConfig {
            thread_count: Some(1),
            node_name: Some(node_name.to_owned()),
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

/// A minimal scheduler-supervised process that never exits on its own: the
/// link endpoint whose lifetime each scenario controls via `exit_signal` (the
/// `pg_distribution_e2e` `Idle` role, as a plain `NativeHandler` so the
/// node-drop scenarios retain no `Arc<Scheduler>` clone in an `ActorRef`).
/// It drains and ignores its mailbox, so a trapping instance consumes its
/// `{'EXIT', _, _}` tuples and keeps parking, like a gen_server ignoring
/// unknown messages.
struct Idle;

impl NativeHandler for Idle {
    fn handle(&mut self, ctx: &mut NativeContext<'_>) -> NativeOutcome {
        while ctx.recv().is_some() {}
        NativeOutcome::Wait
    }
}

/// Exits with reason `Normal` on the first message: the public-API way to
/// drive a clean process death (scenarios 2 and 6) — poke it with
/// `Scheduler::enqueue_atom_message`.
struct ExitsNormallyOnPoke;

impl NativeHandler for ExitsNormallyOnPoke {
    fn handle(&mut self, ctx: &mut NativeContext<'_>) -> NativeOutcome {
        if ctx.recv().is_some() {
            NativeOutcome::Stop(ExitReason::Normal)
        } else {
            NativeOutcome::Wait
        }
    }
}

fn spawn_idle(node: &Scheduler) -> u64 {
    node.spawn_native(Box::new(|| Box::new(Idle)))
        .expect("idle process spawns")
}

fn spawn_exits_normally(node: &Scheduler) -> u64 {
    node.spawn_native(Box::new(|| Box::new(ExitsNormallyOnPoke)))
        .expect("exits-normally process spawns")
}

fn remote(node: Atom, pid: u64) -> RemotePid {
    RemotePid {
        node,
        pid_number: pid,
        serial: 0,
    }
}

/// Poll `predicate` for up to ~5s, sleeping between attempts (headroom over
/// the pg harness's ~1s window: these scenarios run in parallel, each with
/// two full schedulers).
async fn eventually(mut predicate: impl FnMut() -> bool) -> bool {
    for _ in 0..500 {
        if predicate() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    predicate()
}

/// Hold `predicate` false over a ~1s window (a bounded negative observation,
/// the `pg_join_is_connected_only_and_not_replayed` "leaked" pattern).
async fn never_within_window(mut predicate: impl FnMut() -> bool) -> bool {
    for _ in 0..100 {
        if predicate() {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    !predicate()
}

/// HS-5 watchdog discipline (scenario 10): run `scenario` on a worker thread
/// and fail hard when it does not complete within 60s, so a hung multi-node
/// scenario (a lost EXIT, an EOF that never surfaces, a blocked dispatch)
/// fails the test instead of parking the whole test binary.
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
             (a connect, delivery, or node-down convergence never happened)"
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

/// Two connected in-process nodes. Only B listens (A dials), so B is "the
/// peer" whose `AcceptHandle` + `Scheduler` the node-drop scenarios destroy.
struct Duo {
    node_a: Scheduler,
    node_b: Scheduler,
    listen_b: AcceptHandle,
    atom_table: Arc<AtomTable>,
    a_atom: Atom,
    b_atom: Atom,
}

async fn connect_duo() -> Duo {
    let resolver = Arc::new(DynamicResolver::default());
    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let node_a = scheduler(
        "a@127.0.0.1",
        Arc::clone(&resolver),
        Arc::clone(&atom_table),
    );
    let node_b = scheduler(
        "b@127.0.0.1",
        Arc::clone(&resolver),
        Arc::clone(&atom_table),
    );
    let listen_b = node_b
        .try_distribution_connections()
        .expect("distribution owned")
        .listen("127.0.0.1:0".parse().expect("listen address parses"))
        .await
        .expect("node B listens");
    resolver.insert("b@127.0.0.1", listen_b.local_addr());
    let a_atom = node_a.local_node().name;
    let b_atom = node_b.local_node().name;

    node_a
        .try_distribution_connections()
        .expect("distribution owned")
        .connect("b@127.0.0.1")
        .await
        .expect("A connects to B");
    // The accept side registers the reciprocal connection asynchronously; the
    // scenarios drive `link_remote` from BOTH nodes, so wait for both tables.
    assert!(
        eventually(|| {
            node_b
                .try_distribution_connections()
                .expect("distribution owned")
                .connected_nodes()
                .contains(&a_atom)
        })
        .await,
        "B registers the accepted connection to A"
    );

    Duo {
        node_a,
        node_b,
        listen_b,
        atom_table,
        a_atom,
        b_atom,
    }
}

/// Scenario 4's package shape: drop the peer's `AcceptHandle` and `Scheduler`.
/// `shutdown` joins the workers and aborts the sender drain; dropping the
/// `Scheduler` then drops its `SharedState` — the owned distribution runtime
/// (killing the read-loop tasks and their read halves) and the connection
/// table (the write halves) — fully closing every socket, so the surviving
/// node's read loop observes EOF and marks the connection down (`PeerClosed`).
fn drop_peer(listen_b: AcceptHandle, node_b: Scheduler) {
    listen_b.shutdown();
    drop(listen_b);
    node_b.shutdown();
    drop(node_b);
}

/// Scenario 1, both directions: `Scheduler::link_remote` + an abnormal local
/// exit kills the linked peer across the wire. Each direction is driven
/// entirely from the linker's node: the LINK control is enqueued by
/// `link_remote` and the EXIT control by the linker's death, in that order on
/// one FIFO lane (DC-5), so the receiver always applies LINK before EXIT.
#[test]
fn cross_node_link_and_abnormal_exit_kill_the_linked_peer_both_directions() {
    run_under_watchdog(
        "scenario 1: cross-node link + exit, both directions",
        || {
            block_on_scenario(async {
                let duo = connect_duo().await;

                // Direction A -> B: A's linker dies, B's peer dies.
                let pid_a = spawn_idle(&duo.node_a);
                let pid_b = spawn_idle(&duo.node_b);
                duo.node_a
                    .link_remote(pid_a, remote(duo.b_atom, pid_b))
                    .expect("A links to B's process");
                duo.node_a
                    .exit_signal(0, pid_a, ExitReason::Error)
                    .expect("exit signal delivered on A");
                assert!(
                    eventually(|| duo.node_a.process_namespace(pid_a).is_none()).await,
                    "the linker dies locally"
                );
                assert!(
                    eventually(|| duo.node_b.process_namespace(pid_b).is_none()).await,
                    "the wire EXIT kills the non-trapping linked peer (A -> B)"
                );

                // Direction B -> A, mirrored.
                let pid_a2 = spawn_idle(&duo.node_a);
                let pid_b2 = spawn_idle(&duo.node_b);
                duo.node_b
                    .link_remote(pid_b2, remote(duo.a_atom, pid_a2))
                    .expect("B links to A's process");
                duo.node_b
                    .exit_signal(0, pid_b2, ExitReason::Error)
                    .expect("exit signal delivered on B");
                assert!(
                    eventually(|| duo.node_b.process_namespace(pid_b2).is_none()).await,
                    "the linker dies locally"
                );
                assert!(
                    eventually(|| duo.node_a.process_namespace(pid_a2).is_none()).await,
                    "the wire EXIT kills the non-trapping linked peer (B -> A)"
                );

                duo.listen_b.shutdown();
                duo.node_a.shutdown();
                duo.node_b.shutdown();
            });
        },
    );
}

/// Scenario 2: a linked process exiting with reason `normal` does not kill
/// its non-trapping remote peer — positive aliveness held over a full
/// observation window after the clean death is confirmed.
#[test]
fn normal_remote_exit_never_kills_the_linked_peer() {
    run_under_watchdog("scenario 2: normal exit does not kill", || {
        block_on_scenario(async {
            let duo = connect_duo().await;

            let pid_a = spawn_idle(&duo.node_a);
            let pid_b = spawn_exits_normally(&duo.node_b);
            duo.node_b
                .link_remote(pid_b, remote(duo.a_atom, pid_a))
                .expect("B links to A's process");

            // Poke B's process into a clean, self-driven Normal exit.
            assert!(
                duo.node_b
                    .enqueue_atom_message(pid_b, duo.atom_table.intern("poke")),
                "the poke reaches B's live process"
            );
            assert!(
                eventually(|| duo.node_b.process_namespace(pid_b).is_none()).await,
                "B's process exits normally"
            );

            // Its EXIT(normal) is already on the lane (propagate_exit runs
            // before the body is removed). Over the full window, A's linked
            // process must never die.
            assert!(
                never_within_window(|| duo.node_a.process_namespace(pid_a).is_none()).await,
                "a normal remote exit must never kill the non-trapping linked peer"
            );

            duo.listen_b.shutdown();
            duo.node_a.shutdown();
            duo.node_b.shutdown();
        });
    });
}

/// Scenario 3: `exit_signal(.., Kill)` terminates the linked process and the
/// wire EXIT leaves the node pre-terminalized as `killed` — trappable at the
/// receiver. The trapping peer surviving IS the discriminator: raw `kill` on
/// the wire would have killed it untrappably. A non-trapping sibling linked
/// to the same dying process dies, proving `killed` still kills non-trappers.
#[test]
fn kill_crosses_the_wire_as_killed_and_the_trapping_peer_survives() {
    run_under_watchdog("scenario 3: Kill -> killed across the wire", || {
        block_on_scenario(async {
            let duo = connect_duo().await;

            let pid_a_trap = spawn_idle(&duo.node_a);
            // `set_trap_exit` returns the PREVIOUS flag value.
            let _was_trapping = duo
                .node_a
                .set_trap_exit(pid_a_trap, true)
                .expect("trap flag set");
            assert_eq!(
                duo.node_a.trap_exit(pid_a_trap),
                Some(true),
                "trap_exit enabled on the trapping peer"
            );
            let pid_a_victim = spawn_idle(&duo.node_a);
            let pid_a_canary = spawn_idle(&duo.node_a);

            let pid_b = spawn_idle(&duo.node_b);
            let pid_b_canary = spawn_idle(&duo.node_b);
            duo.node_b
                .link_remote(pid_b, remote(duo.a_atom, pid_a_trap))
                .expect("B links to the trapping A process");
            duo.node_b
                .link_remote(pid_b, remote(duo.a_atom, pid_a_victim))
                .expect("B links to the non-trapping A process");
            duo.node_b
                .link_remote(pid_b_canary, remote(duo.a_atom, pid_a_canary))
                .expect("B's canary links to A's canary target");

            duo.node_b
                .exit_signal(0, pid_b, ExitReason::Kill)
                .expect("Kill delivered on B");
            assert!(
                eventually(|| duo.node_b.process_namespace(pid_b).is_none()).await,
                "Kill terminates B's linker"
            );

            // Sequencing canary: pid_b is gone, so both of its EXIT(killed)
            // controls are on the lane; the canary's EXIT(error) is enqueued
            // strictly after them (DC-5 FIFO). Once the canary's target dies
            // on A, the killed EXITs were applied.
            duo.node_b
                .exit_signal(0, pid_b_canary, ExitReason::Error)
                .expect("canary exit delivered on B");
            assert!(
                eventually(|| duo.node_a.process_namespace(pid_a_canary).is_none()).await,
                "the canary EXIT(error) is delivered and kills its target"
            );

            assert!(
                eventually(|| duo.node_a.process_namespace(pid_a_victim).is_none()).await,
                "killed still kills a non-trapping linked peer"
            );
            assert!(
                duo.node_a.process_namespace(pid_a_trap).is_some(),
                "the wire carried killed, not kill: the trapping peer trapped it and survives"
            );

            duo.listen_b.shutdown();
            duo.node_a.shutdown();
            duo.node_b.shutdown();
        });
    });
}

/// Scenario 4: node death. Dropping the peer's `AcceptHandle` + `Scheduler`
/// closes its sockets; the surviving node's read loop hits EOF (`PeerClosed`)
/// and the down hook delivers `noconnection` to every remote-linked process:
/// the non-trapping one dies, the trapping one traps it and survives.
#[test]
fn node_death_delivers_noconnection_to_every_remote_linked_process() {
    run_under_watchdog("scenario 4: node death -> noconnection", || {
        block_on_scenario(async {
            let duo = connect_duo().await;
            let Duo {
                node_a,
                node_b,
                listen_b,
                atom_table: _,
                a_atom: _,
                b_atom,
            } = duo;

            let pid_dies = spawn_idle(&node_a);
            let pid_traps = spawn_idle(&node_a);
            // `set_trap_exit` returns the PREVIOUS flag value.
            let _was_trapping = node_a
                .set_trap_exit(pid_traps, true)
                .expect("trap flag set");
            assert_eq!(node_a.trap_exit(pid_traps), Some(true), "trap_exit enabled");
            let pid_b1 = spawn_idle(&node_b);
            let pid_b2 = spawn_idle(&node_b);
            node_a
                .link_remote(pid_dies, remote(b_atom, pid_b1))
                .expect("A links its non-trapping process to B");
            node_a
                .link_remote(pid_traps, remote(b_atom, pid_b2))
                .expect("A links its trapping process to B");

            drop_peer(listen_b, node_b);

            assert!(
                eventually(|| node_a.process_namespace(pid_dies).is_none()).await,
                "the non-trapping remote-linked process dies of noconnection"
            );
            assert!(
                eventually(|| {
                    !node_a
                        .try_distribution_connections()
                        .expect("distribution owned")
                        .connected_nodes()
                        .contains(&b_atom)
                })
                .await,
                "A drops the dead node from its connection table"
            );
            // Same down-hook chain delivered to the trapping process; it must
            // have trapped the noconnection, not died of it.
            assert!(
                never_within_window(|| node_a.process_namespace(pid_traps).is_none()).await,
                "the trapping remote-linked process traps noconnection and survives"
            );

            node_a.shutdown();
        });
    });
}

/// Scenario 5: pg purged before noconnection is observed. The composed
/// down-subscriber runs the pg purge synchronously BEFORE the noconnection
/// delivery, so at the very first instant the linked process's death is
/// observable, the dead node's pg members are already gone — asserted with
/// NO polling.
#[test]
fn pg_members_are_purged_before_noconnection_is_observable() {
    run_under_watchdog("scenario 5: pg purge precedes noconnection", || {
        block_on_scenario(async {
            let duo = connect_duo().await;
            let Duo {
                node_a,
                node_b,
                listen_b,
                atom_table,
                a_atom: _,
                b_atom,
            } = duo;

            // B publishes a pg member; A observes it while the link is up.
            let scope = node_b.pg_registry().default_scope();
            let group = atom_table.intern("scenario5_workers");
            let member_pid = 4242_u64;
            node_b.pg_registry().join(scope, group, member_pid);
            let b_member = RemoteMember {
                node: b_atom,
                pid_number: member_pid,
                serial: 0,
            };
            let registry_a = node_a.pg_registry();
            assert!(
                eventually(|| registry_a.remote_members(scope, group).contains(&b_member)).await,
                "A observes B's pg member while the link is up"
            );

            // A non-trapping A process remote-linked over the node: its death
            // is the public observable for "noconnection was delivered".
            let pid_a = spawn_idle(&node_a);
            let pid_b = spawn_idle(&node_b);
            node_a
                .link_remote(pid_a, remote(b_atom, pid_b))
                .expect("A links to B's process");

            drop_peer(listen_b, node_b);

            assert!(
                eventually(|| node_a.process_namespace(pid_a).is_none()).await,
                "the linked process dies of noconnection after the node death"
            );
            // Structural order pin (D6): purge precedes delivery inside one
            // synchronous subscriber body, so no polling is allowed here.
            assert!(
                registry_a.remote_members(scope, group).is_empty(),
                "the dead node's pg members are purged BEFORE its noconnection is observable"
            );

            node_a.shutdown();
        });
    });
}

/// Scenario 6, forward order (the publicly drivable one): a wire EXIT that
/// consumed the link is NOT re-signalled by the node death that follows
/// (DC-4 exactly-once). A `normal` wire EXIT consumes the receiver's link
/// without killing the non-trapping receiver; if the later node-down
/// backstop double-fired, its `noconnection` WOULD kill it — so the process
/// still being alive after the node death is the exactly-once proof.
#[test]
fn consumed_link_is_not_resignalled_as_noconnection_by_node_death() {
    run_under_watchdog("scenario 6 forward: no double-fire", || {
        block_on_scenario(async {
            let duo = connect_duo().await;
            let Duo {
                node_a,
                node_b,
                listen_b,
                atom_table,
                a_atom,
                b_atom,
            } = duo;

            let pid_a = spawn_idle(&node_a);
            let pid_a_canary = spawn_idle(&node_a);
            let pid_b = spawn_exits_normally(&node_b);
            let pid_b_canary = spawn_idle(&node_b);
            node_b
                .link_remote(pid_b, remote(a_atom, pid_a))
                .expect("B links to A's process");
            node_b
                .link_remote(pid_b_canary, remote(a_atom, pid_a_canary))
                .expect("B's canary links to A's canary target");

            // B's linker exits NORMALLY: the wire EXIT(normal) consumes A's
            // link but kills nothing (non-trapping + normal).
            assert!(
                node_b.enqueue_atom_message(pid_b, atom_table.intern("poke")),
                "the poke reaches B's live process"
            );
            assert!(
                eventually(|| node_b.process_namespace(pid_b).is_none()).await,
                "B's linker exits normally"
            );
            // Sequencing canary (DC-5 FIFO + serial apply): once the canary's
            // EXIT(error) has killed its target on A, the normal EXIT enqueued
            // before it was applied — the link is consumed on A.
            node_b
                .exit_signal(0, pid_b_canary, ExitReason::Error)
                .expect("canary exit delivered on B");
            assert!(
                eventually(|| node_a.process_namespace(pid_a_canary).is_none()).await,
                "the canary EXIT is delivered, so the normal EXIT was applied first"
            );
            assert!(
                node_a.process_namespace(pid_a).is_some(),
                "the normal wire EXIT killed nothing"
            );

            // The node death that follows. The backstop must find NO link left
            // for pid_a: a double-fire would deliver noconnection and kill the
            // non-trapping process.
            drop_peer(listen_b, node_b);
            assert!(
                eventually(|| {
                    !node_a
                        .try_distribution_connections()
                        .expect("distribution owned")
                        .connected_nodes()
                        .contains(&b_atom)
                })
                .await,
                "A observes the node death"
            );
            assert!(
                never_within_window(|| node_a.process_namespace(pid_a).is_none()).await,
                "exactly one signal per link (DC-4): the consumed link is not \
                 re-signalled as noconnection"
            );

            node_a.shutdown();
        });
    });
}

/// Scenario 9, e2e half: a hostile (well-framed but undecodable) frame from a
/// connected peer — written through the PUBLIC `DistConnection::write_raw` —
/// is dropped without killing the receiver's read loop: a subsequent pg frame
/// still applies, and a subsequent LINK + EXIT still kills the linked target.
#[test]
fn read_loop_survives_hostile_frame_and_subsequent_frames_deliver() {
    run_under_watchdog("scenario 9 e2e half: hostile frame", || {
        block_on_scenario(async {
            let duo = connect_duo().await;

            // Well-framed garbage: a 4-byte "control" that is not ETF.
            let mut garbage = Vec::new();
            garbage.extend_from_slice(&4_u32.to_be_bytes());
            garbage.extend_from_slice(&0_u32.to_be_bytes());
            garbage.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
            let b_to_a = duo
                .node_b
                .try_distribution_connections()
                .expect("distribution owned")
                .get_connection(duo.a_atom)
                .expect("B holds a connection to A");
            b_to_a
                .write_raw(&garbage)
                .await
                .expect("the hostile frame writes");
            drop(b_to_a);

            // A's read loop must still be alive. First proof: a pg frame from
            // B applies on A.
            let scope = duo.node_b.pg_registry().default_scope();
            let group = duo.atom_table.intern("scenario9_after_garbage");
            let member_pid = 7_u64;
            duo.node_b.pg_registry().join(scope, group, member_pid);
            let b_member = RemoteMember {
                node: duo.b_atom,
                pid_number: member_pid,
                serial: 0,
            };
            let registry_a = duo.node_a.pg_registry();
            assert!(
                eventually(|| registry_a.remote_members(scope, group).contains(&b_member)).await,
                "a pg frame sent after the hostile frame still applies"
            );

            // Second proof: a LINK + EXIT after the garbage still kills the
            // linked target through the full inbound path.
            let pid_a = spawn_idle(&duo.node_a);
            let pid_b = spawn_idle(&duo.node_b);
            duo.node_b
                .link_remote(pid_b, remote(duo.a_atom, pid_a))
                .expect("B links to A's process after the hostile frame");
            duo.node_b
                .exit_signal(0, pid_b, ExitReason::Error)
                .expect("exit signal delivered on B");
            assert!(
                eventually(|| duo.node_a.process_namespace(pid_a).is_none()).await,
                "a wire EXIT sent after the hostile frame still delivers"
            );

            duo.listen_b.shutdown();
            duo.node_a.shutdown();
            duo.node_b.shutdown();
        });
    });
}
