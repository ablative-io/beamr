//! Distribution identity primitives, node resolution, and connection management.

pub mod atom_cache;
pub mod connection;
pub mod connection_events;
pub mod control;
pub mod control_link;
pub mod etf;
pub mod global;
pub mod handshake;
mod node;
pub mod pg;
pub mod remote_link;
pub mod resolver;
pub mod sender;

pub use connection::ConnectionManager;
pub use connection_events::{
    ConnectionEvent, ConnectionGeneration, NodeDown, NodeUp, SubscriberId,
};
pub use node::{DEFAULT_NODE_NAME, Node};

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tokio::runtime::Runtime;

pub use resolver::{NodeResolver, ResolveError, ResolveFuture, Resolver, StaticResolver};

/// Default distribution authentication cookie used when none is configured.
pub const DEFAULT_COOKIE: &str = "beamr-cookie";

/// Whole-attempt deadline for a blocking [`NetKernel::connect_node`] call:
/// resolver + TCP connect + handshake together.
///
/// The transport stages carry their own 5-second deadlines, but the
/// embedder-provided [`NodeResolver`] future has no completion bound of its
/// own, and `connect_node` holds the net-kernel runtime guard for the whole
/// attempt — unbounded, one stalled resolver would wedge
/// [`NetKernel::shutdown`] (and `Scheduler::shutdown` with it) forever. Sized
/// to cover all three stages' own deadlines without changing the behavior of
/// any attempt that was going to succeed.
pub const NET_KERNEL_CONNECT_DEADLINE: Duration = Duration::from_secs(15);

/// OS thread name of the net-kernel runtime's single worker.
///
/// Set as the runtime's `thread_name` (spec §5 naming defect: the worker was
/// previously unnamed, taking tokio's default), so it is also the name the OS
/// thread probe and the service inventory attribute the worker under.
pub const NET_KERNEL_THREAD_NAME: &str = "beamr-net-kernel";

/// Configuration for beamr distribution services.
#[derive(Clone)]
pub struct DistributionConfig {
    /// Resolver used to map node names to distribution listen addresses.
    pub resolver: Resolver,
    /// Shared secret presented in the OTP handshake challenge/response. Both
    /// peers must agree on this value or the handshake is rejected.
    pub cookie: String,
}

/// The net-kernel's owned tokio [`Runtime`], isolated behind interior
/// mutability so [`NetKernel::shutdown`] can take it through a shared `&self`
/// and hand it to [`join_runtime_drop`] (spec §4), which joins the
/// "beamr-net-kernel" worker before returning from every context except that
/// runtime's own thread (see its three-context docs). The take happens in its
/// own statement so this mutex is never held across the join —
/// [`NetKernel::worker_thread_names`] locks it from worker-side contexts. Held
/// behind an `Arc` so [`NetKernel`] stays cheap to clone while the single
/// runtime is shared; whichever holder shuts it down first empties the slot
/// for all.
struct NetKernelRuntime {
    /// `Some` for a live runtime, `None` once shut down or if the build failed.
    runtime: Mutex<Option<Runtime>>,
    /// This instance's [`mint_runtime_mark`] identity, stamped on its threads.
    mark: u64,
}

impl NetKernelRuntime {
    fn shutdown(&self) {
        // Take the runtime in its OWN statement so the mutex guard drops before
        // the blocking join: `worker_thread_names()` (inventory) locks this same
        // mutex, and a worker-side task blocked on it while shutdown waits for
        // that worker is a lock-inversion deadlock.
        let runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        join_runtime_drop(runtime, self.mark);
    }
}

impl Drop for NetKernelRuntime {
    fn drop(&mut self) {
        // Safety net for a net-kernel never explicitly shut down (spec §4).
        // After `shutdown()` this is a no-op (the slot is already `None`).
        // `get_mut` touches no lock, so the shutdown deadlock shape can't occur.
        let runtime = self
            .runtime
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        join_runtime_drop(runtime, self.mark);
    }
}

/// Source of process-unique marks identifying one owned runtime INSTANCE —
/// thread names cannot: every sender runtime is named "beamr-dist-send", so a
/// name check misclassifies another scheduler's same-named worker as self.
static RUNTIME_MARK_COUNTER: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// The runtime mark of the owned runtime this thread belongs to (worker or
    /// blocking pool — `on_thread_start` runs for both), or 0 for every other
    /// thread. Thread-accurate identity: immune to `Handle::enter` nesting.
    static RUNTIME_MARK: Cell<u64> = const { Cell::new(0) };
}

