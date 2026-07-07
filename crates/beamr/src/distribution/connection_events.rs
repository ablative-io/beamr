//! Connection lifecycle events for distribution links: generation-tagged
//! Up/Down event types, the multi-subscriber [`ConnectionEventHub`], and the
//! legacy single-slot [`ConnectionDownHook`] (the `register_connection_down`
//! target, retained as 0.11-compat surface; `connection.rs` re-exports the
//! legacy types so existing import paths keep compiling).
//!
//! # Delivery and ordering contract
//!
//! - **INV-ALTERNATION** — per node, delivered events form
//!   `Up(g1) Down(g1) Up(g2) …` with strictly increasing generations; for one
//!   generation, Up(g) delivery completes before Down(g) delivery begins
//!   (queue order is table-transition order). "Peer bounced" = `Down(gn)` then
//!   `Up(gn+1)`; compare `peer_creation` across the Ups to distinguish peer
//!   restart (changed) from link blip (same). "Peer gone" = `Down(gn)` with no
//!   subsequent Up.
//! - **INV-EXACTLY-ONCE** — exactly one Up per generation; exactly one Down
//!   per generation delivered while the manager lives, EXCEPT a session still
//!   live at manager teardown, which gets no Down (parity with the legacy
//!   hook: manager drop never synthesized events). Socket displacement by the
//!   SAME peer incarnation (simultaneous connect) is invisible (same session;
//!   generation inherited); a live link displaced by a NEW peer incarnation —
//!   a restarted peer re-dialing past a stale link — is a session boundary
//!   and delivers Down(g) then Up(g+1), so `peer_creation` really does change
//!   iff the peer VM restarted. This holds regardless of the canonical
//!   direction tie-break: that tie-break resolves only SAME-incarnation
//!   simultaneous connects and never shields a stale incarnation, so a
//!   bounced newcomer installs (with Down(g) then Up(g+1)) even against a
//!   live canonical incumbent.
//! - **INV-TOTAL-ORDER** — all subscribers observe all events in one global
//!   sequence (single-drainer queue). Within one event: subscribers in
//!   registration order, then the legacy down-slot last (Down only, 0.11
//!   `ConnectionDownEvent{node, reason}` shape).
//! - **INV-SYNC** — delivery is synchronous with the transition: when the
//!   call that caused a transition returns (`disconnect_node`, the `mark_down`
//!   inside a read/heartbeat/drain task, `register_connection` via
//!   `connect`/accept), every event it produced has been delivered to every
//!   subscriber — by the calling thread, or by a concurrent dispatcher the
//!   call waited on. Sole exception: a transition triggered from INSIDE a
//!   callback is delivered after that callback returns, on the same thread,
//!   before the outermost dispatch returns. pg-purge therefore remains
//!   strictly synchronous-on-socket-drop. The delivering thread is
//!   unspecified (read-loop/heartbeat/drain tasks on the dist-send runtime,
//!   or the `disconnect_node` caller's thread).
//! - **INV-UP-VISIBILITY** — `Up(g)` is enqueued after the generation-g
//!   connection is installed in the table; `get_connection(node)` inside the
//!   Up callback returns a connection with `generation() >= g` (== g unless
//!   the link already bounced again, in which case `Down(g)` follows in event
//!   order).
//! - **INV-DOWN-VISIBILITY** — `get_connection(node)` inside a `Down(g)`
//!   callback never returns the generation-g connection (removal completes
//!   before the enqueueing guard releases; a concurrent dispatcher's lookup
//!   blocks on the shard lock until it does).
//! - **INV-FRAME-ORDER** — no inbound frame from the generation-g socket
//!   reaches the control-frame handler before `Up(g)` delivery completes: the
//!   read loop is spawned only after the installer's dispatch returns. (No
//!   ordering is promised between a Down and the last frames of the closing
//!   generation — a dying socket's final frames may be processed after its
//!   Down, as today.)
//! - **INV-SUB-DISCIPLINE** — callbacks MUST NOT block, MUST NOT perform
//!   socket I/O, and MUST capture only `Weak` handles to scheduler state. A
//!   blocked callback stalls reads, writes, heartbeats, accepts AND
//!   concurrent transition callers for EVERY peer. Brief bounded acquisition
//!   of short-hold locks (process slot mutexes, as the scheduler's own
//!   subscriber does) is acceptable. Callbacks MAY re-enter any non-async
//!   `ConnectionManager` method on the SAME manager, including
//!   subscribe/unsubscribe and `disconnect_node` (delivered after the current
//!   callback); they must NOT synchronously drive another manager's lifecycle
//!   (two managers cross-tearing simultaneously is a classic ABBA on the two
//!   dispatch gates — defer cross-manager actions to another task) and must
//!   not `block_on` async methods.
//! - **INV-SCHED-FIRST** — the scheduler registers its composed subscriber at
//!   construction, before any embedder can subscribe; within it, pg-purge
//!   strictly precedes noconnection delivery. Embedder subscribers, the
//!   legacy slot, and any trap-exit process receiving
//!   `{'EXIT', _, noconnection}` therefore always observe post-purge pg
//!   state.
//! - **INV-NO-REPLAY** — no replay of the real event history for late
//!   subscribers. The blessed late-subscriber path is
//!   `ConnectionManager::subscribe_connection_events_with_snapshot`, which
//!   delivers a subscriber-local synthetic `Up` per live peer under the
//!   dispatch gate before registering, so the subscription starts from a
//!   race-free snapshot. The manual recipe — subscribe via
//!   `subscribe_connection_events`, then `connected_peers()`,
//!   max-by-generation per peer — remains valid.
//!
//! One parity caveat: on simultaneous-connect displacement, frames in the
//! displaced socket's kernel buffer are dropped with no event, and no Up
//! fires (same session) — so an Up-triggered backfill does NOT re-run.
//! Identical to the pre-hub behaviour (no hook fired either).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::ThreadId;

