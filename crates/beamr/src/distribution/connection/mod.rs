//! TCP connection table and lifecycle management for distribution links.
//!
//! The install/dedup/reap core (LinkDirection, ConnectingGuard,
//! canonical_direction, decide_inbound_status, connection_down,
//! register_connection, build_connection) DELIBERATELY stays whole in mod.rs:
//! its dashmap entry-guard ordering prose, the Arc::ptr_eq reap guard, and the
//! mark_down<->connection_down pair are load-bearing side-by-side reading —
//! this is a measured decision (#67 ground, dirtiest-seam), not an oversight;
//! do not split it without re-measuring the coupling.

mod dial;
mod frame;
mod lifecycle;
mod link;
mod residency;
#[cfg(test)]
mod tests;

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use dashmap::DashMap;
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::atom::{Atom, AtomTable};
use crate::distribution::handshake::{HandshakeNode, SimultaneousDecision};
use crate::distribution::resolver::NodeResolver;

pub(crate) use frame::FrameError;
#[cfg(all(test, unix))]
pub(crate) use link::FAIL_TEARDOWN_DUP_FOR_TEST;
use link::PreparedSocket;
pub use link::{AcceptHandle, ConnectError, DistConnection, HeartbeatConfig};
use residency::{InboundAdmissionPermit, InboundResidency};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Default whole-handshake deadline. Mirrors [`DEFAULT_CONNECT_TIMEOUT`]: any
/// finite value removes the deadlock; 5s tolerates a loaded peer without wedging
/// a cluster (DISTRIBUTION-HANDSHAKE-DESIGN.md D3).
const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Default proactive net-tick interval: how often an idle link emits a keepalive
/// and checks the inbound-liveness deadline.
const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

/// Default inbound-liveness deadline: a link with no inbound bytes (data frame
/// OR keepalive) for this long is marked down. Comfortably larger than the
/// interval so a healthy peer's keepalives always refresh liveness in time and
/// no healthy idle link is ever spuriously downed.
const DEFAULT_HEARTBEAT_DEADLINE: Duration = Duration::from_secs(45);

pub use super::connection_events::{ConnectionDownEvent, ConnectionDownHook, ConnectionDownReason};

use super::connection_events::{
    ConnectionEvent, ConnectionEventHub, ConnectionGeneration, NodeUp, SubscriberId,
};

type ControlFrameHandler = dyn Fn(Atom, &[u8], &[u8]) + Send + Sync + 'static;

struct ConnectionManagerInner {
    connections: DashMap<Atom, Arc<DistConnection>>,
    /// Peer-name atoms with an in-flight OUTBOUND dial (the `Connecting` state,
    /// DISTRIBUTION-HANDSHAKE-DESIGN.md §3.1). Recorded before the outbound
    /// handshake awaits so a concurrent inbound responder can detect the
    /// simultaneous case and apply the name-comparison tie-break (HS-3, §3.2).
    ///
    /// The value is an abort flag for that outbound. When this node is the
    /// lower-named peer it keeps the reciprocal INBOUND (the responder decides
    /// `ContinueSimultaneous`) and must retire its own competing outbound rather
    /// than letting both install and collide in the HS-2 dedup — a collision
    /// whose loser-socket drop can tear down the peer's surviving link, leaving
    /// the pair with zero links and no re-dial. The decider sets this flag; the
    /// outbound `connect` checks it after the handshake and bows out cleanly
    /// (`SimultaneousAbort`) if set (§3.2, point 2: "mark the local outbound to
    /// abort").
    connecting: DashMap<Atom, Arc<AtomicBool>>,
    atom_table: Arc<AtomTable>,
    resolver: Arc<dyn NodeResolver + Send + Sync>,
    connect_timeout: Duration,
    /// Whole-handshake deadline applied around the OTP exchange on both the
    /// outbound `connect` and the inbound accept-side responder. Bounds a stalled
    /// or malicious peer so `connect` always returns and no responder task parks
    /// forever (DISTRIBUTION-HANDSHAKE-DESIGN.md HS-1, D3).
    handshake_timeout: Duration,
    /// Connection lifecycle event hub: multi-subscriber Up/Down delivery,
    /// per-peer session generations, and the legacy single-slot down callback
    /// (which fires LAST, Down only).
    events: ConnectionEventHub,
    control_frame_handler: RwLock<Option<Arc<ControlFrameHandler>>>,
    /// Shared handshake secret. Both peers must agree on this value or the OTP
    /// challenge/response is rejected and the connection is dropped.
    cookie: String,
    /// This node's advertised distribution name, sent in the handshake name
    /// packet so the peer keys its connection table by our identity.
    local_node_name: String,
    /// This node's creation value, sent alongside the name in the handshake.
    local_creation: u32,
    /// Runtime handle that drives the read/accept tasks. In production the
    /// scheduler binds the [`DistSender`](crate::distribution::sender::DistSender)
    /// runtime here so the receive side is driven even though no ambient runtime
    /// exists. When unset (e.g. `#[tokio::test]`), the tasks fall back to the
    /// ambient runtime via bare `tokio::spawn`.
    runtime_handle: RwLock<Option<Handle>>,
    /// Proactive net-tick configuration. When `Some`, every established link runs
    /// a heartbeat task (keepalive + inbound-liveness deadline). `None` disables
    /// the net-tick (links are then only marked down on read EOF/error or a write
    /// timeout, the pre-net-tick behaviour).
    heartbeat: Option<HeartbeatConfig>,
    /// Count of proactive net-tick (heartbeat) tasks spawned since construction,
    /// one per established link when the net-tick is enabled. Reported as the
    /// distribution bundle's heartbeat task-class policy line (spec §3.7/§5) —
    /// heartbeats are async tasks with no OS thread, so they are inventoried as a
    /// counter, never a thread line.
    heartbeat_tasks_spawned: AtomicU64,
    /// Inbound accept-side residency meter (lane #64, D5). Every stream the
    /// listener accepts reserves one peer's worth of receive residency before a
    /// responder task is spawned for it; the reservation is released when that
    /// link's read lifecycle ends. Held as a separate `Arc` rather than inline
    /// so a permit can outlive nothing but itself — see [`InboundResidency`].
    inbound_residency: Arc<InboundResidency>,
}