/// Mint a mark for one owned runtime instance.
pub(crate) fn mint_runtime_mark() -> u64 {
    RUNTIME_MARK_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Stamp every thread the runtime under construction spawns (workers AND the
/// blocking pool) with `mark`, so [`join_runtime_drop`] can identify the true
/// self-runtime case by instance, not by display name.
pub(crate) fn stamp_runtime_threads(builder: &mut tokio::runtime::Builder, mark: u64) {
    builder.on_thread_start(move || RUNTIME_MARK.with(|slot| slot.set(mark)));
}

/// Shut a tokio [`Runtime`] down, JOINING its worker whenever a join cannot
/// deadlock. `None` is a no-op. Replaces the former unjoined-helper-thread
/// drop (spec §4). `own_mark` is the runtime's [`mint_runtime_mark`] identity,
/// stamped on its threads via [`stamp_runtime_threads`].
///
/// Three contexts, three teardowns:
///
/// - **This runtime's own thread** (worker or blocking pool, identified by the
///   thread's stamped mark — a per-INSTANCE identity, so another scheduler's
///   same-named worker does NOT land here): waiting for the runtime's exit
///   from inside it is a self-join deadlock no helper thread can break
///   (reachable — the runtime handle is exposed, so a spawned task can carry
///   the owner and trigger shutdown or the final-clone drop).
///   `shutdown_background` is the only non-deadlocking teardown: the worker
///   exits promptly on its own, just not synchronously.
/// - **Any other async context** — another beamr runtime's thread included: a
///   blocking `Runtime` drop would panic here ("Cannot drop a runtime..."),
///   but `thread::join` of a helper that OWNS the drop is legal and preserves
///   the §4 synchronous-join guarantee — the worker is gone before this
///   returns.
/// - **A synchronous context** — the scheduler-owner shutdown path §4 binds:
///   inline blocking drop, worker joined before return.
pub(crate) fn join_runtime_drop(runtime: Option<Runtime>, own_mark: u64) {
    let Some(runtime) = runtime else {
        return;
    };
    if RUNTIME_MARK.with(Cell::get) == own_mark {
        runtime.shutdown_background();
    } else if tokio::runtime::Handle::try_current().is_ok() {
        let joiner = thread::spawn(move || drop(runtime));
        // A failed join means the helper panicked dropping the runtime; there
        // is nothing to recover, and the worker has still exited.
        let _ = joiner.join();
    } else {
        drop(runtime);
    }
}

/// Synchronous net-kernel facade used by native BIFs.
///
/// Owns a single-worker tokio [`Runtime`] used to drive blocking `connect_node`
/// calls from synchronous BIF code. Cheap to clone: the connection manager and
/// the runtime handle are both `Arc`-backed, so every clone shares the one
/// runtime and connection table.
#[derive(Clone)]
pub struct NetKernel {
    connections: ConnectionManager,
    runtime: Arc<NetKernelRuntime>,
}

impl NetKernel {
    /// Create a facade backed by a distribution connection manager.
    #[must_use]
    pub fn new(connections: ConnectionManager) -> Self {
        let mark = mint_runtime_mark();
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder
            .worker_threads(1)
            .thread_name(NET_KERNEL_THREAD_NAME)
            .enable_all();
        stamp_runtime_threads(&mut builder, mark);
        let runtime = builder.build().ok();
        Self {
            connections,
            runtime: Arc::new(NetKernelRuntime {
                runtime: Mutex::new(runtime),
                mark,
            }),
        }
    }

    /// Return the backing connection manager.
    #[must_use]
    pub fn connection_manager(&self) -> &ConnectionManager {
        &self.connections
    }

    /// OS thread names of the net-kernel runtime workers (spec §5 inventory).
    ///
    /// One worker, named [`NET_KERNEL_THREAD_NAME`], while the runtime is live;
    /// empty when it could not be built (then `connect_node` returns `false`) or
    /// after [`shutdown`](Self::shutdown) has joined it — so the post-shutdown
    /// inventory is truthful. The lazily-spawned blocking pool is not live at
    /// rest, so it is not reported.
    #[must_use]
    pub fn worker_thread_names(&self) -> Vec<String> {
        if self
            .runtime
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
        {
            vec![NET_KERNEL_THREAD_NAME.to_owned()]
        } else {
            Vec::new()
        }
    }

    /// Stop the net-kernel: synchronously JOIN the runtime worker before
    /// returning (spec §4). Idempotent. An in-flight [`connect_node`](Self::connect_node)
    /// holds the runtime guard, so shutdown blocks until it completes rather than
    /// dropping the runtime out from under it.
    pub fn shutdown(&self) {
        self.runtime.shutdown();
    }

    /// Connect to `node`, mapping all connection failures — including exceeding
    /// [`NET_KERNEL_CONNECT_DEADLINE`] — to `false`.
    pub fn connect_node(&self, node: crate::atom::Atom) -> bool {
        self.connect_node_with_deadline(node, NET_KERNEL_CONNECT_DEADLINE)
    }

    /// [`connect_node`](Self::connect_node) with an explicit whole-attempt
    /// deadline, exposed so tests can pin the bounded-shutdown property without
    /// waiting out the production deadline.
    #[cfg(any(test, feature = "test-support"))]
    pub fn connect_node_deadline_for_test(
        &self,
        node: crate::atom::Atom,
        deadline: Duration,
    ) -> bool {
        self.connect_node_with_deadline(node, deadline)
    }

    fn connect_node_with_deadline(&self, node: crate::atom::Atom, deadline: Duration) -> bool {
        if self.connections.get_connection(node).is_some() {
            return true;
        }

        // Hold the runtime guard across `block_on`: this serialises the rare
        // `connect_node` calls and makes a concurrent `shutdown()` wait for an
        // in-flight connect rather than dropping the runtime under it. `None`
        // means the runtime never built or was shut down — no node to reach.
        //
        // The WHOLE attempt is bounded by `deadline`: the per-stage transport
        // timeouts (5 s TCP connect, 5 s handshake) do not cover the
        // embedder-provided resolver, whose `resolve` future carries no
        // completion bound of its own — unbounded, a stalled resolver would
        // hold this guard forever and wedge `shutdown()` (and with it
        // `Scheduler::shutdown`) permanently. Timing out drops the connect
        // future, cancelling whichever stage was pending.
        let guard = self
            .runtime
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(runtime) = guard.as_ref() else {
            return false;
        };
        let connections = self.connections.clone();
        let connect = || async {
            tokio::time::timeout(deadline, connections.connect_node(node))
                .await
                .unwrap_or(false)
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            thread::scope(|scope| {
                scope
                    .spawn(|| runtime.block_on(connect()))
                    .join()
                    .unwrap_or(false)
            })
        } else {
            runtime.block_on(connect())
        }
    }

    /// Return node-name atoms for active connections.
    #[must_use]
    pub fn nodes(&self) -> Vec<crate::atom::Atom> {
        self.connections.connected_nodes()
    }

    /// Disconnect `node` if connected. Missing connections are already disconnected.
    pub fn disconnect_node(&self, node: crate::atom::Atom) -> bool {
        self.connections.disconnect_node(node)
    }
}

impl fmt::Debug for NetKernel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetKernel")
            .field("connection_count", &self.connections.connection_count())
            .finish()
    }
}

