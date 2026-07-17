//! Browser connection-event hub (WPORT-4 R2): the socket v0.2 vocabulary fill.
//!
//! Distribution is COMPILED OUT of the wasm closure (`beamr-wasm` selects only
//! `cooperative,json`; no `net`), so there is no `ConnectionManager` here. This
//! hub is HOST-FED: the JS host (haematite's carrier, or any embedder) TELLS
//! beamr-wasm about connection fate through the exported ingress, and the hub
//! provides the subscription surface and the vocabulary mirror — event
//! delivery, not connection management. It mirrors the native hub in
//! `beamr::distribution::connection_events` exactly; it never invents a second
//! model.
//!
//! # Host-identity contract (tear Ruling 2)
//!
//! The host feeds `{node, peer_creation}` on up-ingress and `{node, reason}`
//! on down-ingress — NOTHING else. Generation is NEVER host-supplied: the hub
//! mints it locally (per-peer monotonic from 1, mirroring native
//! `ConnectionGeneration`'s never-on-the-wire law), so INV-ALTERNATION and
//! generation density are enforced at this seam rather than trusted from the
//! host. The explicit `connection_replaced(node, new_peer_creation, reason)`
//! ingress expands atomically into `Down(g, reason)` then `Up(g+1)`. A bare
//! double-Up without an intervening Down is a LOUD typed error
//! (`ConnectionEventProtocolError`), never silently coerced.
//!
//! # Vocabulary mirror
//!
//! Events are delivered to subscriber callbacks as JSON strings (the wrapper's
//! host-value convention) with exactly two shapes and NO third variant:
//!
//! - `{"type":"up","node":N,"generation":G,"peer_creation":C}` — mirrors
//!   native `NodeUp`; `peer_creation` — not generation — answers
//!   restart-vs-blip.
//! - `{"type":"down","node":N,"generation":G,"reason":R}` — mirrors native
//!   `NodeDown`; `generation` always equals the most recent Up generation.
//!
//! "Replaced" is NOT an event variant: it is the native sequence `Down(g)`
//! then `Up(g+1)` with `peer_creation` as the restart discriminator.
//!
//! # Down-reason mapping (tear Ruling 3): the seven native variants, exactly
//!
//! | ingress string        | native `ConnectionDownReason` | browser fate mapped                         |
//! |-----------------------|-------------------------------|---------------------------------------------|
//! | `peer_closed`         | `PeerClosed`                  | clean close from the peer (normal closure)  |
//! | `read_error`          | `ReadError`                   | transport receive/message error             |
//! | `write_error`         | `WriteError`                  | transport send failure                      |
//! | `write_timeout`       | `WriteTimeout`                | send-side backpressure deadline exceeded    |
//! | `manual_disconnect`   | `ManualDisconnect`            | local side explicitly closed the connection |
//! | `heartbeat_timeout`   | `HeartbeatTimeout`            | liveness deadline with no inbound traffic   |
//! | `control_overflow`    | `ControlOverflow`             | must-deliver control lane overflowed        |
//!
//! There is NO local non-exhaustive extension: a browser-only variant IS the
//! second model the spec forbids. A fate string outside this table is a LOUD
//! typed error — a genuinely unmappable browser fate is a STOP routed to the
//! native vocabulary owner (the native enum is `#[non_exhaustive]`; any new
//! variant lands THERE, keeping one model).
//!
//! # Subscription surface (tear Ruling 1)
//!
//! Mirrors BOTH native method names — `subscribe_connection_events`,
//! `subscribe_connection_events_with_snapshot` — plus
//! `unsubscribe_connection_events -> bool`. `SubscriberId` is NUMERIC (a
//! disposable handle would import lifetime/drop ambiguity across the
//! wasm-bindgen boundary that the native contract does not have). Snapshot
//! delivery is SYNCHRONOUS: single-threaded wasm makes the native
//! no-interleaving contract hold by construction under sync delivery, whereas
//! an async snapshot would create precisely the synthetic-vs-real interleaving
//! window the native contract prohibits. Reentrant registration through the
//! snapshot path (from inside a subscriber callback) registers WITHOUT
//! catch-up — the native rule, carried over verbatim.
//!
//! # NO-POLLING (binding, arc law verbatim)
//!
//! "A polling scheduler is a TEAR CONDITION (F-0d acceptance). Any timer whose
//! job is 'check whether something changed' is a design error." Subscribers
//! are TOLD: every callback invocation happens synchronously inside a host-fed
//! ingress call (or the synchronous snapshot). No branch of this module arms a
//! recurring callback, timer fallback, or poll, and no branch touches the
//! wake-path scheduling surface (`request_external_turn` /
//! `schedule_external_edge`): subscribers are JS callbacks, not VM-resident
//! consumers. If a VM-resident consumer is ever wired to this hub, its
//! delivery must consume the EXISTING arbiter surface.
//!
//! # Ten-invariant classification (tear-added requirement)
//!
//! Each named native invariant
//! (`beamr::distribution::connection_events`, module docs) is classified as
//! **wasm-seam-asserted** (enforced by this hub's code and pinned by a wasm
//! test) or **inherited-by-construction** (holds structurally from the
//! host-fed single-threaded design, with the argument stated):
//!
//! 1. **INV-ALTERNATION — wasm-seam-asserted.** The per-peer state machine
//!    admits Up only when the peer is down and Down only when it is up (loud
//!    typed error otherwise) and mints generations locally, monotonic from 1;
//!    `connection_replaced` expands to `Down(g)` + `Up(g+1)` atomically.
//!    Pinned by the churn wall's alternation/density verifier.
//! 2. **INV-EXACTLY-ONCE — wasm-seam-asserted.** Each transition mints or
//!    closes exactly one generation exactly once (same state machine); as in
//!    the native manager, hub teardown synthesizes no Downs for still-open
//!    sessions. Pinned by the churn wall's density check (a repeat is a
//!    double-see, a gap a missed session).
//! 3. **INV-TOTAL-ORDER — inherited-by-construction.** One thread and one
//!    FIFO queue: all subscribers observe all events in the single queue
//!    order; within one event, subscribers run in registration order.
//! 4. **INV-SYNC — inherited-by-construction.** Dispatch is synchronous with
//!    ingress: when an ingress call returns, every event it produced has been
//!    delivered to every subscriber. The native sole exception carries over
//!    verbatim: a transition fed from INSIDE a callback is queued and
//!    delivered after that callback returns, before the outermost ingress
//!    returns (the `dispatching` latch).
//! 5. **INV-UP-VISIBILITY — inherited-by-construction.** The peer record (the
//!    hub's only table) is mutated before the event is enqueued and delivered
//!    within the same synchronous frame; there is no connection table in the
//!    wasm closure for a callback to observe stale.
//! 6. **INV-DOWN-VISIBILITY — inherited-by-construction.** Same structural
//!    argument: the record is marked down before `Down(g)` delivery begins.
//! 7. **INV-FRAME-ORDER — inherited-by-construction at this seam, obligation
//!    named.** The hub owns no socket and processes no frames
//!    (`SOCKET-CARRIER-v0.2` is externally filled, owner Apollo Biscuit). The
//!    hub guarantees the analogue it can: queue order means no generation-g
//!    event is observable before `Up(g)` delivery completed. Frame-vs-event
//!    ordering is the carrier's contract, outside the wasm closure.
//! 8. **INV-SUB-DISCIPLINE — inherited-by-construction for the stall clause,
//!    wasm-seam-asserted for the reentrancy grants.** Single-threaded wasm has
//!    no reads/heartbeats/accepts for a slow callback to stall (it blocks only
//!    the host turn it runs in). Callbacks MAY re-enter subscribe /
//!    unsubscribe / ingress on this hub — pinned by the churn wall's
//!    reentrant registration.
//! 9. **INV-SCHED-FIRST — inherited-by-construction (vacuous at this seam).**
//!    No scheduler subscriber and no pg exist in the wasm closure, so there is
//!    no purge ordering to preserve. A future VM-resident subscriber must
//!    register first and ride the existing arbiter surface.
//! 10. **INV-NO-REPLAY — wasm-seam-asserted.** No history is replayed, ever.
//!     The blessed late-subscriber path is
//!     `subscribe_connection_events_with_snapshot`: subscriber-local synthetic
//!     `Up` rows for every live peer, delivered under the dispatch latch
//!     before registration, invisible to every other subscriber. Pinned by
//!     the snapshot and locality walls.
//!
//! # Downstream constraint, named but not built
//!
//! The banked WPORT-10 browser-carrier amendment rides exactly this
//! vocabulary ("open/down/reconnect ride the same hub semantics,
//! snapshot-at-subscribe for join state"). This module builds nothing for it.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};