impl ConnectionManagerInner {
    /// Spawn `future` on the bound runtime handle when one is set, else on the
    /// ambient runtime. Used for the read/accept lifecycle tasks.
    fn spawn_lifecycle<F>(&self, future: F) -> JoinHandle<()>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let handle = self
            .runtime_handle
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        match handle {
            Some(handle) => handle.spawn(future),
            None => tokio::spawn(future),
        }
    }

    /// Build the local handshake descriptor advertised to peers.
    fn handshake_node(&self) -> Result<HandshakeNode, ConnectError> {
        HandshakeNode::with_default_flags(self.local_node_name.clone(), self.local_creation)
            .map_err(|error| ConnectError::Io(error.to_string()))
    }

    /// Produce a per-handshake challenge value. The challenge is drawn from a
    /// cryptographically secure random source, so it is unpredictable per
    /// session. This is the canonical OTP behavior: the shared cookie still
    /// provides authentication, while an unpredictable challenge adds
    /// defense-in-depth against replay (an attacker cannot precompute the
    /// digest for a challenge they cannot guess).
    fn gen_challenge(&self) -> u32 {
        rand::random::<u32>()
    }

    /// Decide the OTP status an inbound responder should emit for a peer whose
    /// advertised name is `peer_name` (HS-3, D1 — OTP verbatim).
    ///
    /// With no competing local outbound to that peer, continue normally. If a
    /// local outbound dial to the same peer name is in flight, break the tie by
    /// literal name comparison: the higher-named node's OUTBOUND survives, so the
    /// responder on the lower-named node continues this inbound
    /// (`ContinueSimultaneous`, when `peer_name > local_name`) and the responder
    /// on the higher-named node rejects it (`Reject`, when `local_name > peer_name`)
    /// to keep its own outbound. Distinct cluster members have unique names, so
    /// equality cannot occur; if it ever did, `Continue` plus the install-time
    /// dedup (HS-2) is the backstop.
    fn decide_inbound_status(&self, peer_name: &str) -> SimultaneousDecision {
        let peer_atom = self.atom_table.intern(peer_name);
        let Some(abort) = self
            .connecting
            .get(&peer_atom)
            .map(|entry| Arc::clone(&entry))
        else {
            return SimultaneousDecision::Continue;
        };
        match peer_name.cmp(self.local_node_name.as_str()) {
            std::cmp::Ordering::Greater => {
                // This (lower-named) node keeps the inbound. Retire its own
                // competing outbound so it does not also install and collide in
                // the HS-2 dedup (§3.2: "mark the local outbound to abort").
                abort.store(true, Ordering::SeqCst);
                SimultaneousDecision::ContinueSimultaneous
            }
            std::cmp::Ordering::Less => SimultaneousDecision::Reject,
            std::cmp::Ordering::Equal => SimultaneousDecision::Continue,
        }
    }

    /// The connection direction that survives a simultaneous connect for `peer`,
    /// computed identically on both nodes by literal name comparison: the
    /// higher-named node's OUTBOUND wins (equivalently, the lower-named node's
    /// INBOUND). This is the install-time backstop that makes
    /// [`ConnectionManager::register_connection`] timing-independent even when the
    /// in-handshake HS-3 tie-break window is missed. The pathological equal-name
    /// case (distinct cluster members never collide) falls to `Inbound`, an
    /// arbitrary-but-consistent local choice; the per-pair agreement that matters
    /// is preserved because names are unique.
    fn canonical_direction(&self, peer: Atom) -> LinkDirection {
        let Some(peer_name) = self.atom_table.resolve(peer) else {
            // Unknown peer name: no competing direction can be reasoned about, so
            // treat the incoming link as the canonical one (install it).
            return LinkDirection::Inbound;
        };
        if self.local_node_name.as_str() > peer_name {
            LinkDirection::Outbound
        } else {
            LinkDirection::Inbound
        }
    }
}

