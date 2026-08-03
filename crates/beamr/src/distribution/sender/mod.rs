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
//! ## Backpressure — the data lane is bounded in BYTES
//!
//! **What protects memory on this lane is [`DIST_SEND_QUEUE_BYTE_BUDGET`], not
//! a slot count.** The lane carries `Arc<[u8]>` frames of unbounded individual
//! size, so a slot count says nothing about how many bytes are resident: 64 of
//! the 1024 slots can hold a quarter of a gigabyte. Every frame is charged
//! `frame.len()` against the budget at [`DistSender::enqueue`] and the charge
//! is released when the drain finishes with it, so the lane's retention has a
//! byte ceiling regardless of frame size.
//!
//! [`DIST_SEND_QUEUE_CAP`] survives as the `mpsc` channel's slot count — a
//! secondary bound on the NUMBER of pending items (and on the `Arc` handles
//! they retain). It is not the memory protection and must not be read as one.
//!
//! Refusal fires on bytes OR slots; the ACTION is unchanged. [`DistSender::enqueue`]
//! uses a non-blocking `try_send` behind a non-blocking charge: on an exhausted
//! byte budget, a full channel, or a closed channel the frame is DROPPED. A
//! dropped membership update is self-correcting — the next `pg` join/leave or a
//! node-down purge re-establishes the correct view — so dropping is safe and is
//! strictly preferable to blocking a scheduler worker behind a stalled peer.
//!
//! ### Fan-out is charged per ENQUEUE, not per buffer (deliberate over-count)
//!
//! A broadcast clones the `Arc` handle, not the bytes: `pg_propagation` encodes
//! one frame and enqueues it once per connected node, so N slots can share ONE
//! buffer. The accounting charges `frame.len()` on each enqueue and therefore
//! OVER-COUNTS a fan-out by a factor of N.
//!
//! This is chosen over unique-buffer tracking, and the consequence is stated
//! rather than hidden. A pointer-keyed side table would have to be consulted on
//! the enqueue fast path — a path whose whole contract is that it never blocks a
//! scheduler worker — and keying on `Arc::as_ptr` is a correctness trap, since a
//! freed buffer's address can be reused by the next allocation. The cost of the
//! over-count is that a wide fan-out reaches the budget sooner than its true
//! retention warrants (one 1 MiB frame broadcast to 64 peers charges 64 MiB
//! while retaining 1 MiB), so the effective admitted volume under fan-out is
//! lower than the nominal budget. The error is ENTIRELY in the fail-closed
//! direction: the charge is never less than the bytes actually retained, so the
//! budget remains a true upper bound on this lane's retention.
//!
//! Note the scope of that over-count. It applies to this lane's OWN budget. It
//! does not contradict the caveat at [`MAX_DIST_FRAME_BYTES`]'s derivation
//! ("the send lane fans out over `Arc<[u8]>`, so a broadcast retains ONE
//! buffer: do not double-count the send lane against this budget") — that
//! caveat governs charging the send lane against the RECEIVE-side 13 GiB
//! envelope, which this lane still does not do.
//!
//! ### Coverage — what these bounds do NOT cover
//!
//! `send_remote` bypasses both queues entirely via a direct blocking write (see
//! the same derivation comment). Bytes on that path are never enqueued, so they
//! are neither charged nor bounded here. The budgets below bound what the two
//! QUEUES retain, not the total outbound distribution traffic.
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
//! drain up to `WRITE_TIMEOUT` — or until the wedged connection itself is
//! marked down (its own lane overflow, the net-tick), whichever comes first:
//! `mark_down`'s socket shutdown errors the parked write immediately, so the
//! drain recovers without waiting out the timer (commit 4). Within that
//! window a control to a HEALTHY peer may still overflow, marking the healthy
//! peer down — a spurious noconnection + redial (availability blip), never a
//! lost signal.
//!
//! ### Why the control lane carries NO byte budget (stated absence)
//!
//! The data lane got a byte budget because its frames have no structural size
//! ceiling. The control lane's do, so its slot count already IS a byte bound
//! and a second, redundant budget would buy nothing while adding a refusal arm
//! to a must-deliver lane — where every refusal costs a healthy connection.
//!
//! The ceiling, measured at the bytes (see
//! `control_frame_encoded_sizes_measured_at_the_bytes`): a control frame is
//! `{Op, FromExtPid, ToExtPid[, ReasonAtom]}` over an ALWAYS-NIL payload, so it
//! carries no user term at all. Its only variable-length components are the two
//! node-name atoms — each ceilinged at 65535 bytes by `ATOM_UTF8_EXT`'s `u16`
//! length field, and independently refused above `u16::MAX` by the handshake —
//! and a reason atom drawn from `ExitReason`'s closed six-atom set. With
//! realistic node names LINK/UNLINK encode to 74 bytes and EXIT/EXIT2 to 88;
//! against an adversarial peer advertising a name at the atom ceiling the worst
//! case is 131131 bytes.
//!
//! [`DIST_CONTROL_QUEUE_CAP`] slots x that ceiling is ~32 MiB — LESS THAN ONE
//! maximum-size inbound data frame ([`MAX_DIST_FRAME_BYTES`]). The count is
//! therefore load-bearing here in a way it is not on the data lane: it bounds
//! bytes because the per-frame size is bounded. This is pinned by
//! `control_lane_slot_cap_bounds_retained_bytes_below_one_max_data_frame`, so
//! the claim cannot rot silently if a wider frame ever reaches this lane.
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
//! The owned tokio [`Runtime`] performs a BLOCKING shutdown when dropped —
//! joining the "beamr-dist-send" worker — and both panics inside an async
//! context and self-deadlocks if awaited from its OWN worker. Because the last
//! [`DistSender`] `Arc` can resolve anywhere — a scheduler worker, the main
//! thread, a `#[tokio::test]` task, or a task on this very runtime —
//! `DistSenderInner` holds the runtime in a `Mutex<Option<_>>` and hands
//! teardown to `join_runtime_drop`,
//! which joins the worker before returning from every context except this
//! runtime's own thread (the only place a join must deadlock — there it falls
//! back to `shutdown_background`).
//!
//! ## Wedged-peer write deadline
//!
//! The single drain serialises writes across all peers, so one peer that is
//! TCP-connected but never reads (kernel send buffer full) would stall
//! propagation cluster-wide. Each write is bounded by `WRITE_TIMEOUT`; on
//! elapse the connection is marked down (firing the connection-down hook and
//! remote-node purge) and the drain proceeds.
//!
//! ## Module shape — what deliberately stays in `mod.rs`
//!
//! The runtime-lifecycle core (DistSenderInner, its Drop ordering, new's inlined
//! biased two-lane drain, handle/worker_thread_names/shutdown) and the wire types
//! DELIBERATELY stay whole in mod.rs: the two-level ownership rationale (a queued
//! charge must not keep the runtime alive from inside its own queue) and Inner's
//! Drop shutdown ordering are load-bearing side-by-side reading with shutdown() —
//! this is a measured decision (sender ground, 5 inner reaches all in one
//! cluster), not an oversight; do not split it without re-measuring the coupling.