use js_sys::Function;
use serde_json::json;
use wasm_bindgen::JsValue;

/// The seven down reasons, mirrored one-to-one from the native
/// `ConnectionDownReason` — NO local variant (tear Ruling 3).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum DownReason {
    /// The peer closed its side cleanly.
    PeerClosed,
    /// A receive operation reported an error.
    ReadError,
    /// A send operation reported an error.
    WriteError,
    /// A send exceeded its backpressure deadline.
    WriteTimeout,
    /// The local side explicitly closed the connection.
    ManualDisconnect,
    /// No inbound traffic within the liveness deadline.
    HeartbeatTimeout,
    /// The must-deliver control lane overflowed.
    ControlOverflow,
}

impl DownReason {
    /// Parse the canonical ingress string; `None` for any fate outside the
    /// ruled seven-variant mapping (the caller raises the loud typed error).
    fn parse(reason: &str) -> Option<Self> {
        match reason {
            "peer_closed" => Some(Self::PeerClosed),
            "read_error" => Some(Self::ReadError),
            "write_error" => Some(Self::WriteError),
            "write_timeout" => Some(Self::WriteTimeout),
            "manual_disconnect" => Some(Self::ManualDisconnect),
            "heartbeat_timeout" => Some(Self::HeartbeatTimeout),
            "control_overflow" => Some(Self::ControlOverflow),
            _ => None,
        }
    }