use dashmap::DashMap;

use crate::atom::Atom;

/// Reason a distribution connection left the active table.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ConnectionDownReason {
    /// The peer closed its side of the connection cleanly.
    PeerClosed,
    /// A read operation reported an error.
    ReadError,
    /// A write operation reported an error.
    WriteError,
    /// A write exceeded its deadline (peer connected but not reading; kernel
    /// send buffer full). Treated as a terminal write failure by the outbound
    /// sender so a wedged peer cannot stall the shared drain.
    WriteTimeout,
    /// The local node explicitly closed the connection.
    ManualDisconnect,
    /// The proactive net-tick observed no inbound traffic (data frame or
    /// keepalive) within the configured liveness deadline: the peer is silently
    /// partitioned (no FIN/RST), so the link is marked down so the
    /// connection-event hub fires (pg-purge, noconnection delivery, embedder
    /// subscribers). Remote-monitor DOWN purge lands with the monitor stage.
    HeartbeatTimeout,
    /// The must-deliver control lane overflowed against this connection: the
    /// peer cannot absorb pending LINK/EXIT controls, so it is treated as down
    /// and the noconnection backstop supplies the coarsened signals (DC-1/DC-3,
    /// `DIST-CONTROL-WIRE-SPEC.md` §5).
    ControlOverflow,
}

/// Event emitted when a connection is removed from the active connection table.
///
/// This is the legacy 0.11 event shape delivered to the single-slot
/// [`ConnectionDownHook`]; new consumers should prefer
/// [`ConnectionEvent`] via `ConnectionManager::subscribe_connection_events`,
/// which also carries connection Up transitions and session generations.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ConnectionDownEvent {
    /// Node name key whose connection went down.
    pub node: Atom,
    /// Why the connection was removed.
    pub reason: ConnectionDownReason,
}

type ConnectionDownCallback = dyn Fn(ConnectionDownEvent) + Send + Sync + 'static;

/// Per-manager callback registration for connection-down notifications.
///
/// Legacy 0.11 replace-on-register single slot: registering a callback
/// replaces any previous registrant. It fires LAST (after every hub
/// subscriber), Down only. New consumers should prefer
/// `ConnectionManager::subscribe_connection_events`, which supports multiple
/// ordered subscribers and Up events.
#[derive(Clone, Default)]
pub struct ConnectionDownHook {
    callback: Arc<RwLock<Option<Arc<ConnectionDownCallback>>>>,
}

impl ConnectionDownHook {
    /// Create an empty connection-down callback slot.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or replace the connection-down callback.
    pub fn register<F>(&self, callback: F)
    where
        F: Fn(ConnectionDownEvent) + Send + Sync + 'static,
    {
        let mut slot = self
            .callback
            .write()
            .unwrap_or_else(|error| error.into_inner());
        *slot = Some(Arc::new(callback));
    }

    /// Remove the registered callback.
    pub fn unregister(&self) {
        let mut slot = self
            .callback
            .write()
            .unwrap_or_else(|error| error.into_inner());
        *slot = None;
    }