impl ConnectionManagerInner {
    fn connection_down(
        &self,
        node: Atom,
        connection: &Arc<DistConnection>,
        reason: ConnectionDownReason,
    ) {
        use dashmap::mapref::entry::Entry;
        if let Entry::Occupied(occupied) = self.connections.entry(node)
            && Arc::ptr_eq(occupied.get(), connection)
        {
            // Enqueue UNDER the entry guard: a racing register_connection
            // for this node blocks on the entry until we release, so its
            // Up(g+1) can never be queued ahead of this Down(g). The
            // enqueue precedes `remove` only because dashmap's
            // `OccupiedEntry::remove(self)` consumes the guard;
            // INV-DOWN-VISIBILITY still holds because a concurrent
            // dispatcher's `get_connection` blocks on the shard lock this
            // entry holds until the removal completes.
            self.events
                .enqueue(ConnectionEvent::down(node, connection.generation(), reason));
            occupied.remove();
        }
        // Guard released. Deliver with no locks held (same discipline the old
        // hook.invoke had, now ORDERED against concurrent installs and
        // SYNCHRONOUS: when this returns, every subscriber has run). Dispatch
        // even when the ptr-eq LOST: the winner (an HS-4 re-dial that replaced
        // this socket) enqueued this session's Down under the entry guard we
        // just contended on but may not have DELIVERED it yet — returning
        // without draining would let our caller (e.g. `disconnect_node`)
        // return before the Down its own `mark_down` caused was delivered,
        // breaking INV-SYNC. When the queue is already empty this is a cheap
        // bounce off the dispatch gate.
        self.events.dispatch();
    }
}

/// Which side opened a TCP connection, used by the install-time canonical
/// dedup ([`ConnectionManagerInner::canonical_direction`]) to resolve a
/// simultaneous connect deterministically on both nodes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LinkDirection {
    /// This node dialed the peer (`connect`).
    Outbound,
    /// This node accepted the peer's dial (`handle_accepted`).
    Inbound,
}

/// RAII marker that records an in-flight outbound dial in the manager's
/// `connecting` set and clears it on drop, on every `connect` exit path (HS-3).
struct ConnectingGuard {
    inner: Arc<ConnectionManagerInner>,
    peer: Atom,
    /// Set by a concurrent inbound responder (the tie-break) to tell this
    /// outbound to bow out so the reciprocal inbound is the sole survivor.
    abort: Arc<AtomicBool>,
}

impl ConnectingGuard {
    fn new(inner: &Arc<ConnectionManagerInner>, peer_name: &str) -> Self {
        let peer = inner.atom_table.intern(peer_name);
        let abort = Arc::new(AtomicBool::new(false));
        inner.connecting.insert(peer, Arc::clone(&abort));
        Self {
            inner: Arc::clone(inner),
            peer,
            abort,
        }
    }

    /// Whether a concurrent inbound responder has claimed the reciprocal link,
    /// asking this outbound to abort (HS-3 tie-break, §3.2).
    fn is_aborted(&self) -> bool {
        self.abort.load(Ordering::SeqCst)
    }
}

impl Drop for ConnectingGuard {
    fn drop(&mut self) {
        self.inner.connecting.remove(&self.peer);
    }
}

/// Distribution TCP connection manager and active connection table.
#[derive(Clone)]
pub struct ConnectionManager {
    inner: Arc<ConnectionManagerInner>,
}

impl ConnectionManager {
    /// Create a connection manager with the default five-second connect timeout.
    ///
    /// `cookie`, `local_node_name`, and `local_creation` are the local node's
    /// OTP handshake identity: the cookie authenticates peers, while the name and
    /// creation are advertised so a peer keys its connection table by this node.
    #[must_use]
    pub fn new(
        atom_table: Arc<AtomTable>,
        resolver: Arc<dyn NodeResolver + Send + Sync>,
        cookie: impl Into<String>,
        local_node_name: impl Into<String>,
        local_creation: u32,
    ) -> Self {
        Self::with_connect_timeout(
            atom_table,
            resolver,
            cookie,
            local_node_name,
            local_creation,
            DEFAULT_CONNECT_TIMEOUT,
        )
    }

    /// Create a connection manager with a caller-specified connect timeout.
    #[must_use]
    pub fn with_connect_timeout(
        atom_table: Arc<AtomTable>,
        resolver: Arc<dyn NodeResolver + Send + Sync>,
        cookie: impl Into<String>,
        local_node_name: impl Into<String>,
        local_creation: u32,
        connect_timeout: Duration,
    ) -> Self {
        Self {
            inner: Arc::new(ConnectionManagerInner {
                connections: DashMap::new(),
                connecting: DashMap::new(),
                atom_table,
                resolver,
                connect_timeout,
                handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
                events: ConnectionEventHub::new(),
                control_frame_handler: RwLock::new(None),
                cookie: cookie.into(),
                local_node_name: local_node_name.into(),
                local_creation,
                runtime_handle: RwLock::new(None),
                heartbeat: None,
                heartbeat_tasks_spawned: AtomicU64::new(0),
                inbound_residency: Arc::new(InboundResidency::new()),
            }),
        }
    }

