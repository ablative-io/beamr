//! Async outbound distribution sender.
//!
//! [`DistSender`] owns all outbound distribution I/O on a dedicated single-worker
//! tokio runtime. Callers ENQUEUE a pre-encoded frame and return immediately —
//! they never `block_on` on a scheduler worker thread, so a slow or dead peer can
//! never stall a worker. A single drain task serialises writes per connection
//! (preserving per-node FIFO ordering) behind the connection's writer `Mutex`.
//!
//! ## `Arc`-cycle avoidance (the load-bearing invariant)
//!
//! The drain task closure captures **only** the [`ConnectionManager`] (an
//! `Arc<ConnectionManagerInner>`), never `Arc<SharedState>`. `DistSender` is held
//! by `SharedState`, and the producers (`SchedulerPgPropagation`) reach the sender
//! via `SharedState::dist_sender` after upgrading their own `Weak<SharedState>`.
//! Because the sender holds the connection table — not the scheduler — there is no
//! `SharedState -> DistSender -> SharedState` cycle, and the scheduler still drops
//! cleanly.
//!
//! ## Backpressure
//!
//! The data queue is bounded ([`DIST_SEND_QUEUE_CAP`]). [`DistSender::enqueue`]
//! uses a non-blocking `try_send`: on a full or closed channel the frame is
//! DROPPED. A dropped membership update is self-correcting — the next `pg`
//! join/leave or a node-down purge re-establishes the correct view — so
//! dropping is safe and is strictly preferable to blocking a scheduler worker
//! behind a stalled peer.
//!
//! ## Must-deliver control lane — DC-1 (send-or-down; no silent arm)
//!
//! LINK/UNLINK/EXIT/EXIT2 controls MUST NOT be silently dropped: a lost EXIT is
//! a lost death signal. They travel a second bounded channel
//! ([`DIST_CONTROL_QUEUE_CAP`]) via [`DistSender::enqueue_control`], with the
//! contract that for every enqueued control C against pinned connection G,
//! exactly one of: (a) C is written to G's socket in per-node FIFO order
//! (DC-5: one bounded channel, one drain task, one per-connection writer
//! mutex); or (b) G is marked down. Loss-path table: lane full ⇒
//! `mark_down_control_overflow` at enqueue; write error ⇒ `write_raw` marks
//! down; write timeout ⇒ `mark_down_write_timeout`; encode failure (producer
//! side) ⇒ `mark_down_control_overflow`. Down ⇒ connection-down hook ⇒ pg
//! purge + noconnection delivery to every locally-linked process (DC-3), so a
//! lost link control is never a lost signal — it is coarsened to
//! `noconnection`. Both-sides convergence: our `mark_down` wakes the read loop
//! which drops its read half closing the socket, so the peer sees EOF, a write
//! error, or its heartbeat deadline — its own hook fires within a bounded
//! window.
//!
//! The single drain prefers the control lane (`tokio::select!` with `biased`):
//! controls are small and latency-sensitive, and preferring them empties the
//! bounded lane fastest. Accepted v1 blast radius: a wedged peer can hold the
//! drain up to [`WRITE_TIMEOUT`], during which a control to a HEALTHY peer may
//! overflow, marking the healthy peer down — a spurious noconnection + redial
//! (availability blip), never a lost signal.
//!
//! ## Generation pinning — DC-2
//!
//! [`ControlOutbound`] pins the `Arc<DistConnection>` (the connection
//! GENERATION) the control was enqueued against. The drain writes ONLY to that
//! pinned connection and skips it once down (`is_down`) — a control can never
//! leak onto a post-redial socket (the data lane's by-node resolve at write
//! time is exactly the hazard this closes). Corollary: after any down+redial,
//! cross-node link state between the pair starts empty on both sides (all
//! links noconnection'd at down) and must be re-established by fresh LINKs.
//!
//! ## Async-safe runtime drop
//!
//! The owned tokio [`Runtime`] performs a BLOCKING shutdown when dropped and
//! panics ("Cannot drop a runtime in a context where blocking is not allowed")
//! if that drop happens inside an async context. Because the last [`DistSender`]
//! `Arc` can resolve anywhere — a scheduler worker, the main thread, or a
//! `#[tokio::test]` task — `DistSenderInner` holds the runtime in an `Option` and
//! its [`Drop`] moves the shutdown onto a dedicated `std::thread`. The blocking
//! drop therefore always runs off any async context and can never panic,
//! independent of where the final reference was released.
//!
//! ## Wedged-peer write deadline
//!
//! The single drain serialises writes across all peers, so one peer that is
//! TCP-connected but never reads (kernel send buffer full) would stall
//! propagation cluster-wide. Each write is bounded by [`WRITE_TIMEOUT`]; on
//! elapse the connection is marked down (firing the connection-down hook and
//! remote-node purge) and the drain proceeds.

use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::atom::Atom;
use crate::distribution::connection::{ConnectionManager, DistConnection};