    /// The canonical serialized name (round-trips the ingress string).
    const fn as_str(self) -> &'static str {
        match self {
            Self::PeerClosed => "peer_closed",
            Self::ReadError => "read_error",
            Self::WriteError => "write_error",
            Self::WriteTimeout => "write_timeout",
            Self::ManualDisconnect => "manual_disconnect",
            Self::HeartbeatTimeout => "heartbeat_timeout",
            Self::ControlOverflow => "control_overflow",
        }
    }
}

/// Connection lifecycle event: exactly the two native variants. "Replaced" is
/// the sequence `Down(g)` then `Up(g+1)`, never a third shape.
#[derive(Clone, Debug, Eq, PartialEq)]
enum HubEvent {
    /// A peer session opened (mirrors native `NodeUp`).
    Up {
        /// Peer identity, the host's node name key.
        node: String,
        /// Locally minted session generation (monotonic from 1 per peer).
        generation: u64,
        /// Host-fed peer incarnation: the restart-vs-blip discriminator.
        peer_creation: u32,
    },
    /// A peer session closed (mirrors native `NodeDown`).
    Down {
        /// Peer identity, the host's node name key.
        node: String,
        /// Always equals the most recent Up generation for `node`.
        generation: u64,
        /// Why the session closed, drawn from the ruled mapping.
        reason: DownReason,
    },
}

impl HubEvent {
    /// Serialize to the JSON shape delivered to subscriber callbacks.
    fn to_json(&self) -> String {
        match self {
            Self::Up {
                node,
                generation,
                peer_creation,
            } => json!({
                "type": "up",
                "node": node,
                "generation": generation,
                "peer_creation": peer_creation,
            })
            .to_string(),
            Self::Down {
                node,
                generation,
                reason,
            } => json!({
                "type": "down",
                "node": node,
                "generation": generation,
                "reason": reason.as_str(),
            })
            .to_string(),
        }
    }
}

/// Per-peer session state: the seam INV-ALTERNATION is enforced against.
#[derive(Default)]
struct PeerRecord {
    /// Last generation minted for this peer (0 = none yet; generations start
    /// at 1). Never reset while the hub lives, so generations stay dense.
    generation: u64,
    /// Peer incarnation carried by the in-force (or most recent) session.
    peer_creation: u32,
    /// Whether a session is currently open.
    up: bool,
}

/// Host-fed browser connection-event hub: vocabulary mirror, ingress, and
/// subscription surface. Owned by `WasmVm`; all state is interior-mutable so
/// subscriber callbacks may re-enter every method (single-threaded).
#[derive(Default)]
pub(crate) struct BrowserConnectionHub {
    /// Per-peer session records, keyed by the host's node name.
    peers: RefCell<BTreeMap<String, PeerRecord>>,
    /// `(id, callback)` pairs; registration order == invocation order.
    subscribers: RefCell<Vec<(u32, Function)>>,
    /// Next numeric `SubscriberId` (tear Ruling 1: numeric, never a handle).
    next_subscriber_id: Cell<u32>,
    /// Pending events in one global order (mirrors the native hub queue).
    queue: RefCell<VecDeque<HubEvent>>,
    /// Single-threaded analogue of the native dispatch gate: set while a
    /// dispatch (or a snapshot-subscribe) is on the stack, so reentrant
    /// ingress enqueues for the outer loop and reentrant snapshot-subscribe
    /// degrades to plain registration.
    dispatching: Cell<bool>,
}