mod residency;
#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::atom::Atom;
use crate::distribution::connection::{ConnectionManager, DistConnection};
use crate::distribution::etf::MAX_DIST_FRAME_BYTES;
use crate::distribution::join_runtime_drop;

use residency::{ChargedOutbound, LaneResidency};

/// OS thread name of the sender's single tokio worker.
///
/// Set as the runtime's `thread_name`, so it is also the name the OS thread
/// probe and the service inventory (spec §5) attribute the worker under.
pub const DIST_SEND_THREAD_NAME: &str = "beamr-dist-send";

/// Bounded SLOT COUNT of the outbound distribution queue.
///
/// **This number does not protect memory.** It bounds how many items may be
/// pending, and with them how many `Arc` handles the lane retains; it says
/// nothing about the bytes behind those handles, because a data-lane frame has
/// no structural size ceiling. The memory protection on this lane is
/// [`DIST_SEND_QUEUE_BYTE_BUDGET`], and a frame is refused when EITHER bound
/// would be exceeded. Retained as the `mpsc` channel's capacity because the
/// constructor takes a count and a slot bound is still worth having: it caps
/// per-item overhead and keeps the queue's own allocation bounded even when
/// every frame is tiny.
///
/// Sized for low-frequency control traffic (pg join/leave). When either bound
/// refuses, the producer drops rather than blocks; see the module docs.
pub const DIST_SEND_QUEUE_CAP: usize = 1024;