/// OS thread name of the sender's single tokio worker.
///
/// Set as the runtime's `thread_name`, so it is also the name the OS thread
/// probe and the service inventory (spec §5) attribute the worker under.
pub const DIST_SEND_THREAD_NAME: &str = "beamr-dist-send";

/// Bounded depth of the outbound distribution queue.
///
/// Sized for low-frequency control traffic (pg join/leave). When full, the
/// producer drops rather than blocks; see the module docs.
pub const DIST_SEND_QUEUE_CAP: usize = 1024;

/// Bounded depth of the must-deliver control lane.
///
/// Small on purpose: a peer that cannot absorb this many pending LINK/EXIT
/// controls is effectively down (DC-1), and `enqueue_control` marks the pinned
/// connection down on overflow rather than dropping the control silently.
pub const DIST_CONTROL_QUEUE_CAP: usize = 256;

/// Per-frame write deadline for the drain task.
///
/// A peer that is TCP-connected but never reads fills the kernel send buffer,
/// after which `write_all` parks indefinitely (until OS keepalive, ~2h). Because
/// the single drain serialises all peers, one wedged peer would otherwise stall
/// pg propagation for the entire cluster. Bounding each write at this deadline
/// turns a wedged peer into an ordinary write failure: the connection is marked
/// down (firing the connection-down hook and remote-node purge) and the drain
/// moves on. Sized generously relative to control-frame size so a merely-slow
/// (not wedged) peer is not spuriously torn down.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// A unit of outbound distribution work.
///
/// The frame is fully ETF-encoded by the producer (on the calling worker
/// thread); the drain task performs only TCP I/O.
#[derive(Clone, Debug)]
pub enum DistOutbound {
    /// Send a pre-encoded frame to a single connected node.
    ToNode {
        /// Destination node-name atom.
        node: Atom,
        /// Pre-encoded control frame (`Arc`-shared so a fan-out broadcast clones
        /// the handle, not the bytes).
        frame: Arc<[u8]>,
    },
}

/// A control frame pinned to the connection GENERATION it was enqueued
/// against (DC-2).
///
/// The drain writes only to this connection and skips it once down — a control
/// can never leak onto a post-redial socket (the data lane's by-node resolve
/// at write time is exactly the hazard this closes).
#[derive(Clone)]
pub struct ControlOutbound {
    /// The pinned connection generation. Holding the `Arc<DistConnection>`
    /// directly is safe for the Arc-cycle invariant: `DistConnection::manager`
    /// is already `Weak`, and the bounded lane bounds retained `Arc`s.
    pub connection: Arc<DistConnection>,
    /// Pre-encoded control frame (encoded by the producer on the calling
    /// worker thread; the drain performs only TCP I/O).
    pub frame: Arc<[u8]>,
}

/// Why [`DistSender::enqueue_control`] did not accept a control frame.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ControlEnqueueError {
    /// Lane full — `enqueue_control` has ALREADY marked the pinned connection
    /// down (`ControlOverflow`) before returning; the caller needs no further
    /// action (the down-hook's noconnection backstop supplies the signals).
    Overflow,
    /// Sender shut down (scheduler teardown); peers converge via EOF.
    Closed,
}

struct DistSenderInner {
    /// Owned single-worker runtime. Drives the drain task and, via
    /// [`DistSender::handle`], the connection manager's read/accept tasks.
    /// Dropped when the last [`DistSender`] clone drops.
    ///
    /// Stored as `Option` so [`Drop`] can `take()` it and move the (blocking)
    /// runtime shutdown onto a dedicated `std::thread`. Dropping a tokio
    /// `Runtime` performs a BLOCKING shutdown and panics if it happens inside an
    /// async context; the dedicated-thread idiom guarantees the drop always runs
    /// off any async context — whether the last `Arc` resolves on a scheduler
    /// worker, the main thread, or inside a `#[tokio::test]` task. Always `Some`
    /// for a live `DistSenderInner`; only `None` transiently during `drop`.
    runtime: Option<Runtime>,
    /// Cached handle to `runtime`, kept independently of the `Option` so
    /// [`DistSender::handle`] never has to inspect (or risk a `None` from) the
    /// drop-only `runtime` field. Cloning a `Handle` does not keep the runtime
    /// alive, so this does not interfere with the dedicated-thread drop.
    handle: Handle,
    /// Drain task handle, used to abort the loop on shutdown before the runtime
    /// is dropped.
    drain: JoinHandle<()>,
}

impl Drop for DistSenderInner {
    fn drop(&mut self) {
        // Abort the drain loop so the runtime has no in-flight task to wind down.
        // `shutdown()` is the public path and is idempotent (a second abort is a
        // no-op); calling it here also covers the case where the last `Arc`
        // resolves without a prior explicit `shutdown()`.
        self.drain.abort();
        // Move the blocking runtime shutdown OFF any async context. A tokio
        // `Runtime` drop blocks and panics ("Cannot drop a runtime in a context
        // where blocking is not allowed") if performed from within an async
        // context. Spawning a plain `std::thread` to own the drop guarantees it
        // runs on a non-async thread, so it can never panic regardless of where
        // the final `Arc` was released.
        if let Some(runtime) = self.runtime.take() {
            std::thread::spawn(move || drop(runtime));
        }
    }
}