    /// Return true when a callback is registered.
    #[must_use]
    pub fn is_registered(&self) -> bool {
        self.callback
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
    }

    pub(crate) fn invoke(&self, event: ConnectionDownEvent) {
        let callback = self
            .callback
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(callback) = callback {
            callback(event);
        }
    }
}

/// Monotonic per-peer connectivity-session counter, LOCAL to this node.
/// Strictly increasing per peer name for the lifetime of the
/// `ConnectionManager`. NOT the peer's OTP creation; NOT on the wire; NOT
/// comparable across nodes; resets when the local manager is rebuilt.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ConnectionGeneration(u64);

impl ConnectionGeneration {
    /// The raw counter value (generations start at 1).
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// For consumer test harnesses; production values come from events.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// A peer transitioned disconnected -> connected (session opened).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NodeUp {
    /// Peer's authenticated handshake-name atom (the connection-table key).
    pub node: Atom,
    /// Session generation opened by this transition.
    pub generation: ConnectionGeneration,
    /// Peer incarnation from the authenticated handshake
    /// (`HandshakeResult::remote_creation`). 0 is the "no handshake" sentinel
    /// used by the in-crate test helper — production installs always come
    /// through the handshake. Changes iff the peer VM restarted: THIS — not
    /// `generation` — answers "did the peer bounce or did the link blip?".
    pub peer_creation: u32,
}

/// A peer transitioned connected -> disconnected (session closed).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NodeDown {
    /// Peer's authenticated handshake-name atom (the connection-table key).
    pub node: Atom,
    /// Always equals the most recent NodeUp generation delivered for `node`.
    pub generation: ConnectionGeneration,
    /// Why the session closed.
    pub reason: ConnectionDownReason,
}

/// Connection lifecycle event. `#[non_exhaustive]`: cross-crate subscribers
/// must use a wildcard arm; in-crate matches stay exhaustive WITHOUT a `_`
/// arm (non_exhaustive is inert in-crate; a trailing `_` trips
/// unreachable_patterns under -D warnings).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConnectionEvent {
    /// A peer session opened.
    Up(NodeUp),
    /// A peer session closed.
    Down(NodeDown),
}

impl ConnectionEvent {
    // Constructors + accessors so cross-crate consumers (liminal/haematite/
    // frame tests) can fabricate and inspect events despite #[non_exhaustive].

    /// Build an Up event.
    #[must_use]
    pub fn up(node: Atom, generation: ConnectionGeneration, peer_creation: u32) -> Self {
        Self::Up(NodeUp {
            node,
            generation,
            peer_creation,
        })
    }

    /// Build a Down event.
    #[must_use]
    pub fn down(
        node: Atom,
        generation: ConnectionGeneration,
        reason: ConnectionDownReason,
    ) -> Self {
        Self::Down(NodeDown {
            node,
            generation,
            reason,
        })
    }

    /// The peer node this event is about.
    #[must_use]
    pub fn node(&self) -> Atom {
        match self {
            Self::Up(up) => up.node,
            Self::Down(down) => down.node,
        }
    }

    /// The session generation this event opens or closes.
    #[must_use]
    pub fn generation(&self) -> ConnectionGeneration {
        match self {
            Self::Up(up) => up.generation,
            Self::Down(down) => down.generation,
        }
    }

    /// The down reason; `None` for Up.
    #[must_use]
    pub fn down_reason(&self) -> Option<ConnectionDownReason> {
        match self {
            Self::Up(_) => None,
            Self::Down(down) => Some(down.reason),
        }
    }
}

/// Opaque handle identifying one subscription; pass to unsubscribe.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct SubscriberId(u64);

type ConnectionEventCallback = dyn Fn(ConnectionEvent) + Send + Sync + 'static;