/// Byte budget for data-lane residency: `2 * MAX_DIST_FRAME_BYTES` (128 MiB).
///
/// **This is the load-bearing memory protection on the data lane.** Every frame
/// is charged `frame.len()` at enqueue and released when the drain is done with
/// it; a frame that would push residency past this budget is dropped.
///
/// # Derivation
///
/// Imported from [`MAX_DIST_FRAME_BYTES`] rather than minted, so the two cannot
/// drift and the 64 MiB figure keeps its single home
/// (`distribution/connection/`). Two max-size frames resident is already
/// pathological for a lane that exists to carry small `pg` control traffic (see
/// the module docs), and the multiplier keeps outbound retention well inside
/// the envelope the receive side consumes.
pub const DIST_SEND_QUEUE_BYTE_BUDGET: usize = 2 * MAX_DIST_FRAME_BYTES;

/// Bounded SLOT COUNT of the must-deliver control lane.
///
/// Unlike [`DIST_SEND_QUEUE_CAP`], this count IS the byte bound — because a
/// control frame's size is structurally ceilinged. Every frame on this lane is
/// `{Op, FromExtPid, ToExtPid[, ReasonAtom]}` over an always-NIL payload, whose
/// only variable-length parts are two `u16`-length-ceilinged node atoms and a
/// reason atom from a closed six-atom set. Measured worst case is 131131 bytes,
/// so these slots retain at most ~32 MiB — less than ONE
/// [`MAX_DIST_FRAME_BYTES`] data frame. That is why this lane carries no
/// separate byte budget; the module docs state the absence and its evidence.
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
pub(crate) const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// Held in a `Mutex<Option<_>>` so [`DistSender::shutdown`] can `take()` it
    /// through a shared `&self` and hand it to
    /// [`join_runtime_drop`](crate::distribution::join_runtime_drop), which
    /// joins the "beamr-dist-send" worker before returning from every context
    /// except this runtime's own thread (where any join self-deadlocks and
    /// teardown falls back to `shutdown_background`). The take happens in its
    /// own statement so this mutex is NEVER held across the join —
    /// [`DistSender::worker_thread_names`] locks it from worker-side contexts.
    /// `Some` for a live sender; `None` once shut down (or transiently in
    /// `drop`).
    runtime: Mutex<Option<Runtime>>,
    /// Cached handle to `runtime`, kept independently of the `Option` so
    /// [`DistSender::handle`] never has to inspect (or risk a `None` from) the
    /// shutdown-consumed `runtime` field. Cloning a `Handle` does not keep the
    /// runtime alive, so this does not interfere with the teardown.
    handle: Handle,
    /// Drain task handle, used to abort the loop on shutdown before the runtime
    /// is dropped.
    drain: JoinHandle<()>,
    /// This instance's runtime mark (see `mint_runtime_mark`), stamped on its
    /// worker and blocking-pool threads — `join_runtime_drop`'s per-INSTANCE
    /// self-runtime detector.
    mark: u64,
}

impl Drop for DistSenderInner {
    fn drop(&mut self) {
        // Abort the drain loop so the runtime has no in-flight task to wind down.
        // `shutdown()` is the primary path and is idempotent (a second abort is a
        // no-op); calling it here also covers a sender dropped without a prior
        // explicit `shutdown()`.
        self.drain.abort();
        // Safety net for a sender never explicitly shut down: joined whenever a
        // join cannot deadlock (see `join_runtime_drop`). After an explicit
        // `shutdown()` the runtime is already `None`, so this is a no-op.
        // `get_mut` touches no lock, so the guard-across-join deadlock shape
        // can't occur here.
        let runtime = self
            .runtime
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        join_runtime_drop(runtime, self.mark);
    }
}

/// Async outbound distribution sender.
///
/// Cheap to clone (an `mpsc::Sender` plus an `Arc`). All clones share the one
/// runtime, queue, and drain task.
#[derive(Clone)]
pub struct DistSender {
    tx: mpsc::Sender<ChargedOutbound>,
    control_tx: mpsc::Sender<ControlOutbound>,
    /// Data-lane byte meter. Held beside `inner` rather than inside it: a
    /// queued charge holds an `Arc` to this, and queued items are owned by the
    /// receiver that `inner`'s runtime drives. Were the meter reachable through
    /// `Arc<DistSenderInner>`, a pending frame would keep the sender — and so
    /// its runtime — alive from inside its own queue.
    data_residency: Arc<LaneResidency>,
    inner: Arc<DistSenderInner>,
}