/// Async outbound distribution sender.
///
/// Cheap to clone (an `mpsc::Sender` plus an `Arc`). All clones share the one
/// runtime, queue, and drain task.
#[derive(Clone)]
pub struct DistSender {
    tx: mpsc::Sender<DistOutbound>,
    control_tx: mpsc::Sender<ControlOutbound>,
    inner: Arc<DistSenderInner>,
}

impl DistSender {
    /// Build a sender owning a dedicated single-worker tokio runtime and spawn its
    /// drain task. Returns `None` only if the runtime could not be created.
    #[must_use]
    pub fn new(connections: ConnectionManager) -> Option<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name(DIST_SEND_THREAD_NAME)
            .enable_all()
            .build()
            .ok()?;
        let (tx, mut rx) = mpsc::channel::<DistOutbound>(DIST_SEND_QUEUE_CAP);
        let (control_tx, mut control_rx) = mpsc::channel::<ControlOutbound>(DIST_CONTROL_QUEUE_CAP);
        // The drain closure captures ONLY `connections` (an
        // `Arc<ConnectionManagerInner>`) and the receivers — never
        // `Arc<SharedState>`. This is the Arc-cycle invariant: the sender must
        // not transitively own the scheduler.
        let drain = runtime.spawn(async move {
            let mut control_open = true;
            let mut data_open = true;
            // One task drains both lanes; `biased` polls the control lane
            // first, so controls are preferred whenever both are ready (they
            // are small, latency-sensitive, and must-deliver — preferring them
            // empties the bounded control lane fastest). The loop exits only
            // when BOTH channels are closed.
            while control_open || data_open {
                tokio::select! {
                    biased;
                    item = control_rx.recv(), if control_open => match item {
                        Some(item) => {
                            // DC-2: write ONLY to the pinned connection; once it
                            // is down the control is skipped, never re-resolved
                            // by node onto a post-redial socket.
                            if item.connection.is_down() {
                                continue;
                            }
                            // Identical failure discipline to the data lane: a
                            // write error marks down inside `write_raw`; a
                            // timeout has not observed a failure, so mark down
                            // explicitly to drive the same down path (DC-1(b)).
                            if tokio::time::timeout(
                                WRITE_TIMEOUT,
                                item.connection.write_raw(&item.frame),
                            )
                            .await
                            .is_err()
                            {
                                item.connection.mark_down_write_timeout();
                            }
                        }
                        None => control_open = false,
                    },
                    item = rx.recv(), if data_open => match item {
                        Some(DistOutbound::ToNode { node, frame }) => {
                            // CONNECTED-ONLY: look up an already-established
                            // connection; never trigger an inline reconnect
                            // from the send path.
                            if let Some(connection) = connections.get_connection(node) {
                                // Bound each write so a wedged peer
                                // (TCP-connected but never reading, kernel send
                                // buffer full) cannot park the single drain
                                // indefinitely and stall propagation for every
                                // other peer. On success or write error the
                                // result is ignored: `write_raw` already marks
                                // the connection down on a write error, firing
                                // the connection-down hook and remote purge. On
                                // timeout, `write_raw` has NOT observed a
                                // failure, so we explicitly mark the connection
                                // down here to drive the same down path; the
                                // drain then moves on to the next frame.
                                if tokio::time::timeout(WRITE_TIMEOUT, connection.write_raw(&frame))
                                    .await
                                    .is_err()
                                {
                                    connection.mark_down_write_timeout();
                                }
                            }
                        }
                        None => data_open = false,
                    },
                }
            }
        });
        let handle = runtime.handle().clone();
        Some(Self {
            tx,
            control_tx,
            inner: Arc::new(DistSenderInner {
                runtime: Some(runtime),
                handle,
                drain,
            }),
        })
    }

    /// A clone of the owned runtime's handle, for binding the connection
    /// manager's read/accept tasks to this runtime (so the receive side is driven
    /// in production, where there is no ambient runtime).
    #[must_use]
    pub fn handle(&self) -> Handle {
        self.inner.handle.clone()
    }

    /// OS thread names of the sender's runtime workers (spec §5 inventory).
    ///
    /// A live sender owns exactly one worker, named [`DIST_SEND_THREAD_NAME`].
    /// The lazily-spawned blocking pool is not live at rest, so it is not
    /// reported.
    #[must_use]
    pub fn worker_thread_names(&self) -> Vec<String> {
        vec![DIST_SEND_THREAD_NAME.to_owned()]
    }

    /// Enqueue an outbound frame. NON-BLOCKING: on a full or closed queue the
    /// frame is dropped (see module docs on backpressure). Never blocks the
    /// calling thread, so it is safe to call from a scheduler worker.
    pub fn enqueue(&self, item: DistOutbound) {
        // `try_send` returns `Err` on `Full` or `Closed`; both are dropped.
        let _ = self.tx.try_send(item);
    }

    /// NON-BLOCKING must-deliver enqueue for LINK/UNLINK/EXIT/EXIT2 controls
    /// (`try_send`). Full lane ⇒ mark the PINNED connection down
    /// (`ControlOverflow`), then return [`ControlEnqueueError::Overflow`] —
    /// DC-1 has no silent-drop arm. Never blocks the calling thread, so it is
    /// safe from scheduler workers; the connection-down hook may run inline on
    /// the caller (a supported context — the same as `ManualDisconnect`).
    /// The overflow `mark_down` is invoked holding no shard guard (the `Arc`
    /// is owned).
    pub fn enqueue_control(&self, item: ControlOutbound) -> Result<(), ControlEnqueueError> {
        match self.control_tx.try_send(item) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(item)) => {
                item.connection.mark_down_control_overflow();
                Err(ControlEnqueueError::Overflow)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(ControlEnqueueError::Closed),
        }
    }

    /// Abort the drain task. Called during scheduler shutdown before worker
    /// threads are joined; the owned runtime is then dropped with `SharedState`.
    pub fn shutdown(&self) {
        self.inner.drain.abort();
    }
}