    /// Enable the proactive net-tick (heartbeat) on a freshly-built manager.
    ///
    /// Builder-style: must be called before the manager is cloned or any
    /// connection is started, while `inner` is still uniquely owned (the config
    /// is read by per-connection lifecycle tasks at spawn time). Returns `self`
    /// unchanged if the manager has already been shared. `config.deadline` should
    /// exceed `config.interval` so healthy idle links are never spuriously downed.
    #[must_use]
    pub fn with_heartbeat(mut self, config: HeartbeatConfig) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.heartbeat = Some(config);
        }
        self
    }

    /// Bind a tokio runtime handle for the read/accept lifecycle tasks.
    ///
    /// The scheduler calls this with the owned `DistSender` runtime handle so the
    /// receive side is driven in production (where no ambient runtime exists).
    /// Must be called before any connection is established; existing tasks keep
    /// the runtime they were spawned on.
    pub fn set_runtime_handle(&self, handle: Handle) {
        *self
            .inner
            .runtime_handle
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Some(handle);
    }

    /// Return the configured outbound TCP connection timeout.
    #[must_use]
    pub fn connect_timeout(&self) -> Duration {
        self.inner.connect_timeout
    }

    /// Return the configured whole-handshake deadline.
    #[must_use]
    pub fn handshake_timeout(&self) -> Duration {
        self.inner.handshake_timeout
    }

    /// Override the whole-handshake deadline on a freshly-built manager.
    ///
    /// Builder-style: must be called before the manager is cloned or any
    /// connection is started, while its `inner` is still uniquely owned. Returns
    /// `self` unchanged if the manager has already been shared (a clone exists),
    /// since the deadline is read by in-flight handshakes and cannot be mutated
    /// race-free afterward.
    #[must_use]
    pub fn with_handshake_timeout(mut self, handshake_timeout: Duration) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.handshake_timeout = handshake_timeout;
        }
        self
    }

    /// Return a clone of the legacy connection-down callback slot.
    ///
    /// 0.11-compat surface: the slot is replace-on-register, fires LAST
    /// (after every hub subscriber), and only for Down. New consumers should
    /// prefer [`subscribe_connection_events`](Self::subscribe_connection_events).
    #[must_use]
    pub fn connection_down_hook(&self) -> ConnectionDownHook {
        self.inner.events.legacy_down_hook()
    }

    /// Register or replace the legacy connection-down callback.
    ///
    /// 0.11-compat surface: replace-on-register, fires LAST (after every hub
    /// subscriber), Down only. New consumers should prefer
    /// [`subscribe_connection_events`](Self::subscribe_connection_events).
    pub fn register_connection_down<F>(&self, callback: F)
    where
        F: Fn(ConnectionDownEvent) + Send + Sync + 'static,
    {
        self.inner.events.legacy_down_hook().register(callback);
    }

    /// Subscribe to connection lifecycle events (Up + Down). Unlimited
    /// subscribers, invoked in registration order; see the module-level
    /// "Delivery and ordering contract" in
    /// [`connection_events`](super::connection_events). Callbacks must not
    /// block, must not perform socket I/O, and must capture `Weak` (never
    /// `Arc`) handles to anything owning this manager.
    pub fn subscribe_connection_events<F>(&self, callback: F) -> SubscriberId
    where
        F: Fn(ConnectionEvent) + Send + Sync + 'static,
    {
        self.inner.events.subscribe(callback)
    }

    /// Remove a subscription. `false` if the id was not (or no longer)
    /// registered.
    pub fn unsubscribe_connection_events(&self, id: SubscriberId) -> bool {
        self.inner.events.unsubscribe(id)
    }

    /// Subscribe to connection lifecycle events with synthetic catch-up: the
    /// blessed late-subscriber path (INV-NO-REPLAY). Before this method
    /// returns, `callback` is invoked on the calling thread with a synthetic
    /// [`ConnectionEvent::Up`]`(node, generation, peer_creation)` for every
    /// currently live peer (down links excluded), then registered. Snapshot,
    /// synthetic delivery, and registration all happen while holding the
    /// event-dispatch gate, so no real event interleaves between them: the
    /// subscriber observes a per-node stream satisfying INV-ALTERNATION from
    /// its first synthetic Up, missing no session and seeing none twice.
    ///
    /// The synthetic Ups are subscriber-local catch-up: they are delivered to
    /// THIS callback only and are NOT part of the global event order other
    /// subscribers saw (nothing is replayed to, or duplicated for, anyone
    /// else). See the "Delivery and ordering contract" in
    /// [`connection_events`](super::connection_events).
    ///
    /// Do NOT call this from inside a subscriber callback on the same
    /// manager: the reentrancy check then registers and returns the
    /// subscription WITHOUT any synthetic events (no race-free snapshot
    /// exists mid-drain, and blocking would self-deadlock). Callback
    /// discipline is as
    /// [`subscribe_connection_events`](Self::subscribe_connection_events):
    /// must not block, must not perform socket I/O, and must capture `Weak`
    /// (never `Arc`) handles to anything owning this manager.
    pub fn subscribe_connection_events_with_snapshot<F>(&self, callback: F) -> SubscriberId
    where
        F: Fn(ConnectionEvent) + Send + Sync + 'static,
    {
        self.inner
            .events
            .subscribe_with_snapshot(callback, || self.connected_peers())
    }

    /// Snapshot of live connections as their in-force [`NodeUp`] rows (down
    /// links excluded). Per-peer-consistent; no cross-peer atomicity.
    /// Late-subscriber recipe: subscribe FIRST, then snapshot, then per peer
    /// keep the row/event with the highest generation — generation is the
    /// dedupe key. No replay.
    #[must_use]
    pub fn connected_peers(&self) -> Vec<NodeUp> {
        self.inner
            .connections
            .iter()
            .filter(|entry| !entry.value().is_down())
            .map(|entry| NodeUp {
                node: *entry.key(),
                generation: entry.value().generation(),
                peer_creation: entry.value().peer_creation(),
            })
            .collect()
    }

    /// Last generation ever assigned for `node` (even if currently down);
    /// `None` if this manager never installed a connection to `node`.
    #[must_use]
    pub fn last_peer_generation(&self, node: Atom) -> Option<ConnectionGeneration> {
        self.inner.events.last_generation(node)
    }

    /// Register a handler for framed distribution control messages read from active links.
    ///
    /// Legacy shape without the frame's origin; the body wraps
    /// [`Self::register_control_frame_handler_with_origin`] and drops the
    /// origin argument. Kept byte-identical for 0.11 embedders.
    pub fn register_control_frame_handler<F>(&self, handler: F)
    where
        F: Fn(&[u8], &[u8]) + Send + Sync + 'static,
    {
        self.register_control_frame_handler_with_origin(move |_origin, control, payload| {
            handler(control, payload);
        });
    }

    /// Register a handler for framed distribution control messages read from
    /// active links, receiving the authenticated origin with each frame.
    ///
    /// The origin is the connection's node atom — the authenticated handshake
    /// name that keys the connection table — passed by the read loop so the
    /// handler can reject link controls whose `from` field forges another
    /// peer's identity.
    pub fn register_control_frame_handler_with_origin<F>(&self, handler: F)
    where
        F: Fn(Atom, &[u8], &[u8]) + Send + Sync + 'static,
    {
        let mut slot = self
            .inner
            .control_frame_handler
            .write()
            .unwrap_or_else(|error| error.into_inner());
        *slot = Some(Arc::new(handler));
    }

    /// Number of active, identified distribution connections.
    #[must_use]
    pub fn connection_count(&self) -> usize {
        self.inner.connections.len()
    }

    /// Whether the proactive net-tick (heartbeat) is enabled on this manager.
    #[must_use]
    pub fn heartbeat_enabled(&self) -> bool {
        self.inner.heartbeat.is_some()
    }

    /// Count of proactive net-tick (heartbeat) tasks spawned since construction
    /// (spec §3.7/§5). One per established link when the net-tick is enabled;
    /// zero when it is disabled or no link has yet been established.
    #[must_use]
    pub fn heartbeat_tasks_spawned(&self) -> u64 {
        self.inner.heartbeat_tasks_spawned.load(Ordering::Relaxed)
    }

    /// Bytes of inbound receive residency currently reserved by admitted links
    /// (D5). One admitted inbound link reserves the worst case it can hold: a
    /// single framed buffer,
    /// [`MAX_DIST_FRAME_BYTES`](crate::distribution::etf::MAX_DIST_FRAME_BYTES).
    ///
    /// Exposed so the accept bound is observable — a leaked reservation would
    /// shrink the envelope permanently, and a bound nobody can read is a bound
    /// nobody can test.
    #[must_use]
    pub fn inbound_residency_bytes(&self) -> u64 {
        self.inner.inbound_residency.charged.load(Ordering::Acquire)
    }

    /// Accepted streams DECLINED because admitting them would have carried
    /// inbound residency past the envelope (D5), since construction.
    ///
    /// The declined stream is dropped, closing the TCP connection; this counter
    /// is what makes that refusal observable rather than silent.
    #[must_use]
    pub fn inbound_accepts_refused(&self) -> u64 {
        self.inner.inbound_residency.refused.load(Ordering::Relaxed)
    }

    /// The atom table this manager keys its connection table by.
    ///
    /// Connections are keyed by the peer's authenticated handshake name interned
    /// into this table, so callers that look up a connection by name (e.g.
    /// `get_connection(atom_table().intern(peer_name))`) must intern through the
    /// same table the manager used. Exposed for integration tests and callers that
    /// drive the manager directly rather than through the scheduler.
    #[must_use]
    pub fn atom_table(&self) -> Arc<AtomTable> {
        Arc::clone(&self.inner.atom_table)
    }

    /// Look up an active distribution connection by node-name atom.
    #[must_use]
    pub fn get_connection(&self, node: Atom) -> Option<Arc<DistConnection>> {
        self.inner
            .connections
            .get(&node)
            .map(|entry| Arc::clone(entry.value()))
    }

    /// Return the node-name atoms for all active distribution connections.
    #[must_use]
    pub fn connected_nodes(&self) -> Vec<Atom> {
        let mut nodes: Vec<_> = self
            .inner
            .connections
            .iter()
            .map(|entry| *entry.key())
            .collect();
        nodes.sort_unstable_by_key(|node| node.index());
        nodes
    }

    /// Idempotently connect to a node-name atom, returning `false` for transport failures.
    ///
    /// A simultaneous-connect `nok` abort ([`ConnectError::SimultaneousAbort`]) is
    /// treated as success, not a failure: the peer is keeping the reciprocal link,
    /// so the pair is (or is about to be) connected and the caller must not
    /// retry-storm (HS-3).
    ///
    /// The "already connected" early-return only fires for a LIVE link. A link
    /// that has gone down but not yet been reaped from the table (the window
    /// between `mark_down` flipping the flag and `connection_down` removing the
    /// entry) must NOT be reported as connected, or a caller's reconnect attempt
    /// would be told the peer is up and never re-dial. Skipping a down entry here
    /// makes re-dial deterministic: `connect` runs the handshake and
    /// `register_connection` replaces the stale entry (HS-4, §3.4).
    pub async fn connect_node(&self, node: Atom) -> bool {
        if self
            .get_connection(node)
            .is_some_and(|connection| !connection.is_down())
        {
            return true;
        }
        let Some(node_name) = self.inner.atom_table.resolve(node).map(str::to_owned) else {
            return false;
        };
        matches!(
            self.connect(&node_name).await,
            Ok(_) | Err(ConnectError::SimultaneousAbort)
        )
    }

    /// Manually disconnect an active node and emit the connection-down hook once.
    pub fn disconnect_node(&self, node: Atom) -> bool {
        let Some(connection) = self.get_connection(node) else {
            return true;
        };
        connection.mark_down(ConnectionDownReason::ManualDisconnect);
        true
    }

    /// Tear down every active connection and abort every in-flight outbound
    /// dial — the scheduler-shutdown half of §3.6's connection-complete
    /// teardown. Each active connection goes through the ordinary `mark_down`
    /// path (`ManualDisconnect`: the local node is explicitly closing), so its
    /// write half closes immediately (FIN), its read loop is woken to exit, the
    /// table entry is removed, and the Down event is DELIVERED before this
    /// returns (INV-SYNC). In-flight dials get their HS-3 abort flag set; the
    /// dial's own exit paths retire it.
    pub fn disconnect_all(&self) {
        for entry in &self.inner.connecting {
            entry.value().store(true, Ordering::Release);
        }
        let nodes: Vec<Atom> = self
            .inner
            .connections
            .iter()
            .map(|entry| *entry.key())
            .collect();
        for node in nodes {
            self.disconnect_node(node);
        }
    }

    /// Create a manager and start a dedicated asynchronous TCP accept loop.
    pub async fn start(
        listen_addr: SocketAddr,
        resolver: Arc<dyn NodeResolver + Send + Sync>,
        cookie: impl Into<String>,
        local_node_name: impl Into<String>,
        local_creation: u32,
    ) -> io::Result<(Self, AcceptHandle)> {
        let manager = Self::new(
            Arc::new(AtomTable::with_common_atoms()),
            resolver,
            cookie,
            local_node_name,
            local_creation,
        );
        let handle = manager.listen(listen_addr).await?;
        Ok((manager, handle))
    }

    /// Start a dedicated asynchronous TCP accept loop for this manager.
    pub async fn listen(&self, listen_addr: SocketAddr) -> io::Result<AcceptHandle> {
        let listener = TcpListener::bind(listen_addr).await?;
        Ok(self.listen_with(listener))
    }

    /// Start a dedicated asynchronous TCP accept loop on a pre-bound listener.
    ///
    /// Separated from [`listen`](Self::listen) so callers that must bind the
    /// listener before the manager exists (e.g. to publish the chosen port into a
    /// resolver) can reuse the same accept-loop spawn. The accept loop runs on the
    /// bound runtime handle via `ConnectionManagerInner::spawn_lifecycle`.
    #[must_use]
    pub fn listen_with(&self, listener: TcpListener) -> AcceptHandle {
        let local_addr = listener
            .local_addr()
            .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        let shutdown = Arc::new(Notify::new());
        let task_shutdown = Arc::clone(&shutdown);
        let manager = self.clone();
        let task = self.inner.spawn_lifecycle(async move {
            manager.accept_loop(listener, task_shutdown).await;
        });
        AcceptHandle {
            local_addr,
            shutdown,
            task,
        }
    }

    /// Install an authenticated link, deduplicating against an existing `Up`
    /// connection for the same peer name (HS-2) by the deterministic
    /// canonical-direction rule.
    ///
    /// Two simultaneous handshakes (one inbound, one outbound) for the same pair
    /// can both reach this point — and on a busy host BOTH outbounds can finish
    /// and register before either inbound responder is scheduled, so the HS-3
    /// in-handshake tie-break window is missed entirely. A first-registered-wins
    /// dedup then resolves the collision differently on the two nodes (whichever
    /// direction happened to register first locally), so each node can drop the
    /// very socket its peer is keeping — leaving the pair with zero live links and
    /// no re-dial.
    ///
    /// The fix is timing-independent: for any pair both nodes agree, by literal
    /// name comparison, that the survivor is the HIGHER-named node's OUTBOUND
    /// connection (equivalently the lower-named node's inbound) — the same single
    /// TCP socket on both ends. The dedup keyed on the incumbent's stored
    /// [`LinkDirection`]: a newcomer loses ONLY to a LIVE incumbent of the
    /// canonical direction carrying the same peer incarnation; otherwise it
    /// installs, displacing a down, non-canonical, or stale-incarnation
    /// incumbent (a nonzero `peer_creation` mismatch proves the peer
    /// restarted — a session boundary the tie-break must not shield, so the
    /// old session closes with Down(g) and the newcomer opens Up(g+1)). So
    /// during a simultaneous connect each node keeps
    /// only its canonical-direction link (the canonical socket is never torn down
    /// by either side), while a LONE re-dial — which only ever meets a stale,
    /// non-canonical, or absent incumbent — always re-establishes the link.
    ///
    /// `admission` carries the accept-side residency reservation for an INBOUND
    /// link (D5); outbound dials and the test helper pass `None`. Every exit
    /// from here either hands it to the installed link's read lifecycle or lets
    /// it drop — including the dedup arm below, where a losing newcomer's
    /// reservation is returned because no buffer of its own will ever exist.
    fn register_connection(
        &self,
        node: Atom,
        peer_addr: SocketAddr,
        stream: TcpStream,
        direction: LinkDirection,
        peer_creation: u32,
        admission: Option<InboundAdmissionPermit>,
    ) -> io::Result<Arc<DistConnection>> {
        use dashmap::mapref::entry::Entry;

        // The fallible half FIRST, before the entry lock: a connection whose
        // teardown dup cannot be created (fd exhaustion) is REFUSED here —
        // never installed with a degraded closure guarantee (spec §3.6).
        let socket = PreparedSocket::prepare(stream)?;
        let canonical = self.inner.canonical_direction(node);
        // The connection installed (if any) plus the displaced link to retire
        // AFTER the entry guard is released — `mark_down` re-enters this same
        // `connections` map, so calling it while holding the entry lock would
        // deadlock on the shard.
        let (installed, read_half, displaced) = match self.inner.connections.entry(node) {
            Entry::Occupied(mut occupied) => {
                let incumbent = occupied.get();
                // A nonzero-creation mismatch proves the incumbent serves a
                // DEAD peer incarnation: the peer restarted (its old
                // incarnation died without a FIN/RST reaching us — silent
                // partition, power loss, kill-9 + fast restart) and this
                // newcomer is the restarted VM's dial. The canonical
                // tie-break below exists to resolve SAME-incarnation
                // simultaneous connects; it must never shield a stale
                // incarnation, so a creation-mismatch newcomer always
                // installs. 0 is the handshake-less test-helper sentinel,
                // never a discriminator.
                let creation_mismatch = incumbent.peer_creation() != 0
                    && peer_creation != 0
                    && incumbent.peer_creation() != peer_creation;
                if !incumbent.is_down() && incumbent.direction == canonical && !creation_mismatch {
                    // A LIVE incumbent of the canonical direction, serving the
                    // same peer incarnation as far as the handshake can tell,
                    // already holds this pair — it is the rightful survivor on
                    // both nodes, so this newcomer loses regardless of its own
                    // direction. Drop its stream (closing the TCP connection)
                    // and do NOT spawn a reader. (A lone re-dial never hits
                    // this: the only live incumbent it could meet is a stale
                    // non-canonical link or a stale incarnation, both handled
                    // below.)
                    drop(socket);
                    return Ok(Arc::clone(incumbent));
                }
                // The incumbent is down (reap/reconnect), a non-canonical link
                // this newcomer is entitled to replace (a simultaneous-connect
                // canonical winner, or a lone re-dial superseding a stale
                // link), OR a stale incarnation losing to the restarted peer's
                // dial. Install this one and retire the old.
                let previous = Arc::clone(incumbent);
                // Sample the down flag ONCE: it can flip concurrently, and the
                // generation choice and the Down+Up emission must agree.
                let previous_down = previous.is_down();
                // A LIVE incumbent displaced by a NEW peer incarnation —
                // canonical or not — is a peer bounce, not a socket swap.
                // That is a session boundary: inheriting the generation here
                // would swallow the bounce forever (no pg purge, no
                // noconnection delivery, no peer_creation change on any Up).
                let peer_bounced = !previous_down && creation_mismatch;
                let generation = if previous_down {
                    // HS-4 re-dial window: the incumbent went down but its own
                    // `connection_down` has not (or will not, having lost the
                    // ptr-eq race after this replacement) removed the entry.
                    // Close the old session HERE, under the same entry guard
                    // any competing emission site needs, so its Down is never
                    // lost and always precedes the new session's Up.
                    self.inner.events.enqueue(ConnectionEvent::down(
                        node,
                        previous.generation(),
                        // No unwrap: the fallback is reachable only when a test
                        // flips `down` directly without `mark_down` recording a
                        // reason.
                        previous
                            .down_reason
                            .get()
                            .copied()
                            .unwrap_or(ConnectionDownReason::ReadError),
                    ));
                    self.inner.events.next_generation(node)
                } else if peer_bounced {
                    // Close the old incarnation's session under the same entry
                    // guard the HS-4 arm uses, so the Down is never lost and
                    // always precedes the new session's Up. The stale link
                    // never reported a failure (no reason recorded), so the
                    // reason is ReadError — matching the retirement reason the
                    // displaced socket itself is marked down with below.
                    self.inner.events.enqueue(ConnectionEvent::down(
                        node,
                        previous.generation(),
                        ConnectionDownReason::ReadError,
                    ));
                    self.inner.events.next_generation(node)
                } else {
                    // Live same-incarnation displacement (simultaneous
                    // connect): same logical session, so the newcomer inherits
                    // the generation and no event fires.
                    previous.generation()
                };
                let (connection, read_half) = self.build_connection(
                    node,
                    peer_addr,
                    socket,
                    direction,
                    generation,
                    peer_creation,
                );
                occupied.insert(Arc::clone(&connection));
                if previous_down || peer_bounced {
                    self.inner
                        .events
                        .enqueue(ConnectionEvent::up(node, generation, peer_creation));
                }
                (connection, read_half, Some(previous))
            }
            Entry::Vacant(vacant) => {
                let generation = self.inner.events.next_generation(node);
                let (connection, read_half) = self.build_connection(
                    node,
                    peer_addr,
                    socket,
                    direction,
                    generation,
                    peer_creation,
                );
                let entry_ref = vacant.insert(Arc::clone(&connection));
                // Enqueue AFTER the table mutation, still under the entry
                // guard: no event can be delivered while the table does not
                // yet reflect it (INV-UP-VISIBILITY).
                self.inner
                    .events
                    .enqueue(ConnectionEvent::up(node, generation, peer_creation));
                drop(entry_ref);
                (connection, read_half, None)
            }
        };
        // Entry guard dropped: safe to re-enter the map. The displaced link's
        // `connection_down` ptr-eq guard sees the freshly inserted entry (not the
        // displaced one), so it does not evict the survivor (and enqueues no
        // event); it wakes the old link's read loop to drop its socket, and its
        // unconditional dispatch may deliver the events this install queued —
        // harmless: same thread, same order, still before the read lifecycle
        // spawns below.
        if let Some(previous) = displaced {
            previous.mark_down(ConnectionDownReason::ReadError);
        }
        // Deliver BEFORE the read lifecycle spawns: no generation-g inbound
        // frame can reach the control-frame handler before Up(g) delivery
        // completes (INV-FRAME-ORDER), and a queued prior Down's cleanup has
        // run before the new generation's read loop exists.
        self.inner.events.dispatch();
        self.spawn_read_lifecycle(Arc::clone(&installed), read_half, admission);
        Ok(installed)
    }

    /// Split a stream into a [`DistConnection`] and its read half, without
    /// touching the connection table. Shared by both `register_connection` arms.
    fn build_connection(
        &self,
        node: Atom,
        peer_addr: SocketAddr,
        socket: PreparedSocket,
        direction: LinkDirection,
        generation: ConnectionGeneration,
        peer_creation: u32,
    ) -> (Arc<DistConnection>, OwnedReadHalf) {
        let (connection, read_half) = DistConnection::new(
            node,
            peer_addr,
            socket,
            Arc::downgrade(&self.inner),
            direction,
            generation,
            peer_creation,
        );
        (Arc::new(connection), read_half)
    }

    /// Register a pre-connected standard stream for native BIF unit tests.
    ///
    /// `peer_creation` is 0, the documented "no handshake" sentinel: this
    /// helper skips the handshake, so there is no peer incarnation to surface
    /// (and the peer-bounce discriminator never fires on the sentinel).
    #[cfg(test)]
    pub(crate) fn register_test_connection(
        &self,
        node: Atom,
        peer_addr: SocketAddr,
        stream: std::net::TcpStream,
    ) -> io::Result<Arc<DistConnection>> {
        self.register_test_connection_with_creation(node, peer_addr, stream, 0)
    }

    /// [`Self::register_test_connection`] with an explicit `peer_creation`,
    /// for tests exercising the peer-bounce (creation-mismatch) install arm.
    #[cfg(test)]
    pub(crate) fn register_test_connection_with_creation(
        &self,
        node: Atom,
        peer_addr: SocketAddr,
        stream: std::net::TcpStream,
        peer_creation: u32,
    ) -> io::Result<Arc<DistConnection>> {
        stream.set_nonblocking(true)?;
        let stream = TcpStream::from_std(stream)?;
        // Test helper: a pre-connected stream, no handshake, so the direction
        // only selects the install arm; `Inbound` here. `None` admission — this
        // stream never passed through `accept_loop`, so it holds no reservation
        // to hand on, and a helper that minted one would let tests drain an
        // envelope the accept path never spent.
        self.register_connection(
            node,
            peer_addr,
            stream,
            LinkDirection::Inbound,
            peer_creation,
            None,
        )
    }
}