impl BrowserConnectionHub {
    /// Create an empty hub: no peers, no subscribers, nothing queued.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Up-ingress: the host reports a session opened for `node` with the
    /// authenticated `peer_creation`. The hub mints the generation locally.
    ///
    /// # Errors
    ///
    /// A bare double-Up (the peer is already up) is a loud typed
    /// `ConnectionEventProtocolError` — never silently coerced.
    pub(crate) fn connection_up(&self, node: &str, peer_creation: u32) -> Result<(), JsValue> {
        let generation = {
            let mut peers = self.peers.borrow_mut();
            let record = peers.entry(node.to_owned()).or_default();
            if record.up {
                return Err(protocol_error(
                    "bare_double_up",
                    &format!(
                        "connection_up for {node}: a session is already open; a replacement \
                         must come through connection_replaced (Down(g) then Up(g+1))"
                    ),
                ));
            }
            record.generation = record.generation.saturating_add(1);
            record.peer_creation = peer_creation;
            record.up = true;
            record.generation
        }; // peers borrow dropped before any host callback runs
        self.deliver([HubEvent::Up {
            node: node.to_owned(),
            generation,
            peer_creation,
        }]);
        Ok(())
    }

    /// Down-ingress: the host reports the open session for `node` closed for
    /// `reason` (one of the seven canonical mapping strings).
    ///
    /// # Errors
    ///
    /// An unmapped reason string or a Down with no open session is a loud
    /// typed `ConnectionEventProtocolError`.
    pub(crate) fn connection_down(&self, node: &str, reason: &str) -> Result<(), JsValue> {
        let reason = parse_reason(node, reason)?;
        let generation = self.close_open_session(node, "connection_down")?;
        self.deliver([HubEvent::Down {
            node: node.to_owned(),
            generation,
            reason,
        }]);
        Ok(())
    }

    /// Replacement ingress: the host reports the open session for `node` was
    /// displaced by a new peer incarnation. Expands ATOMICALLY into
    /// `Down(g, reason)` then `Up(g+1, new_peer_creation)` — both events are
    /// queued before delivery begins, so no other ingress can interleave
    /// (tear Ruling 2; the spec's "map richer fate states explicitly").
    ///
    /// # Errors
    ///
    /// An unmapped reason string or a replacement with no open session is a
    /// loud typed `ConnectionEventProtocolError`.
    pub(crate) fn connection_replaced(
        &self,
        node: &str,
        new_peer_creation: u32,
        reason: &str,
    ) -> Result<(), JsValue> {
        let reason = parse_reason(node, reason)?;
        let (down_generation, up_generation) = {
            let mut peers = self.peers.borrow_mut();
            let Some(record) = peers.get_mut(node).filter(|record| record.up) else {
                return Err(protocol_error(
                    "replaced_without_session",
                    &format!("connection_replaced for {node}: no session is open"),
                ));
            };
            let down_generation = record.generation;
            record.generation = record.generation.saturating_add(1);
            record.peer_creation = new_peer_creation;
            (down_generation, record.generation)
        }; // peers borrow dropped before any host callback runs
        self.deliver([
            HubEvent::Down {
                node: node.to_owned(),
                generation: down_generation,
                reason,
            },
            HubEvent::Up {
                node: node.to_owned(),
                generation: up_generation,
                peer_creation: new_peer_creation,
            },
        ]);
        Ok(())
    }

    /// Register `callback` for subsequent events (no catch-up). Returns the
    /// numeric `SubscriberId`. Invocation order is registration order.
    pub(crate) fn subscribe(&self, callback: Function) -> u32 {
        let id = self.next_subscriber_id.get();
        self.next_subscriber_id.set(id.saturating_add(1));
        self.subscribers.borrow_mut().push((id, callback));
        id
    }

    /// Register `callback` with subscriber-local synthetic catch-up: one
    /// synthetic `Up` per currently live peer is delivered SYNCHRONOUSLY to
    /// this callback only — never queued, never seen by other subscribers —
    /// then the callback is registered (INV-NO-REPLAY, the blessed
    /// late-subscriber path).
    ///
    /// Called reentrantly (from inside a subscriber callback), this registers
    /// and returns WITHOUT synthetic events — the native rule verbatim: no
    /// race-free snapshot exists mid-dispatch.
    pub(crate) fn subscribe_with_snapshot(&self, callback: Function) -> u32 {
        if self.dispatching.get() {
            return self.subscribe(callback);
        }
        // Hold the dispatch latch across snapshot + synthetic delivery +
        // registration — the single-threaded analogue of the native dispatch
        // gate. No real event can interleave (single thread); a transition
        // fed from inside the catch-up callback is queued and delivered
        // after registration below, postdating the snapshot (correct order,
        // not replay).
        self.dispatching.set(true);
        let rows: Vec<HubEvent> = {
            let peers = self.peers.borrow();
            peers
                .iter()
                .filter(|(_, record)| record.up)
                .map(|(node, record)| HubEvent::Up {
                    node: node.clone(),
                    generation: record.generation,
                    peer_creation: record.peer_creation,
                })
                .collect()
        }; // peers borrow dropped before any host callback runs
        for row in rows {
            let payload = JsValue::from_str(&row.to_json());
            // A throwing subscriber must not poison the hub (see drain_queue).
            let _thrown = callback.call1(&JsValue::UNDEFINED, &payload);
        }
        let id = self.subscribe(callback);
        self.drain_queue();
        self.dispatching.set(false);
        id
    }