/// Multi-subscriber registration point + delivery queue for connection
/// lifecycle events, plus the legacy single-slot down callback (invoked LAST
/// on Down). Owned by `ConnectionManagerInner`.
#[derive(Default)]
pub(crate) struct ConnectionEventHub {
    /// (id, callback) pairs; registration order == invocation order.
    subscribers: RwLock<Vec<(SubscriberId, Arc<ConnectionEventCallback>)>>,
    next_subscriber_id: AtomicU64,
    /// The legacy replace-on-register slot (register_connection_down target).
    legacy_down: ConnectionDownHook,
    /// Last-assigned generation per peer. Mutated ONLY under the caller's
    /// connections-table entry guard for that peer (one-way lock order:
    /// entry guard, then this shard, a leaf). Entries never removed: bounded
    /// by cluster size.
    generations: DashMap<Atom, u64>,
    /// Globally-ordered pending events. Push sites hold the connections entry
    /// guard for the event's node; pop/dispatch holds no other lock.
    queue: Mutex<VecDeque<ConnectionEvent>>,
    /// Blocking dispatch gate: exactly one drainer at a time; waiters block
    /// until the holder has drained (including THEIR events), so dispatch()
    /// returning means delivery happened (INV-SYNC).
    dispatch_gate: Mutex<()>,
    /// ThreadId of the current gate holder (None when free). Read before
    /// blocking so a reentrant dispatch() on the holder's own thread returns
    /// immediately instead of self-deadlocking. Leaf lock.
    dispatch_owner: Mutex<Option<ThreadId>>,
}

impl ConnectionEventHub {
    /// Create an empty hub: no subscribers, no legacy registrant, no assigned
    /// generations, empty queue.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Append `callback` to the subscriber registry. Invocation order is
    /// registration order.
    pub(crate) fn subscribe<F>(&self, callback: F) -> SubscriberId
    where
        F: Fn(ConnectionEvent) + Send + Sync + 'static,
    {
        let id = SubscriberId(self.next_subscriber_id.fetch_add(1, Ordering::Relaxed));
        let mut subscribers = self
            .subscribers
            .write()
            .unwrap_or_else(|error| error.into_inner());
        subscribers.push((id, Arc::new(callback)));
        id
    }

    /// Returns false if the id is unknown / already removed. NOT a barrier: an
    /// in-flight dispatch that already snapshotted may invoke the callback
    /// once more after unsubscribe returns.
    pub(crate) fn unsubscribe(&self, id: SubscriberId) -> bool {
        let mut subscribers = self
            .subscribers
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let before = subscribers.len();
        subscribers.retain(|(subscriber_id, _)| *subscriber_id != id);
        subscribers.len() != before
    }

    /// Clone sharing the legacy slot (`connection_down_hook()` /
    /// `register_connection_down`).
    pub(crate) fn legacy_down_hook(&self) -> ConnectionDownHook {
        self.legacy_down.clone()
    }

    /// Next session generation for `node` (starts at 1). MUST be called while
    /// holding the connections entry guard for `node`, so assignment order ==
    /// install order and generations are strictly increasing per peer.
    pub(crate) fn next_generation(&self, node: Atom) -> ConnectionGeneration {
        let mut entry = self.generations.entry(node).or_insert(0);
        *entry += 1;
        ConnectionGeneration(*entry)
    }

    /// Read-only: last generation ever assigned for `node`.
    pub(crate) fn last_generation(&self, node: Atom) -> Option<ConnectionGeneration> {
        self.generations
            .get(&node)
            .map(|entry| ConnectionGeneration(*entry.value()))
    }

    /// Append to the delivery queue. MUST be called while holding the
    /// connections entry guard for the event's node (this is what totally
    /// orders a node's events).
    pub(crate) fn enqueue(&self, event: ConnectionEvent) {
        let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        queue.push_back(event);
    }

    /// Drain and deliver pending events. MUST be called with NO manager locks
    /// held. Blocking: on return, every event enqueued before the call has
    /// been delivered to all subscribers (by this thread or by the concurrent
    /// drainer it waited on) — EXCEPT when called reentrantly from inside a
    /// subscriber on the draining thread, where it returns immediately and the
    /// outer drain delivers after the current callback returns.
    pub(crate) fn dispatch(&self) {
        let me = std::thread::current().id();
        // Reentrancy check: if THIS thread already holds the gate we are
        // inside a subscriber callback; the outer drain loop delivers whatever
        // we enqueued, after the current callback returns. Blocking here would
        // self-deadlock.
        {
            let owner = self
                .dispatch_owner
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if *owner == Some(me) {
                return;
            }
        } // owner lock released before blocking on the gate
        let _gate = self
            .dispatch_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // RAII owner record: reset even if a (test-only) subscriber panics, so
        // a poisoned pass cannot wedge reentrancy detection for this thread
        // forever.
        let _owner = OwnerGuard::set(&self.dispatch_owner, me);
        self.drain_queue();
        // _owner resets dispatch_owner to None, then _gate releases.
    }