impl Default for DistributionConfig {
    fn default() -> Self {
        Self {
            resolver: Arc::new(StaticResolver::new(HashMap::new())),
            cookie: DEFAULT_COOKIE.to_owned(),
        }
    }
}

impl fmt::Debug for DistributionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DistributionConfig")
            .field("resolver", &"<node resolver>")
            .field("cookie", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod connection_events_tests;
#[cfg(test)]
mod pg_tests;

#[cfg(test)]
mod net_kernel_tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use super::*;
    use crate::atom::AtomTable;
    use crate::distribution::connection::ConnectionManager;
    use crate::distribution::resolver::{NodeResolver, ResolveFuture};

    /// Resolver whose future never completes; flags when it has been polled so
    /// the test can force the wedge interleaving deterministically.
    struct StalledResolver {
        entered: Arc<AtomicBool>,
    }

    impl NodeResolver for StalledResolver {
        fn resolve<'a>(&'a self, _name: &'a str) -> ResolveFuture<'a> {
            self.entered.store(true, Ordering::Release);
            Box::pin(std::future::pending())
        }
    }

    #[test]
    fn stalled_resolver_cannot_wedge_net_kernel_shutdown() {
        // `connect_node` holds the runtime guard for the whole attempt, and the
        // embedder resolver has no completion bound of its own — without the
        // whole-attempt deadline, one stalled resolver holds the guard forever
        // and `shutdown()` (hence `Scheduler::shutdown`) never returns.
        let atom_table = Arc::new(AtomTable::with_common_atoms());
        let entered = Arc::new(AtomicBool::new(false));
        let manager = ConnectionManager::new(
            Arc::clone(&atom_table),
            Arc::new(StalledResolver {
                entered: Arc::clone(&entered),
            }),
            "test-cookie",
            "local@test",
            0,
        );
        let net_kernel = NetKernel::new(manager);
        let node = atom_table.intern("peer@stalled");

        let on_thread = net_kernel.clone();
        let connect = thread::spawn(move || {
            on_thread.connect_node_deadline_for_test(node, Duration::from_millis(200))
        });
        // Wait until the resolver is genuinely pending INSIDE the guarded
        // block_on, so shutdown below contends with a live wedge, not a
        // not-yet-started connect.
        let poll_deadline = Instant::now() + Duration::from_secs(10);
        while !entered.load(Ordering::Acquire) {
            assert!(
                Instant::now() < poll_deadline,
                "resolver never entered; connect thread failed to start"
            );
            thread::sleep(Duration::from_millis(2));
        }

        let started = Instant::now();
        net_kernel.shutdown();
        // Bound generously (deadline is 200ms): the property is "finite", the
        // margin only guards slow hosts.
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "shutdown must complete once the connect deadline fires"
        );
        assert!(
            !connect.join().unwrap_or(true),
            "a timed-out connect reports false"
        );
        assert!(
            net_kernel.worker_thread_names().is_empty(),
            "post-shutdown net-kernel inventory reports no live worker"
        );
    }
}