impl DistSender {
    /// Build a sender owning a dedicated single-worker tokio runtime and spawn its
    /// drain task. Returns `None` only if the runtime could not be created.
    #[must_use]
    pub fn new(connections: ConnectionManager) -> Option<Self> {
        let mark = crate::distribution::mint_runtime_mark();
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder
            .worker_threads(1)
            .thread_name(DIST_SEND_THREAD_NAME)
            .enable_all();
        crate::distribution::stamp_runtime_threads(&mut builder, mark);
        let runtime = builder.build().ok()?;
        let (tx, mut rx) = mpsc::channel::<ChargedOutbound>(DIST_SEND_QUEUE_CAP);
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
                        Some(charged) => {
                            // Take the byte reservation out of the item and
                            // hold it across the write: the bytes ARE resident
                            // until the write is done with them. `charge` is
                            // dropped at the end of this arm, so every exit —
                            // write completed, write error, write timeout, or
                            // no connection to write to at all — releases it
                            // exactly once, with no path able to skip the
                            // release.
                            let ChargedOutbound {
                                item: DistOutbound::ToNode { node, frame },
                                charge,
                            } = charged;
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
                            drop(charge);
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
            data_residency: Arc::new(LaneResidency::new(DIST_SEND_QUEUE_BYTE_BUDGET)),
            inner: Arc::new(DistSenderInner {
                runtime: Mutex::new(Some(runtime)),
                mark,
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
    /// A live sender owns exactly one worker, named [`DIST_SEND_THREAD_NAME`];
    /// after [`shutdown`](Self::shutdown) has joined the runtime the slot is
    /// empty, so this reports zero — the post-shutdown inventory is truthful
    /// rather than claiming a worker that no longer exists. The lazily-spawned
    /// blocking pool is not live at rest, so it is not reported.
    #[must_use]
    pub fn worker_thread_names(&self) -> Vec<String> {
        if self
            .inner
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
        {
            vec![DIST_SEND_THREAD_NAME.to_owned()]
        } else {
            Vec::new()
        }
    }

    /// Enqueue an outbound frame. NON-BLOCKING: on an exhausted byte budget, a
    /// full queue, or a closed queue the frame is dropped (see module docs on
    /// backpressure). Never blocks the calling thread, so it is safe to call
    /// from a scheduler worker.
    ///
    /// The byte charge is taken FIRST, because the byte budget — not the slot
    /// count — is what protects memory here. A frame that does not fit the
    /// budget never reaches the channel; a frame that fits the budget but finds
    /// no free slot hands its charge back when `try_send` returns the item.
    pub fn enqueue(&self, item: DistOutbound) {
        let DistOutbound::ToNode { frame, .. } = &item;
        // Charge the frame's own bytes. A fan-out charges once per enqueue and
        // so over-counts a shared buffer; the module docs state that choice and
        // its consequence. The over-count is fail-closed — never an under-count.
        let Some(charge) = self.data_residency.try_charge(frame.len()) else {
            return;
        };
        // `try_send` returns `Err` on `Full` or `Closed`, handing the item back;
        // dropping it releases the charge with it.
        let _ = self.tx.try_send(ChargedOutbound { item, charge });
    }

    /// Bytes currently charged to the data lane's residency budget
    /// ([`DIST_SEND_QUEUE_BYTE_BUDGET`]).
    ///
    /// A frame is charged from the moment [`enqueue`](Self::enqueue) accepts it
    /// until the drain is done with it. Exposed so the byte bound is
    /// observable — a leaked reservation is a slow-starve, and a bound nobody
    /// can read is a bound nobody can test.
    #[must_use]
    pub fn data_lane_resident_bytes(&self) -> usize {
        self.data_residency.resident_bytes()
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

    /// Stop the sender: abort the drain task, then synchronously JOIN the owned
    /// "beamr-dist-send" runtime worker before returning (spec §4). Aborting the
    /// drain first leaves the runtime with no in-flight task; taking and dropping
    /// the runtime on a dedicated joined thread aborts the connection read/accept
    /// and heartbeat tasks it drives and winds the worker down off any async
    /// context. Idempotent: a second call finds the runtime already taken and is
    /// a no-op (a repeated `drain.abort()` is harmless).
    pub fn shutdown(&self) {
        self.inner.drain.abort();
        // Take the runtime in its OWN statement so the mutex guard drops before
        // the blocking join: `worker_thread_names()` (inventory) locks this
        // same mutex, and a worker-side task blocked on it while shutdown waits
        // for that worker is a lock-inversion deadlock.
        let runtime = self
            .inner
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        join_runtime_drop(runtime, self.inner.mark);
    }
}

// FUTURE: per-node sub-channels if a single drain becomes a head-of-line
// bottleneck for a hot peer. Not needed for low-frequency pg control traffic.