// FUTURE: per-node sub-channels if a single drain becomes a head-of-line
// bottleneck for a hot peer. Not needed for low-frequency pg control traffic.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    use super::*;
    use crate::atom::AtomTable;
    use crate::distribution::connection::{ConnectionDownReason, ConnectionManager};
    use crate::distribution::resolver::StaticResolver;

    fn manager() -> (ConnectionManager, Arc<AtomTable>) {
        let atom_table = Arc::new(AtomTable::with_common_atoms());
        let resolver = Arc::new(StaticResolver::new(HashMap::new()));
        (
            ConnectionManager::new(
                Arc::clone(&atom_table),
                resolver,
                "test-cookie",
                "local@test",
                0,
            ),
            atom_table,
        )
    }

    /// A length-prefixed frame the read-lifecycle parser accepts: 8-byte header
    /// (control_len, payload_len) followed by `control_len + payload_len` bytes.
    fn framed(control: &[u8]) -> Arc<[u8]> {
        let control_len = u32::try_from(control.len()).expect("control fits u32");
        let mut frame = Vec::with_capacity(8 + control.len());
        frame.extend_from_slice(&control_len.to_be_bytes());
        frame.extend_from_slice(&0u32.to_be_bytes());
        frame.extend_from_slice(control);
        Arc::from(frame.into_boxed_slice())
    }

    /// Enqueue never blocks and silently drops once the bounded queue is full,
    /// even with no peer connected and the drain unable to make progress.
    #[test]
    fn enqueue_is_non_blocking_and_drops_when_full() {
        let (connections, atom_table) = manager();
        let sender = DistSender::new(connections).expect("sender builds");
        let node = atom_table.intern("absent@127.0.0.1");

        // Far more than the queue capacity. With no connection the drain drops
        // each item, but even if it stalled, `enqueue` must return promptly for
        // every call and never panic.
        for index in 0..(DIST_SEND_QUEUE_CAP * 4) {
            sender.enqueue(DistOutbound::ToNode {
                node,
                frame: framed(&index.to_be_bytes()),
            });
        }
        sender.shutdown();
    }

    /// Frames enqueued for one node arrive at that node in FIFO order: the single
    /// drain plus the per-connection writer `Mutex` serialise writes.
    ///
    /// Single-threaded `#[tokio::test]` deliberately: it also exercises FIX 1 by
    /// letting the owned `DistSender` drop directly inside this async context. The
    /// `DistSenderInner::Drop` impl moves the blocking runtime shutdown onto a
    /// dedicated `std::thread`, so the drop must NOT panic even here, where there
    /// is no `block_in_place` escape hatch and a naive runtime drop would abort.
    #[tokio::test]
    async fn per_node_fifo_ordering() {
        let (connections, atom_table) = manager();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        // Peer side: read every frame and record its 1-byte control sequence id.
        let received = Arc::new(Mutex::new(Vec::new()));
        let received_for_task = Arc::clone(&received);
        let count = 16usize;
        let reader = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            for _ in 0..count {
                let mut header = [0u8; 8];
                if stream.read_exact(&mut header).await.is_err() {
                    break;
                }
                let control_len =
                    u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
                let payload_len =
                    u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
                let mut body = vec![0u8; control_len + payload_len];
                if stream.read_exact(&mut body).await.is_err() {
                    break;
                }
                received_for_task
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(body[0]);
            }
        });

        // Register the accepted client stream as a connection on the manager.
        let std_stream = std::net::TcpStream::connect(addr).expect("client connects");
        let node = atom_table.intern("peer@127.0.0.1");
        let peer_addr: SocketAddr = std_stream.peer_addr().expect("peer addr");
        connections
            .register_test_connection(node, peer_addr, std_stream)
            .expect("register test connection");

        let sender = DistSender::new(connections).expect("sender builds");
        for index in 0..count {
            let seq = u8::try_from(index).expect("seq fits u8");
            sender.enqueue(DistOutbound::ToNode {
                node,
                frame: framed(&[seq]),
            });
        }

        reader.await.expect("reader task joins");
        let order = received
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let expected: Vec<u8> = (0..count).map(|i| i as u8).collect();
        assert_eq!(order, expected, "frames must arrive in enqueue order");
        sender.shutdown();
        // Drop directly in the async context. FIX 1 (the dedicated-thread runtime
        // drop) is what keeps this from panicking — see the test doc comment.
        drop(sender);
    }

    /// A dead peer (write half closed) does not stall the drain: a second, live
    /// node still receives its frame, and the dead connection's down-hook fires.
    ///
    /// Single-threaded `#[tokio::test]`, like `per_node_fifo_ordering`, so the
    /// final direct `drop(sender)` also proves FIX 1's async-safe runtime drop.
    #[tokio::test]
    async fn dead_peer_does_not_stall_drain() {
        let (connections, atom_table) = manager();
        let down_count = Arc::new(AtomicUsize::new(0));
        let down_for_hook = Arc::clone(&down_count);
        connections.register_connection_down(move |_| {
            down_for_hook.fetch_add(1, Ordering::SeqCst);
        });

        // Dead node: connect a stream, then drop the peer's read half so writes
        // eventually fail and mark it down.
        let dead_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind dead");
        let dead_addr = dead_listener.local_addr().expect("dead addr");
        let dead_node = atom_table.intern("dead@127.0.0.1");
        let dead_stream = std::net::TcpStream::connect(dead_addr).expect("dead connects");
        let dead_peer_addr = dead_stream.peer_addr().expect("dead peer addr");
        let dead_accept = tokio::spawn(async move { dead_listener.accept().await });
        connections
            .register_test_connection(dead_node, dead_peer_addr, dead_stream)
            .expect("register dead connection");
        let accepted = dead_accept
            .await
            .expect("dead accept join")
            .expect("accepted");
        drop(accepted); // close the peer so writes to `dead_node` fail.

        // Live node: a real reader that records what it receives.
        let live_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind live");
        let live_addr = live_listener.local_addr().expect("live addr");
        let live_received = Arc::new(Mutex::new(Vec::new()));
        let live_for_task = Arc::clone(&live_received);
        let live_reader = tokio::spawn(async move {
            let (mut stream, _) = live_listener.accept().await.expect("live accept");
            let mut header = [0u8; 8];
            if stream.read_exact(&mut header).await.is_ok() {
                let control_len =
                    u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
                let payload_len =
                    u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
                let mut body = vec![0u8; control_len + payload_len];
                if stream.read_exact(&mut body).await.is_ok() {
                    live_for_task
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push(body[0]);
                }
            }
        });
        let live_stream = std::net::TcpStream::connect(live_addr).expect("live connects");
        let live_node = atom_table.intern("live@127.0.0.1");
        let live_peer_addr = live_stream.peer_addr().expect("live peer addr");
        connections
            .register_test_connection(live_node, live_peer_addr, live_stream)
            .expect("register live connection");

        let sender = DistSender::new(connections.clone()).expect("sender builds");
        // Many frames to the dead node first, then one to the live node. If the
        // drain stalled on the dead peer, the live frame would never arrive.
        for index in 0..32u8 {
            sender.enqueue(DistOutbound::ToNode {
                node: dead_node,
                frame: framed(&[index]),
            });
        }
        sender.enqueue(DistOutbound::ToNode {
            node: live_node,
            frame: framed(&[0xAB]),
        });

        // The live reader joining proves the drain made progress past the dead
        // peer (bounded by its read-exact, not a fixed sleep).
        live_reader.await.expect("live reader joins");
        let got = live_received
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        assert_eq!(got, vec![0xAB], "live node must still receive its frame");

        // The dead connection's down-hook must have fired (write failure path).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while down_count.load(Ordering::SeqCst) == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "dead peer down-hook never fired"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(connections.get_connection(dead_node).is_none());
        sender.shutdown();
        // Drop directly in the async context; FIX 1 keeps it panic-free.
        drop(sender);
    }

    /// FIX 3: a peer that is TCP-connected but never reads (its kernel send
    /// buffer fills, so `write_all` would otherwise park ~2h until OS keepalive)
    /// must NOT stall the shared drain indefinitely. The per-write [`WRITE_TIMEOUT`]
    /// turns the wedged write into a write failure: the wedged connection is
    /// marked down (down-hook fires, connection purged) and the drain proceeds to
    /// the healthy peer, which still receives its frame — bounded by the timeout,
    /// not the kernel keepalive.
    ///
    /// Multi-threaded so the wedged blocking write and the test's own polling can
    /// make progress concurrently on the test runtime; the `DistSender` has its
    /// own runtime regardless.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wedged_peer_does_not_stall_drain_indefinitely() {
        let (connections, atom_table) = manager();
        let down_count = Arc::new(AtomicUsize::new(0));
        let down_for_hook = Arc::clone(&down_count);
        connections.register_connection_down(move |_| {
            down_for_hook.fetch_add(1, Ordering::SeqCst);
        });

        // Wedged node: accept the connection but NEVER read from it. Holding the
        // accepted stream (without reading) keeps the peer connected, so writes do
        // not fail — they block once the kernel send+recv buffers fill.
        let wedged_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind wedged");
        let wedged_addr = wedged_listener.local_addr().expect("wedged addr");
        let wedged_node = atom_table.intern("wedged@127.0.0.1");
        let wedged_stream = std::net::TcpStream::connect(wedged_addr).expect("wedged connects");
        let wedged_peer_addr = wedged_stream.peer_addr().expect("wedged peer addr");
        let wedged_accept = tokio::spawn(async move { wedged_listener.accept().await });
        connections
            .register_test_connection(wedged_node, wedged_peer_addr, wedged_stream)
            .expect("register wedged connection");
        let wedged_accepted = wedged_accept
            .await
            .expect("wedged accept join")
            .expect("wedged accepted");
        // Keep the accepted half alive but never read it: this is what wedges the
        // writer. Dropping it would instead fail the write fast (the dead-peer
        // case, already covered separately).
        let _wedged_held = wedged_accepted;

        // Healthy node: a real reader that records what it receives.
        let live_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind live");
        let live_addr = live_listener.local_addr().expect("live addr");
        let live_received = Arc::new(Mutex::new(Vec::new()));
        let live_for_task = Arc::clone(&live_received);
        let live_reader = tokio::spawn(async move {
            let (mut stream, _) = live_listener.accept().await.expect("live accept");
            let mut header = [0u8; 8];
            if stream.read_exact(&mut header).await.is_ok() {
                let control_len =
                    u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
                let payload_len =
                    u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
                let mut body = vec![0u8; control_len + payload_len];
                if stream.read_exact(&mut body).await.is_ok() {
                    live_for_task
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push(body[0]);
                }
            }
        });
        let live_stream = std::net::TcpStream::connect(live_addr).expect("live connects");
        let live_node = atom_table.intern("live@127.0.0.1");
        let live_peer_addr = live_stream.peer_addr().expect("live peer addr");
        connections
            .register_test_connection(live_node, live_peer_addr, live_stream)
            .expect("register live connection");

        let sender = DistSender::new(connections.clone()).expect("sender builds");

        // One frame to the wedged node large enough to overflow the kernel send
        // and receive buffers (which the peer never drains), so `write_all` parks
        // and only `WRITE_TIMEOUT` can release it. 16 MiB exceeds default socket
        // buffers on Linux and macOS by orders of magnitude.
        let mut big = vec![0u8; 16 * 1024 * 1024];
        big[0] = 0x01;
        let big_control_len = u32::try_from(big.len()).expect("control fits u32");
        let mut wedged_frame = Vec::with_capacity(8 + big.len());
        wedged_frame.extend_from_slice(&big_control_len.to_be_bytes());
        wedged_frame.extend_from_slice(&0u32.to_be_bytes());
        wedged_frame.extend_from_slice(&big);
        sender.enqueue(DistOutbound::ToNode {
            node: wedged_node,
            frame: Arc::from(wedged_frame.into_boxed_slice()),
        });
        // Then a small frame to the healthy node, enqueued AFTER the wedged one.
        // The single drain reaches it only once the wedged write is released by
        // the timeout — proving the stall is bounded, not indefinite.
        sender.enqueue(DistOutbound::ToNode {
            node: live_node,
            frame: framed(&[0xAB]),
        });

        // The healthy reader must still join — but only after the wedged write
        // times out. Bound the wait generously above WRITE_TIMEOUT (5s) so the
        // test proves "bounded by the timeout" without flaking, yet would fail
        // hard on an indefinite (~2h keepalive) stall.
        let live_join = tokio::time::timeout(Duration::from_secs(30), live_reader)
            .await
            .expect("healthy peer received within the bounded window (not a ~2h stall)");
        live_join.expect("live reader task joins");
        let got = live_received
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        assert_eq!(got, vec![0xAB], "healthy node must still receive its frame");

        // The wedged connection must have been marked down via the write-timeout
        // path (down-hook fired, connection purged from the table).
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while down_count.load(Ordering::SeqCst) == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "wedged peer down-hook never fired after the write timeout"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            connections.get_connection(wedged_node).is_none(),
            "wedged connection must be purged after the write timeout"
        );

        sender.shutdown();
        drop(sender);
    }

    /// DC-5: control frames enqueued against one pinned connection arrive at
    /// that node in FIFO order — one bounded lane, one drain task, one
    /// per-connection writer `Mutex`. Mirror of `per_node_fifo_ordering` on the
    /// control lane.
    #[tokio::test]
    async fn control_lane_per_node_fifo_ordering() {
        let (connections, atom_table) = manager();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        // Peer side: read every frame and record its 1-byte control sequence id.
        let received = Arc::new(Mutex::new(Vec::new()));
        let received_for_task = Arc::clone(&received);
        let count = 16usize;
        let reader = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            for _ in 0..count {
                let mut header = [0u8; 8];
                if stream.read_exact(&mut header).await.is_err() {
                    break;
                }
                let control_len =
                    u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
                let payload_len =
                    u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
                let mut body = vec![0u8; control_len + payload_len];
                if stream.read_exact(&mut body).await.is_err() {
                    break;
                }
                received_for_task
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(body[0]);
            }
        });

        let std_stream = std::net::TcpStream::connect(addr).expect("client connects");
        let node = atom_table.intern("peer@127.0.0.1");
        let peer_addr: SocketAddr = std_stream.peer_addr().expect("peer addr");
        let connection = connections
            .register_test_connection(node, peer_addr, std_stream)
            .expect("register test connection");

        let sender = DistSender::new(connections).expect("sender builds");
        for index in 0..count {
            let seq = u8::try_from(index).expect("seq fits u8");
            sender
                .enqueue_control(ControlOutbound {
                    connection: Arc::clone(&connection),
                    frame: framed(&[seq]),
                })
                .expect("control lane accepts below capacity");
        }

        reader.await.expect("reader task joins");
        let order = received
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let expected: Vec<u8> = (0..count).map(|i| i as u8).collect();
        assert_eq!(order, expected, "controls must arrive in enqueue order");
        sender.shutdown();
        drop(sender);
    }

    /// T-1 (DC-1): flooding the control lane against a wedged peer (accepted
    /// but never read, so the drain parks on the first oversized write) must
    /// (a) never block the caller, (b) overflow and mark the PINNED connection
    /// down exactly once — `ControlOverflow` at enqueue, or `WriteTimeout` if
    /// the write deadline won the race; either is DC-1(b) — and (c) purge the
    /// connection from the table.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn control_lane_overflow_flood_marks_wedged_peer_down_exactly_once() {
        let (connections, atom_table) = manager();
        let down_reasons = Arc::new(Mutex::new(Vec::new()));
        let down_for_hook = Arc::clone(&down_reasons);
        connections.register_connection_down(move |event| {
            down_for_hook
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(event.reason);
        });

        // Wedged node: accept the connection but NEVER read from it, so writes
        // block once the kernel send+recv buffers fill (see
        // `wedged_peer_does_not_stall_drain_indefinitely`).
        let wedged_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind wedged");
        let wedged_addr = wedged_listener.local_addr().expect("wedged addr");
        let wedged_node = atom_table.intern("wedged@127.0.0.1");
        let wedged_stream = std::net::TcpStream::connect(wedged_addr).expect("wedged connects");
        let wedged_peer_addr = wedged_stream.peer_addr().expect("wedged peer addr");
        let wedged_accept = tokio::spawn(async move { wedged_listener.accept().await });
        let wedged_connection = connections
            .register_test_connection(wedged_node, wedged_peer_addr, wedged_stream)
            .expect("register wedged connection");
        let _wedged_held = wedged_accept
            .await
            .expect("wedged accept join")
            .expect("wedged accepted");

        let sender = DistSender::new(connections.clone()).expect("sender builds");

        // Park the drain: one control frame large enough to overflow the kernel
        // buffers the peer never drains, so `write_all` blocks until
        // WRITE_TIMEOUT and the lane fills behind it.
        let mut big = vec![0u8; 16 * 1024 * 1024];
        big[0] = 0x01;
        let big_control_len = u32::try_from(big.len()).expect("control fits u32");
        let mut big_frame = Vec::with_capacity(8 + big.len());
        big_frame.extend_from_slice(&big_control_len.to_be_bytes());
        big_frame.extend_from_slice(&0u32.to_be_bytes());
        big_frame.extend_from_slice(&big);
        sender
            .enqueue_control(ControlOutbound {
                connection: Arc::clone(&wedged_connection),
                frame: Arc::from(big_frame.into_boxed_slice()),
            })
            .expect("first control accepted into an empty lane");

        // Flood far past capacity. Every call must return promptly (try_send,
        // never a blocking wait on the wedged write) and the lane must report
        // Overflow once full — having ALREADY marked the pinned connection down.
        let start = std::time::Instant::now();
        let mut overflowed = 0usize;
        for index in 0..(DIST_CONTROL_QUEUE_CAP * 4) {
            match sender.enqueue_control(ControlOutbound {
                connection: Arc::clone(&wedged_connection),
                frame: framed(&index.to_be_bytes()),
            }) {
                Ok(()) => {}
                Err(ControlEnqueueError::Overflow) => {
                    // The Overflow contract: the pinned connection was ALREADY
                    // marked down before enqueue_control returned.
                    assert!(
                        wedged_connection.is_down(),
                        "Overflow must mark the pinned connection down before returning"
                    );
                    overflowed += 1;
                }
                Err(ControlEnqueueError::Closed) => {
                    panic!("control lane must not close while the sender is live")
                }
            }
        }
        let elapsed = start.elapsed();
        assert!(
            overflowed > 0,
            "flooding 4x capacity behind a wedged write must overflow the lane"
        );
        assert!(
            elapsed < WRITE_TIMEOUT,
            "enqueue_control must be non-blocking; flood took {elapsed:?}"
        );

        // The down-hook fires exactly once (mark_down is once-guarded), with a
        // DC-1(b) reason, and the connection leaves the table.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while down_reasons
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_empty()
        {
            assert!(
                std::time::Instant::now() < deadline,
                "overflow down-hook never fired"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let reasons = down_reasons
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        assert_eq!(
            reasons.len(),
            1,
            "down-hook must fire exactly once, got {reasons:?}"
        );
        assert!(
            matches!(
                reasons[0],
                ConnectionDownReason::ControlOverflow | ConnectionDownReason::WriteTimeout
            ),
            "down reason must be a DC-1(b) arm, got {:?}",
            reasons[0]
        );
        assert!(wedged_connection.is_down());
        assert!(
            connections.get_connection(wedged_node).is_none(),
            "wedged connection must be purged from the table"
        );

        sender.shutdown();
        drop(sender);
    }

    /// T-3 (DC-2 generation pinning): a control enqueued against a downed
    /// generation G is skipped by the drain (`is_down`), never re-resolved by
    /// node onto the post-redial socket. The biased drain handles the control
    /// lane first, so if pinning were broken the stale control — not the
    /// data-lane sentinel — would be the first frame on the new socket.
    #[tokio::test]
    async fn control_frame_pinned_to_down_generation_never_reaches_redialed_socket() {
        let (connections, atom_table) = manager();
        let node = atom_table.intern("peer@127.0.0.1");

        // Generation G: register, hold the peer half alive, capture the Arc.
        let listener_g = TcpListener::bind("127.0.0.1:0").await.expect("bind G");
        let addr_g = listener_g.local_addr().expect("G addr");
        let stream_g = std::net::TcpStream::connect(addr_g).expect("G connects");
        let peer_addr_g = stream_g.peer_addr().expect("G peer addr");
        let accept_g = tokio::spawn(async move { listener_g.accept().await });
        let pinned = connections
            .register_test_connection(node, peer_addr_g, stream_g)
            .expect("register generation G");
        let _held_g = accept_g.await.expect("G accept join").expect("G accepted");

        // Down G, then "redial": a fresh connection takes the node key.
        pinned.mark_down_write_timeout();
        assert!(pinned.is_down());
        assert!(
            connections.get_connection(node).is_none(),
            "downed generation must leave the table before the redial"
        );

        let listener_new = TcpListener::bind("127.0.0.1:0").await.expect("bind new");
        let addr_new = listener_new.local_addr().expect("new addr");
        let received = Arc::new(Mutex::new(Vec::new()));
        let received_for_task = Arc::clone(&received);
        let reader = tokio::spawn(async move {
            let (mut stream, _) = listener_new.accept().await.expect("new accept");
            let mut header = [0u8; 8];
            if stream.read_exact(&mut header).await.is_ok() {
                let control_len =
                    u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
                let payload_len =
                    u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
                let mut body = vec![0u8; control_len + payload_len];
                if stream.read_exact(&mut body).await.is_ok() {
                    received_for_task
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push(body[0]);
                }
            }
        });
        let stream_new = std::net::TcpStream::connect(addr_new).expect("new connects");
        let peer_addr_new = stream_new.peer_addr().expect("new peer addr");
        connections
            .register_test_connection(node, peer_addr_new, stream_new)
            .expect("register redialed connection");

        let sender = DistSender::new(connections.clone()).expect("sender builds");
        // Control pinned to the DOWN generation G: accepted into the lane, but
        // the drain must skip it rather than resolve `node` to the new socket.
        sender
            .enqueue_control(ControlOutbound {
                connection: Arc::clone(&pinned),
                frame: framed(&[0xCC]),
            })
            .expect("enqueue accepts; the is_down skip happens at the drain");
        // Data-lane sentinel resolved by node to the NEW connection, enqueued
        // AFTER the control (which the biased drain also prefers).
        sender.enqueue(DistOutbound::ToNode {
            node,
            frame: framed(&[0xDD]),
        });

        reader.await.expect("reader task joins");
        let got = received
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        assert_eq!(
            got,
            vec![0xDD],
            "first frame on the redialed socket must be the sentinel, never the pinned control"
        );
        sender.shutdown();
        drop(sender);
    }
}