    /// Register `callback` with subscriber-local synthetic catch-up. MUST be
    /// called with NO manager locks held (it blocks on the dispatch gate).
    ///
    /// Under the gate: pending queued events are first delivered to the
    /// EXISTING subscribers (and the queue re-checked after each `live_peers`
    /// snapshot, so a queued real `Up(g)` whose install the snapshot already
    /// reflects is never also synthesized); then `callback` alone is invoked
    /// with a synthetic `Up` per snapshot row; then it is registered. Because
    /// the gate is held throughout, no real event interleaves between the
    /// snapshot and the registration — the subscriber's per-node stream
    /// satisfies INV-ALTERNATION from its first synthetic Up. The synthetic
    /// Ups never enter the queue: other subscribers do not observe them.
    ///
    /// Called reentrantly (from inside a subscriber callback on the draining
    /// thread), this registers and returns WITHOUT synthetic events: blocking
    /// on the gate would self-deadlock, and no race-free snapshot exists
    /// mid-drain.
    pub(crate) fn subscribe_with_snapshot<F>(
        &self,
        callback: F,
        live_peers: impl Fn() -> Vec<NodeUp>,
    ) -> SubscriberId
    where
        F: Fn(ConnectionEvent) + Send + Sync + 'static,
    {
        let me = std::thread::current().id();
        {
            let owner = self
                .dispatch_owner
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if *owner == Some(me) {
                // Reentrant: register plain, no catch-up (documented above).
                return self.subscribe(callback);
            }
        } // owner lock released before blocking on the gate
        let _gate = self
            .dispatch_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _owner = OwnerGuard::set(&self.dispatch_owner, me);
        // Drain until a snapshot is taken with the queue observed empty
        // afterwards. Enqueues happen under the connections entry guard for
        // the event's node, after the table mutation, and the snapshot's
        // shard reads block on that guard — so any table state the snapshot
        // reflects has its event queued BEFORE this emptiness check. A
        // non-empty queue therefore means the snapshot may already reflect a
        // queued-but-undelivered event (which would be both synthesized and
        // redelivered): deliver and snapshot again.
        let rows = loop {
            self.drain_queue();
            let rows = live_peers();
            let queue_empty = self
                .queue
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty();
            if queue_empty {
                break rows;
            }
        };
        let callback: Arc<ConnectionEventCallback> = Arc::new(callback);
        for row in rows {
            callback(ConnectionEvent::Up(row));
        }
        let id = SubscriberId(self.next_subscriber_id.fetch_add(1, Ordering::Relaxed));
        self.subscribers
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .push((id, Arc::clone(&callback)));
        // Deliver anything enqueued meanwhile (including by a catch-up
        // callback that triggered a transition) before releasing the gate,
        // preserving INV-SYNC; such events postdate the snapshot, so the new
        // subscriber receiving them here is the correct order, not a replay.
        self.drain_queue();
        id
    }

    /// Deliver every queued event to the current subscribers (then the legacy
    /// slot, Down only). MUST be called only while holding `dispatch_gate`
    /// with `dispatch_owner` recorded for this thread.
    fn drain_queue(&self) {
        loop {
            let event = {
                let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
                queue.pop_front()
            }; // queue lock released before any callback runs
            let Some(event) = event else { break };
            // Snapshot under the read lock, release, then invoke: no hub lock
            // is held during a callback, so callbacks may re-enter
            // subscribe/unsubscribe/the manager freely.
            let snapshot: Vec<Arc<ConnectionEventCallback>> = self
                .subscribers
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .iter()
                .map(|(_, callback)| Arc::clone(callback))
                .collect();
            for callback in snapshot {
                callback(event);
            }
            if let ConnectionEvent::Down(down) = event {
                // Legacy slot LAST, 0.11 shape.
                self.legacy_down.invoke(ConnectionDownEvent {
                    node: down.node,
                    reason: down.reason,
                });
            }
        }
    }
}

/// RAII record of the dispatch-gate holder's thread: set on gate acquisition,
/// reset to `None` on drop — including a panic unwind out of a (test-only)
/// subscriber — so reentrancy detection can never stay wedged for a thread.
struct OwnerGuard<'a> {
    owner: &'a Mutex<Option<ThreadId>>,
}

impl<'a> OwnerGuard<'a> {
    fn set(owner: &'a Mutex<Option<ThreadId>>, holder: ThreadId) -> Self {
        *owner.lock().unwrap_or_else(|error| error.into_inner()) = Some(holder);
        Self { owner }
    }
}

impl Drop for OwnerGuard<'_> {
    fn drop(&mut self) {
        *self.owner.lock().unwrap_or_else(|error| error.into_inner()) = None;
    }
}