    /// Remove a subscription by its numeric id; `false` if the id is unknown
    /// or already removed. Delivery stops from the next event: an event whose
    /// subscriber snapshot was already taken mid-dispatch still completes
    /// (the native "NOT a barrier" clause).
    pub(crate) fn unsubscribe(&self, id: u32) -> bool {
        let mut subscribers = self.subscribers.borrow_mut();
        let before = subscribers.len();
        subscribers.retain(|(subscriber_id, _)| *subscriber_id != id);
        subscribers.len() != before
    }

    /// Queue `events` and deliver everything pending, unless a dispatch is
    /// already on the stack (reentrant ingress from inside a callback): then
    /// the outer loop delivers after the current callback returns — the
    /// INV-SYNC exception, carried over verbatim.
    fn deliver<I: IntoIterator<Item = HubEvent>>(&self, events: I) {
        self.queue.borrow_mut().extend(events);
        if self.dispatching.get() {
            return;
        }
        self.dispatching.set(true);
        self.drain_queue();
        self.dispatching.set(false);
    }

    /// Deliver every queued event to the current subscribers, in queue order,
    /// snapshotting the subscriber list per event so callbacks may re-enter
    /// subscribe/unsubscribe freely. Every hub borrow is dropped before any
    /// host callback is invoked (the WPORT-3 pen-note discipline).
    fn drain_queue(&self) {
        loop {
            let event = self.queue.borrow_mut().pop_front();
            let Some(event) = event else { break };
            let payload = JsValue::from_str(&event.to_json());
            let snapshot: Vec<Function> = self
                .subscribers
                .borrow()
                .iter()
                .map(|(_, callback)| callback.clone())
                .collect(); // subscribers borrow dropped before host calls
            for callback in snapshot {
                // A subscriber exception must not sever delivery to later
                // subscribers or later events; the hub is not the host's
                // exception channel.
                let _thrown = callback.call1(&JsValue::UNDEFINED, &payload);
            }
        }
    }
}

/// Parse a down-reason ingress string against the ruled seven-variant mapping.
///
/// # Errors
///
/// A string outside the table is the loud typed error: a browser fate that
/// cannot map is a STOP routed to the native vocabulary owner, never a local
/// invention.
fn parse_reason(node: &str, reason: &str) -> Result<DownReason, JsValue> {
    DownReason::parse(reason).ok_or_else(|| {
        protocol_error(
            "unmapped_down_reason",
            &format!(
                "down reason {reason:?} for {node} is outside the ruled mapping onto the seven \
                 native ConnectionDownReason variants; a new browser fate must be routed to the \
                 native vocabulary owner (the native enum is #[non_exhaustive]), never invented \
                 locally"
            ),
        )
    })
}

/// The loud typed error every protocol violation raises: a `js_sys::Error`
/// named `ConnectionEventProtocolError` whose message starts with the
/// violation kind.
fn protocol_error(kind: &str, detail: &str) -> JsValue {
    let error = js_sys::Error::new(&format!("{kind}: {detail}"));
    error.set_name("ConnectionEventProtocolError");
    error.into()
}

impl BrowserConnectionHub {
    /// Close the open session for `node`, returning its generation.
    ///
    /// # Errors
    ///
    /// A close with no open session violates INV-ALTERNATION at the seam and
    /// is a loud typed `ConnectionEventProtocolError`.
    fn close_open_session(&self, node: &str, ingress: &str) -> Result<u64, JsValue> {
        let mut peers = self.peers.borrow_mut();
        let Some(record) = peers.get_mut(node).filter(|record| record.up) else {
            return Err(protocol_error(
                "down_without_up",
                &format!("{ingress} for {node}: no session is open"),
            ));
        };
        record.up = false;
        Ok(record.generation)
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
#[path = "connection_events_tests.rs"]
mod tests;
