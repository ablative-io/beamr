# beamr connection-events hook — final implementation spec (work item B-hook)

Multi-subscriber connection lifecycle events (Down + Up), generation-tagged, replacing production use
of the single replace-on-register `ConnectionDownHook` slot while keeping that slot as 0.11-compat
surface. Every symbol below is verified against source at HEAD (`0e7c060`, v0.12.1; every
distribution/scheduler file cited is byte-identical across `5cb997b..0e7c060`); read the cited
file:line before editing. All paths relative to `crates/beamr/`. No wire changes anywhere in this item.

Companion package: the design-constraints package (constraints C1–C14, risks R1–R6, scenarios 1–10)
and work item A (link/monitor/exit on the wire) — this item lands FIRST (R2).

---

## 0. Decision summary

- **D6 = option (b), extended**: multi-subscriber ordered registry + connection-UP events +
  per-peer session generations + a FIFO event queue drained under a **blocking dispatch gate with
  owner-thread reentrancy detection** giving one global delivery order AND strict synchronous
  delivery (INV-SYNC). No priority tiers (no consumer needs to run before pg-purge; additive later).
  Async fan-out (c) rejected: over-engineering for three subscribers and it forfeits
  purge-on-socket-drop (C12). Single composed closure (a) rejected for the public surface:
  perpetuates R2 for embedders.
- **Dispatch mechanism ruling (judges split; decided here).** The two candidate mechanisms both
  fail a judge-identified test: a non-blocking single-drainer baton makes delivery *eventual* under
  contention — the `mark_down` caller can return before pg-purge ran, bending C12's
  synchronous-on-socket-drop letter and leaving the stale-purge race open; a `std::sync::Once`
  UP-latch deadlocks on same-thread re-entry (`call_once` from inside its own closure) and asserts
  a cross-generation ordering it cannot deliver. The synthesis: **enqueue under the connections
  entry guard** (total per-node order, from the concurrency design) + **drain under a blocking
  `Mutex` whose holder's `ThreadId` is recorded** so a same-thread reentrant `dispatch()` returns
  immediately (outer frame delivers) while a cross-thread `dispatch()` *waits* and therefore
  returns only after its events are delivered. This discharges C12 strictly, closes the
  stale-purge-vs-new-generation race structurally (§7 H1), keeps consumers'-style synchronicity
  without the Once deadlock, and costs only a bounded cross-thread wait under concurrent
  transitions (§7 H2). A non-blocking baton remains the recorded escape hatch (§10 deferred).
- **Up event carries `peer_creation`** (from `HandshakeResult::remote_creation`,
  handshake.rs:224-226): the bounce-vs-blip discriminator generations alone cannot provide.
  `direction`/`peer_addr` NOT on the event — deliberate partial deviation from one judge's graft:
  under the session-generation model the serving *socket* can be displaced mid-session with no
  event (§3.3), so socket-identity facts on a session event would go stale silently; they are
  socket facts, reachable via `get_connection(node)` inside the Up callback
  (`DistConnection::peer_addr` connection.rs:275-277), and `NodeUp` is `#[non_exhaustive]` so
  adding them later is additive (§10 deferred).
- **Generation = logical per-peer connectivity session**, not socket: inherited across
  simultaneous-connect displacement, closed exactly once. Formalizes today's
  fires-iff-table-entry-removed behavior (connection.rs:496-511) and closes the pre-existing HS-4
  silent Down-loss hole (§3.3).
- **Up is delivered BEFORE the connection's read loop spawns** (judges' open question, ruled):
  `register_connection` dispatches after table install, before `spawn_read_lifecycle`. Buys
  INV-FRAME-ORDER (no generation-g inbound frame precedes Up(g) delivery — what work item A wants
  for control-lane init) and, combined with blocking dispatch, closes the stale-purge race. C7
  cost is a subscriber-chain-bounded delay to liveness seeding/heartbeat start against a 45s
  deadline (connection.rs:48) — negligible under INV-SUB-DISCIPLINE.
- **Scheduler internals migrate as ONE composed subscriber** (pg-purge then noconnection delivery,
  sequential statements in one body): scenario-5 ordering is structural, not
  registration-order-dependent. Registration order across independent subscribers remains a
  documented, tested invariant for embedders.
- **D7**: pg-purge migrates (semantics unchanged, C12), then `supervision_integration::connection_down`
  goes live (both `#[allow(dead_code)]` removed, supervision_integration.rs:277, :326).
  `global::remove_node` (global.rs:156-164) **deferred with evidence**: `GlobalNameRegistry` is
  never constructed in production — every `GlobalNameRegistry::new` site is test code
  (native/distribution_bifs.rs:171, :202, :231, :256, all inside `mod tests` at :157;
  global.rs:308/:315/:330/:352 are its own tests). There is no registry instance on `SharedState`
  to purge. Dead-node control-queue cleanup deferred to work item A (the must-deliver lane does
  not exist yet).
- **D9 = overturned with evidence — deferred-not-skipped, not a latent bug.** The Executing
  non-trap arm of `process_remote_exit_signal` (supervision_integration.rs:318-320) only inserts an
  exit tombstone, but an Executing slot always has a worker mid-slice that must store back:
  `tombstone_reason` is checked after every slice (execution/core.rs:69-73 →
  `cleanup_exited_process`, the full cascade+DOWN path) and every store-back outcome arm re-checks
  via `cleanup_if_tombstoned_after_store` (execution/core.rs:78, :87, :168; definition :450-458).
  Pinned by a test (§8, scenario 8). Work item A inherits this ruling.
- **Noconnection delivery runs INLINE** on the delivering thread (R3 ruled): the deadlock
  precondition — a slot lock held across blocking distribution I/O — is provably absent.
  `block_on_distribution_send` (supervision_integration.rs:682-730) is called with no slot guard
  held and never needs the dist runtime (non-async-context branch builds a private current-thread
  runtime, :704-729); every slot-touching applier drops slot+entry before wake
  (supervision_integration.rs:299-301, :304-306, :315-317; deliver_payload :733+). The only
  cross-thread wait is the per-connection writer mutex, bounded by `WRITE_TIMEOUT` 5s
  (sender.rs:72, :172).
- **Legacy single-slot hook fires LAST, Down only, 0.11 shape** (judges split 2-1; ruled LAST).
  Rationale: today a legacy registrant *replaces* pg-purge entirely (connection.rs:131-140; sole
  production registrant scheduler/mod.rs:871-880), so no 0.11 embedder can have depended on
  observing pre-purge state — the purge would have been gone. Last = settled state, pinned by
  test. No `#[deprecated]` attribute (would force `#[allow(deprecated)]` in the in-tree
  legacy-bridge tests under `-D warnings`, banned); rustdoc steers to the new API instead.
- **No event replay.** Late subscribers reconcile via `connected_peers()` +
  `last_peer_generation()` (subscribe first, then snapshot, keep max-by-generation per peer).
- D1–D5, D8 are work-item-A scope; nothing here touches `ControlRouter`, the codecs,
  `control_lifecycle`/`control_monitor` (R5 avoided), or any frame format.

---

## 1. Files touched

| File | Change | LOC effect |
|---|---|---|
| `src/distribution/connection_events.rs` | **NEW**: event types, generations, hub+dispatch, moved legacy types, contract rustdoc | ~400, under limit |
| `src/distribution/connection_events_tests.rs` | **NEW** sibling test file (house pattern: `mod pg_tests` distribution/mod.rs:155-156) | test-only |
| `src/distribution/connection.rs` (2537 lines, over budget) | −88 moved types, +~55 (fields, plumbing, 4 API methods, re-exports) | **net ≈ −33** |
| `src/distribution/mod.rs` | module decl + re-exports | +5 |
| `src/scheduler/connection_lifecycle.rs` | **NEW**: composed scheduler subscriber | ~90 |
| `src/scheduler/connection_lifecycle_tests.rs` | **NEW** sibling test file | test-only |
| `src/scheduler/mod.rs` (1367 lines) | replace closure :871-880 with one call; 2 module decls | net ≈ −5 |
| `src/scheduler/supervision_integration.rs` (2289 lines, over budget) | delete 2 `#[allow(dead_code)]` + false comments (:277, :326) | **net ≈ −2** |
| `src/scheduler/supervision_tests.rs` | promote 7 fixture helpers to `pub(super)` | ~0 |
| `tests/connection_events_e2e.rs` | **NEW**, `#![cfg(feature = "net")]` (pattern: distribution_mesh_handshake.rs:54) | test-only |
| `docs/adr/012-connection-events-hub.md` | **NEW** ADR extending ADR-009 | ~40 |

Both over-limit files shrink; all new logic is in new modules (C14). Feature gating mirrors
siblings exactly: `connection_events` sits inside `distribution/` which is `#[cfg(feature = "net")]`
at lib.rs:14-15 (no extra attribute needed, matching connection.rs); the two new scheduler modules
get `#[cfg(feature = "threads")]` exactly like `mod supervision_integration` (scheduler/mod.rs:53-97
gates every scheduler submodule and every `crate::distribution` import behind `threads`).
**Verified**: `cargo check -p beamr --no-default-features --features threads` fails TODAY with 119
pre-existing errors (`threads` does not imply `net`, Cargo.toml:70-71, yet the threads-gated
scheduler imports `crate::distribution` unconditionally) — that combination is not a supported
build and this spec does not change that. Gate bar covers default features and `--all-features`.

---

## 2. New module `src/distribution/connection_events.rs`

### 2.1 Moved types (verbatim relocation, zero semantic change)

`ConnectionDownReason` (connection.rs:84-103), `ConnectionDownEvent` (:106-112),
`ConnectionDownCallback` alias (:114), `ConnectionDownHook` (:118-170) move here.
Two changes only:
- `ConnectionDownHook::invoke` becomes `pub(crate)` (now called cross-module by the hub).
- The `HeartbeatTimeout` doc-comment (:98-102) drops the false "monitor-DOWN machinery" claim
  (package ground-truth doc-lie); new text: "…so the connection-event hub fires (pg-purge,
  noconnection delivery, embedder subscribers). Monitor-DOWN is work item A."

Path compat (C13): `connection.rs` keeps every 0.11-style import path alive:

```rust
pub use super::connection_events::{
    ConnectionDownEvent, ConnectionDownHook, ConnectionDownReason,
};
```

`ConnectionDownEvent{node, reason}` gains NO fields (adding one breaks 0.11 exhaustive
destructuring — generation lives only on the new types). `ConnectionDownReason` gains no variants.

### 2.2 New public types

```rust
/// Monotonic per-peer connectivity-session counter, LOCAL to this node.
/// Strictly increasing per peer name for the lifetime of the ConnectionManager.
/// NOT the peer's OTP creation; NOT on the wire; NOT comparable across nodes;
/// resets when the local manager is rebuilt.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ConnectionGeneration(u64);
impl ConnectionGeneration {
    #[must_use] pub const fn get(self) -> u64;
    /// For consumer test harnesses; production values come from events.
    #[must_use] pub const fn from_raw(raw: u64) -> Self;
}

/// A peer transitioned disconnected -> connected (session opened).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NodeUp {
    /// Peer's authenticated handshake-name atom (the connection-table key).
    pub node: Atom,
    pub generation: ConnectionGeneration,
    /// Peer incarnation from the authenticated handshake
    /// (HandshakeResult::remote_creation, handshake.rs:224-226). 0 is the
    /// "no handshake" sentinel used by the in-crate test helper — production
    /// installs always come through the handshake. Changes iff the peer VM
    /// restarted: THIS — not `generation` — answers "did the peer bounce or
    /// did the link blip?".
    pub peer_creation: u32,
}

/// A peer transitioned connected -> disconnected (session closed).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NodeDown {
    pub node: Atom,
    /// Always equals the most recent NodeUp generation delivered for `node`.
    pub generation: ConnectionGeneration,
    pub reason: ConnectionDownReason,
}

/// Connection lifecycle event. #[non_exhaustive]: cross-crate subscribers must
/// use a wildcard arm; in-crate matches stay exhaustive WITHOUT a `_` arm
/// (non_exhaustive is inert in-crate; a trailing `_` trips unreachable_patterns
/// under -D warnings).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConnectionEvent {
    Up(NodeUp),
    Down(NodeDown),
}
impl ConnectionEvent {
    // Constructors + accessors so cross-crate consumers (liminal/haematite/frame
    // tests) can fabricate and inspect events despite #[non_exhaustive].
    #[must_use] pub fn up(node: Atom, generation: ConnectionGeneration, peer_creation: u32) -> Self;
    #[must_use] pub fn down(node: Atom, generation: ConnectionGeneration, reason: ConnectionDownReason) -> Self;
    #[must_use] pub fn node(&self) -> Atom;
    #[must_use] pub fn generation(&self) -> ConnectionGeneration;
    #[must_use] pub fn down_reason(&self) -> Option<ConnectionDownReason>; // None for Up
}

/// Opaque handle identifying one subscription; pass to unsubscribe.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct SubscriberId(u64);

type ConnectionEventCallback = dyn Fn(ConnectionEvent) + Send + Sync + 'static;
```

Naming: `NodeUp`/`NodeDown` echo OTP net_kernel `{nodeup,N}`/`{nodedown,N}` and avoid collision
with the legacy `ConnectionDownEvent`. Events are `Copy`, passed by value (legacy callback style).

### 2.3 The hub

```rust
/// Multi-subscriber registration point + delivery queue for connection
/// lifecycle events, plus the legacy single-slot down callback (invoked LAST
/// on Down). Owned by ConnectionManagerInner.
#[derive(Default)]
pub(crate) struct ConnectionEventHub {
    /// (id, callback) pairs; registration order == invocation order.
    subscribers: RwLock<Vec<(SubscriberId, Arc<ConnectionEventCallback>)>>,
    next_subscriber_id: AtomicU64,
    /// The legacy replace-on-register slot (register_connection_down target).
    legacy_down: ConnectionDownHook,
    /// Last-assigned generation per peer. Mutated ONLY under the caller's
    /// connections-table entry guard for that peer (§5 lock order). Entries
    /// never removed: bounded by cluster size.
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
    dispatch_owner: Mutex<Option<std::thread::ThreadId>>,
}

impl ConnectionEventHub {
    pub(crate) fn new() -> Self;
    pub(crate) fn subscribe<F>(&self, callback: F) -> SubscriberId
    where F: Fn(ConnectionEvent) + Send + Sync + 'static;
    /// Returns false if the id is unknown / already removed. NOT a barrier: an
    /// in-flight dispatch that already snapshotted may invoke the callback once
    /// more after unsubscribe returns.
    pub(crate) fn unsubscribe(&self, id: SubscriberId) -> bool;
    /// Clone sharing the slot (connection_down_hook() / register_connection_down).
    pub(crate) fn legacy_down_hook(&self) -> ConnectionDownHook;
    /// Next session generation for `node` (starts at 1). MUST be called while
    /// holding the connections entry guard for `node`, so assignment order ==
    /// install order and generations are strictly increasing per peer.
    pub(crate) fn next_generation(&self, node: Atom) -> ConnectionGeneration;
    /// Read-only: last generation ever assigned for `node`.
    pub(crate) fn last_generation(&self, node: Atom) -> Option<ConnectionGeneration>;
    /// Append to the delivery queue. MUST be called while holding the
    /// connections entry guard for the event's node (this is what totally
    /// orders a node's events).
    pub(crate) fn enqueue(&self, event: ConnectionEvent);
    /// Drain and deliver pending events. MUST be called with NO manager locks
    /// held. Blocking: on return, every event enqueued before the call has
    /// been delivered to all subscribers (by this thread or by the concurrent
    /// drainer it waited on) — EXCEPT when called reentrantly from inside a
    /// subscriber on the draining thread, where it returns immediately and the
    /// outer drain delivers after the current callback returns.
    pub(crate) fn dispatch(&self);
}
```

All lock acquisitions use the house poisoned-lock idiom
`unwrap_or_else(|error| error.into_inner())` (connection.rs:135-138). No unwrap/expect/panic (C14).

### 2.4 Dispatch algorithm (normative)

```rust
pub(crate) fn dispatch(&self) {
    let me = std::thread::current().id();
    // Reentrancy check: if THIS thread already holds the gate we are inside a
    // subscriber callback; the outer drain loop delivers whatever we enqueued,
    // after the current callback returns. Blocking here would self-deadlock.
    {
        let owner = self.dispatch_owner.lock().unwrap_or_else(|e| e.into_inner());
        if *owner == Some(me) {
            return;
        }
    } // owner lock released before blocking on the gate
    let _gate = self.dispatch_gate.lock().unwrap_or_else(|e| e.into_inner());
    // RAII owner record: reset even if a (test-only) subscriber panics, so a
    // poisoned pass cannot wedge reentrancy detection for this thread forever.
    let _owner = OwnerGuard::set(&self.dispatch_owner, me);
    loop {
        let event = {
            let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
            queue.pop_front()
        }; // queue lock released before any callback runs
        let Some(event) = event else { break };
        // Snapshot under the read lock, release, then invoke: no hub lock is
        // held during a callback, so callbacks may re-enter
        // subscribe/unsubscribe/the manager freely.
        let snapshot: Vec<Arc<ConnectionEventCallback>> = self
            .subscribers.read().unwrap_or_else(|e| e.into_inner())
            .iter().map(|(_, callback)| Arc::clone(callback)).collect();
        for callback in snapshot {
            callback(event);
        }
        if let ConnectionEvent::Down(down) = event {
            // Legacy slot LAST, 0.11 shape (§0 ruling).
            self.legacy_down.invoke(ConnectionDownEvent {
                node: down.node,
                reason: down.reason,
            });
        }
    }
    // _owner resets dispatch_owner to None, then _gate releases.
}
```

Properties, each pinned by a unit test (§8):

- **Single drainer** ⇒ one global total order; per-node order = enqueue order = table-transition
  order (enqueue holds that node's entry guard).
- **No lost events, no lost-wakeup loop needed**: a producer enqueues *then* calls `dispatch()`;
  it either drains its own event after acquiring the gate, or blocks while the holder — who loops
  until the queue is empty before releasing — delivers it. Either way the event is delivered
  before the producer's `dispatch()` returns (INV-SYNC).
- **No hub lock held at invoke**; callbacks may subscribe/unsubscribe/re-enter the manager.
- **Reentrancy-safe**: a subscriber that triggers a transition (e.g. `disconnect_node`) enqueues;
  its nested `dispatch()` sees `dispatch_owner == me` and returns; the outer loop pops the event
  next iteration — delivered after the current callback returns, before the outermost `dispatch()`
  returns. No self-deadlock (the Once-latch failure mode is structurally impossible).
- Snapshot semantics: a subscriber registered mid-dispatch does not receive the event whose
  snapshot preceded it; an unsubscribed one may receive one in-flight event.

---

## 3. Changes to `src/distribution/connection.rs`

### 3.1 `DistConnection` (struct :200-223, ctor :226-244)

```rust
    /// Session generation this socket serves (immutable; inherited across
    /// simultaneous-connect displacement).
    generation: ConnectionGeneration,
    /// Peer incarnation from the authenticated handshake (0 = handshake-less
    /// test helper sentinel).
    peer_creation: u32,
    /// Reason recorded by mark_down BEFORE the down flag flips, so any observer
    /// of is_down()==true reads Some (set → swap(AcqRel) gives happens-before).
    /// First-set-wins on a mark_down race: both reasons are genuine.
    down_reason: std::sync::OnceLock<ConnectionDownReason>,
```

`DistConnection::new` gains `generation: ConnectionGeneration, peer_creation: u32` (private, sole
caller `build_connection`). New public accessors:

```rust
#[must_use] pub fn generation(&self) -> ConnectionGeneration;
#[must_use] pub fn peer_creation(&self) -> u32;
```

`mark_down` (:310-320) gains ONE line before the swap:

```rust
fn mark_down(self: &Arc<Self>, reason: ConnectionDownReason) {
    let _ = self.down_reason.set(reason);                 // NEW
    if self.down.swap(true, Ordering::AcqRel) { return; }
    self.shutdown.notify_waiters();
    if let Some(manager) = self.manager.upgrade() {
        manager.connection_down(self.node, self, reason);
    }
}
```

### 3.2 `ConnectionManagerInner` (:356-402, init :591-616)

Field `connection_down_hook: ConnectionDownHook` (:381) is **replaced** by
`events: ConnectionEventHub` (the legacy slot lives inside the hub). `with_connect_timeout`
(:591-616) initializes `events: ConnectionEventHub::new()`.

### 3.3 `register_connection` — event-aware arms (:935-988)

Signature gains `peer_creation: u32`; `build_connection` (:992-1008) gains
`generation: ConnectionGeneration, peer_creation: u32`. Call sites:
- `connect` (:908-909): `self.register_connection(node, peer_addr, stream, LinkDirection::Outbound, result.remote_creation())`
- `handle_accepted` (:1186-1188): `manager.register_connection(node, peer_addr, stream, LinkDirection::Inbound, result.remote_creation())`
- `register_test_connection` (:1011-1024, `#[cfg(test)]`): passes `0` (documented sentinel: no
  handshake ⇒ no peer incarnation; no production or consumer contract reads 0 as meaningful —
  `remote_creation` is otherwise consumed only inside the handshake module and its tests).

Arm behavior (same skeleton, same guard discipline — the :944-948 and :979-982 comments stay valid):

| Existing arm | Generation | Events enqueued (in order, UNDER the entry guard, AFTER the table mutation) |
|---|---|---|
| Occupied, incumbent live + canonical (newcomer loses, :952-961) | — (newcomer dropped) | none |
| Occupied, incumbent live + non-canonical (HS-2/HS-3 displacement, :962-970) | **inherit** `incumbent.generation()` | none — same logical session |
| Occupied, incumbent `is_down()` (HS-4 re-dial window, :962-970) | close old, open new | `Down{node, old_gen, old_reason}` then `Up{node, new_gen, peer_creation}` |
| Vacant (:972-977) | `next_generation(node)` | `Up{node, gen, peer_creation}` |

HS-4 arm detail (this closes the pre-existing silent Down-loss hole: today the replaced down
incumbent's own `connection_down` fails ptr-eq after the swap and its hook firing is lost forever):

```rust
let generation = if previous.is_down() {
    self.inner.events.enqueue(ConnectionEvent::down(
        node,
        previous.generation(),
        previous.down_reason.get().copied().unwrap_or(ConnectionDownReason::ReadError), // no unwrap (C14); fallback reachable only when tests flip `down` directly (connection.rs:2043)
    ));
    self.inner.events.next_generation(node)
} else {
    previous.generation() // live non-canonical displacement: inherit, emit nothing
};
let (connection, read_half) = self.build_connection(node, peer_addr, stream, direction, generation, peer_creation);
occupied.insert(Arc::clone(&connection));                  // guard retained (OccupiedEntry::insert(&mut self))
if previous.is_down() {
    self.inner.events.enqueue(ConnectionEvent::up(node, generation, peer_creation));
}
```

pg note on the HS-4 arm (judge-raised, answered): emitting `Down(old)` here runs pg-purge for a
peer that is mid-reconnect — immediately followed by `Up(new)` in the same drain. That is exactly
the normal reap path's down→purge→redial→up sequence compressed into one install; it introduces no
new pg semantics, and the reconnection e2e
(`reconnection_down_up_rejoin_reestablishes_membership_without_stale_resurrection`,
pg_distribution_e2e.rs:469) exercises the normal reap path, not this race window, so it is
unaffected (and must pass unmodified, §8).

Vacant arm: `let entry_ref = vacant.insert(Arc::clone(&connection));` (dashmap 6.1
`VacantEntry::insert(self) -> RefMut` — shard guard retained), then
`self.inner.events.enqueue(ConnectionEvent::up(node, generation, peer_creation));`, then drop the
ref. **Enqueue-after-mutation, still under the guard** — no event can be delivered while the table
does not yet reflect it (queue order per node is guard order; delivery happens only after some
guard release).

Tail of `register_connection` (guard released):

```rust
if let Some(previous) = displaced {
    previous.mark_down(ConnectionDownReason::ReadError);   // unchanged (:983-985); ptr-eq shields
                                                           // the survivor: no enqueue, no dispatch
}
self.inner.events.dispatch();                              // NEW: BEFORE the read lifecycle —
                                                           // INV-FRAME-ORDER + closes H1 (§7)
self.spawn_read_lifecycle(Arc::clone(&installed), read_half);  // unchanged body (:1026-1032)
installed
```

Dispatch-before-spawn ruling (§0): when `dispatch()` returns, every queued event — including a
prior `Down(g_old)` — has been delivered; only then does generation g's read loop (and heartbeat,
:1028-1029) start. A slow subscriber delays liveness seeding by one subscriber chain, bounded by
INV-SUB-DISCIPLINE, against a 45s heartbeat deadline (connection.rs:48) and a peer whose frames
merely wait in the kernel buffer. `register_connection` is called from async context on the dist
runtime (`connect` :909, `handle_accepted` :1188); the bounded gate wait there is the same class
as running the callbacks inline, which today's hook already does (§7 H2).

### 3.4 `ConnectionManagerInner::connection_down` — rewritten (:496-511)

```rust
fn connection_down(&self, node: Atom, connection: &Arc<DistConnection>,
                   reason: ConnectionDownReason) {
    use dashmap::mapref::entry::Entry;
    let removed = match self.connections.entry(node) {
        Entry::Occupied(occupied) if Arc::ptr_eq(occupied.get(), connection) => {
            // Enqueue UNDER the entry guard: a racing register_connection for
            // this node blocks on the entry until we release, so its Up(g+1)
            // can never be queued ahead of this Down(g).
            self.events.enqueue(ConnectionEvent::down(node, connection.generation(), reason));
            occupied.remove();
            true
        }
        _ => false,
    };
    // Guard released. Deliver with no locks held (same discipline the old
    // hook.invoke had, now ORDERED against concurrent installs and SYNCHRONOUS:
    // when this returns, purge + noconnection delivery have run).
    if removed {
        self.events.dispatch();
    }
}
```

Equivalence with the old `remove_if` ptr-eq (:502-505) is pinned by the existing tests passing
unmodified: `manual_disconnect_removes_connection_and_notifies_once` (connection.rs:1620),
`write_error_removes_connection_and_notifies_once` (:1652),
`net_tick_marks_black_holed_peer_down_within_deadline` (:2422). C10 is unchanged and unweakened:
`connection_down` still runs only from `mark_down`, whose callers hold no guards (:944-948 rule);
the entry taken here is the first lock in the chain.

**Down enqueue precedes `occupied.remove()` because dashmap's `OccupiedEntry::remove(self)`
consumes the guard.** INV-DOWN-VISIBILITY nevertheless holds via shard-lock serialization, which
this spec pins as a named dependency: a concurrent dispatcher that pops this Down and whose
callback calls `get_connection(node)` blocks on the shard read lock held by this entry guard until
`remove` completes — so the callback can never observe the generation-g connection still installed.
(dashmap `get` takes the shard `RwLock` read side; `entry` holds the write side for its lifetime —
core dashmap behavior, dashmap 6.1.0 pinned in Cargo.toml.)

**Exactly-once Down per generation — structural argument (C10 extended):**
1. Per socket: `down.swap(true, AcqRel)` (:311) ⇒ `connection_down` runs at most once per
   `DistConnection`.
2. Per session: `Down{g}` has exactly two emission sites — `connection_down` (ptr-eq under the
   entry guard) and `register_connection`'s HS-4 arm (`is_down()` incumbent replaced under the same
   entry guard). Both mutate the same entry under its guard and both verify connection identity
   first; whichever runs second observes the entry already changed (removed / replaced) and emits
   nothing. Mutual exclusion is the entry lock itself; no extra flag.
3. `generation` is immutable on `DistConnection`, so the emitted value cannot tear.

### 3.5 `ConnectionManager` public API (all additive)

```rust
/// Subscribe to connection lifecycle events (Up + Down). Unlimited
/// subscribers, invoked in registration order; see the module-level
/// "Delivery and ordering contract" in connection_events. Callbacks must not
/// block, must not perform socket I/O, and must capture Weak (never Arc)
/// handles to anything owning this manager (C7/C9).
pub fn subscribe_connection_events<F>(&self, callback: F) -> SubscriberId
where F: Fn(ConnectionEvent) + Send + Sync + 'static;

/// Remove a subscription. false if the id was not (or no longer) registered.
pub fn unsubscribe_connection_events(&self, id: SubscriberId) -> bool;

/// Snapshot of live connections as their in-force NodeUp rows (filter
/// !is_down()). Per-peer-consistent; no cross-peer atomicity. Late-subscriber
/// recipe: subscribe FIRST, then snapshot, then per peer keep the row/event
/// with the highest generation — generation is the dedupe key. No replay.
#[must_use] pub fn connected_peers(&self) -> Vec<NodeUp>;

/// Last generation ever assigned for `node` (even if currently down); None if
/// this manager never installed a connection to `node`.
#[must_use] pub fn last_peer_generation(&self, node: Atom) -> Option<ConnectionGeneration>;
```

`register_connection_down` (:681-686) and `connection_down_hook()` (:676-678) keep their exact
signatures and replace-semantics, delegating to `self.inner.events.legacy_down_hook()`. This is the
R2 fix: a late legacy registrant can no longer evict pg-purge, because pg-purge no longer lives in
that slot. sender.rs's tests using `register_connection_down` (:360, :464) compile and pass
unmodified.

**C13 kept-compiling inventory (byte-for-byte signature-identical):**
`ConnectionManager::{connected_nodes, get_connection, set_runtime_handle, register_connection_down,
connection_down_hook, connect_node, disconnect_node, connect, listen, listen_with, start,
connection_count, atom_table, connect_timeout, handshake_timeout, with_heartbeat,
with_handshake_timeout}` (:730, :721, :639, :681, :676, :755, :772, connect, :800, :812, :781,
:703, :715, :649, :655, :626, :667), `DistConnection::{write_raw, node, peer_addr, is_down,
mark_down_write_timeout}` (:288, :269, :275, :281, :306), `ConnectionDownEvent{node, reason}`
(public fields, no additions), `ConnectionDownReason` (no variant changes), `ConnectionDownHook`
(all methods), `AcceptHandle`, `HeartbeatConfig`, `ConnectError`. Import paths preserved via the
§2.1 re-export. `PgRegistry`, `control::encode_*`, `Scheduler::{distribution_connections,
atom_table, pg_registry, start_distribution_listener}` untouched. Everything else in this spec is
a new symbol.

### 3.6 `src/distribution/mod.rs`

```rust
pub mod connection_events;
#[cfg(test)]
mod connection_events_tests;
pub use connection_events::{ConnectionEvent, ConnectionGeneration, NodeDown, NodeUp, SubscriberId};
```

(`ConnectionManager` re-export already at mod.rs:17.)

---

## 4. Scheduler wiring — new module `src/scheduler/connection_lifecycle.rs`

`supervision_integration.rs` (2289 lines) must not grow; the migration lands here.

```rust
//! Scheduler-side connection-event subscriber: node-death cleanup in a FIXED
//! structural order (registry purges before exit-signal delivery). One
//! composed subscriber, not several, so scenario-5 ordering is sequential
//! statements — not registration-order-dependent (R2 cannot recur one level up).

use std::sync::{Arc, Weak};
use super::{SharedState, supervision_integration};
use crate::distribution::connection_events::ConnectionEvent;

/// Register the scheduler's composed connection-event subscriber. Called once
/// from Scheduler construction, before the embedder can subscribe, so embedder
/// subscribers always observe post-purge, post-noconnection state (INV-SCHED-FIRST).
/// Captures Weak<SharedState> (C9): the closure is stored inside
/// ConnectionManagerInner, which SharedState owns — a strong capture would
/// leak every scheduler forever (mirror supervision_integration.rs:77-93).
pub(super) fn register_scheduler_connection_subscriber(shared: &Arc<SharedState>) {
    let weak: Weak<SharedState> = Arc::downgrade(shared);
    shared
        .distribution_connections
        .subscribe_connection_events(move |event| {
            if let Some(shared) = weak.upgrade() {
                handle_connection_event(&shared, event);
            }
        });
    // SubscriberId intentionally discarded: scheduler-lifetime subscription.
}

/// Structural ordering — do not reorder (test scenario 5 depends on it):
///   1. pg purge: a trap-exit handler receiving {'EXIT', _, noconnection} must
///      never observe the dead node's members (C12; semantics of
///      pg.rs:287-292 unchanged).
///   2. [seam: global-name purge (global.rs:156-164) — DEFERRED: GlobalNameRegistry
///      is never constructed in production; every ::new site is test code
///      (native/distribution_bifs.rs:171/:202/:231/:256 under `mod tests` :157;
///      global.rs:308+ own tests). Wire only after a registry lands on SharedState.]
///   3. [seam: dead-node control-lane cleanup — work item A, package D2.]
///   4. noconnection delivery to every local process remote-linked to the node.
/// Up is reserved for work item A control-lane (re)initialization; the match
/// is exhaustive WITHOUT a `_` arm (non_exhaustive is inert in-crate; a
/// wildcard would trip unreachable_patterns under -D warnings).
pub(super) fn handle_connection_event(shared: &Arc<SharedState>, event: ConnectionEvent) {
    match event {
        ConnectionEvent::Down(down) => {
            shared.pg_registry.purge_remote_node(down.node);
            supervision_integration::connection_down(shared, down.node);
        }
        ConnectionEvent::Up(_) => {}
    }
}
```

`handle_connection_event` is `pub(super)` so tests drive it directly (redelivery idempotence, §8).

**`src/scheduler/mod.rs` deltas:** replace the pg-purge closure block (:871-880, comment included)
with `connection_lifecycle::register_scheduler_connection_subscriber(&shared);` placed after
`register_distribution_control_handler(&shared)` (:860) and the pg propagation install (:861-870,
untouched). Add `#[cfg(feature = "threads")] mod connection_lifecycle;` and
`#[cfg(all(test, feature = "threads"))] mod connection_lifecycle_tests;` next to
`mod supervision_tests` (:1363-1364).

**`src/scheduler/supervision_integration.rs` deltas:** delete `#[allow(dead_code)]` and its false
"Called by distribution connection layer and tests" comment at :277
(`process_remote_exit_signal`) and :326 (`connection_down`); replace with doc-comments naming the
real caller chain (hub → `connection_lifecycle::handle_connection_event` → `connection_down` →
`process_remote_exit_signal`). Bodies and signatures unchanged. No `#[allow]` added anywhere.

**`src/scheduler/supervision_tests.rs`:** promote `insert_process` (:26), `read_mailbox_tuple`
(:46), `is_alive` (:63), `add_remote_link` (:83), `set_trap_exit` (:96), `make_executing` (:129),
`make_shared_state` (:249) to `pub(super)` for reuse by `connection_lifecycle_tests` (test-only).
`make_shared_state` builds a REAL `ConnectionManager` (:255-261), so the composed subscriber and
`register_test_connection` work against it; tests that install a real socket must build a tokio
runtime and `set_runtime_handle` first (`spawn_lifecycle` falls back to ambient `tokio::spawn`
otherwise, connection.rs:404-419; existing precedent connection.rs:2271).

### Call-graph delta

```
Scheduler::new (scheduler/mod.rs)
  - DELETE :871-880 (register_connection_down pg closure)
  + connection_lifecycle::register_scheduler_connection_subscriber(&shared)

mark_down (connection.rs:310)  [callers unchanged: read EOF :1057-1059, read errors :1072/:1077/:1091,
                                write timeout :306, heartbeat :1123, disconnect_node :776,
                                displaced-link retire :984]
  └─ ConnectionManagerInner::connection_down
       └─ entry guard: ptr-eq → enqueue(Down{node,gen,reason}) → remove   [was: remove_if → hook.invoke]
       └─ dispatch()   [BLOCKING: returns only after delivery]
            └─ per event, subscribers in registration order:
                 ├─ scheduler composed subscriber
                 │    ├─ pg_registry.purge_remote_node(node)        (pg.rs:287-292; was legacy slot)
                 │    └─ supervision_integration::connection_down    (was dead code, :326)
                 │         └─ process_remote_exit_signal per remote link (:277)
                 │              └─ terminate+cleanup | enqueue_remote_exit_message_pub+wake
                 │                 | pending_exit_messages | tombstone (drained: execution/core.rs:69,:78,:355,:450)
                 ├─ embedder subscribers (liminal, haematite/Apollo, frame)
                 └─ legacy ConnectionDownHook::invoke(ConnectionDownEvent{node,reason})  [tail, Down only]

register_connection (connect :908 | handle_accepted :1188 | register_test_connection :1023)
  └─ entry guard: [next_generation | inherit] → build_connection(.., gen, peer_creation)
                  → insert → enqueue(Up / Down+Up)
  └─ guard released: displaced.mark_down (unchanged, no event) → dispatch() → spawn_read_lifecycle
```

**Wire encodings: none.** Events are strictly node-local. `ConnectionGeneration` is a local
observation counter, never transmitted; `peer_creation` already crosses the wire in the existing
handshake and is merely surfaced. No opcode, frame, keepalive (connection.rs:38, :1064-1069), or
codec change (C6 trivially preserved).

---

## 5. Lock discipline (complete one-way order)

```
connections DashMap entry/shard guard
  ├─> events.generations shard   (next_generation only; leaf)
  └─> events.queue mutex         (enqueue only; push/pop only, never held across anything)
dispatch_gate mutex              (acquired ONLY lock-free; held across callbacks BY DESIGN)
  ├─> dispatch_owner mutex       (leaf; set/clear/read only)
  ├─> events.queue mutex         (pop; leaf)
  ├─> subscribers RwLock         (snapshot only; released before invoke)
  ├─> legacy slot RwLock         (clone-then-invoke, as today connection.rs:160-169)
  └─> SUBSCRIBER CALLBACK        (runs with gate held but ZERO other hub/manager locks;
                                  may take connections shards, slot mutexes, pg state mutex)
```

- `dispatch()` is only entered lock-free (`connection_down` and `register_connection` call it
  after entry-guard release; reentrant same-thread calls return on the owner check). Threads
  holding shard/slot/pg locks never call `dispatch()`, so "gate holder waits on shard/slot/pg"
  can never cycle back into "shard/slot/pg holder waits on gate".
- Callbacks may take connections shard locks (`get_connection`, `connected_nodes`,
  `connected_peers`, `last_peer_generation`), scheduler slot locks (noconnection delivery), and
  pg's state mutex (purge) with no cycle back into the hub.
- `mark_down`/`disconnect_node` from inside a callback is safe: `connection_down` acquires the
  entry guard fresh (the callback holds none), enqueues, and its nested `dispatch()` returns on
  the owner check; the outer drain delivers after the current callback returns. `connect_node` is
  async — never `block_on` it inside a callback (documented; spawn instead).
- **Cross-manager rule** (documented in the contract): a callback must not synchronously drive
  ANOTHER `ConnectionManager`'s lifecycle (`disconnect_node` on manager B from inside manager A's
  callback) — two managers cross-tearing simultaneously is a classic ABBA on the two gates. Defer
  cross-manager actions to another task. No in-tree subscriber does this; multi-scheduler tests
  keep their subscribers manager-local.

---

## 6. Delivery and ordering contract (named invariants — ship verbatim, minus citations, as `connection_events` module rustdoc)

- **INV-ALTERNATION** — per node, delivered events form `Up(g1) Down(g1) Up(g2) …` with strictly
  increasing generations; for one generation, Up(g) delivery completes before Down(g) delivery
  begins (queue order is table-transition order). "Peer bounced" = `Down(gn)` then `Up(gn+1)`;
  compare `peer_creation` across the Ups to distinguish peer restart (changed) from link blip
  (same). "Peer gone" = `Down(gn)` with no subsequent Up.
- **INV-EXACTLY-ONCE** — exactly one Up per generation; exactly one Down per generation delivered
  while the manager lives, EXCEPT a session still live at manager teardown, which gets no Down
  (parity with the legacy hook: manager drop never synthesized events). Socket displacement during
  simultaneous connect is invisible (same session; generation inherited).
- **INV-TOTAL-ORDER** — all subscribers observe all events in one global sequence (single-drainer
  queue). Within one event: subscribers in registration order, then the legacy down-slot last
  (Down only, 0.11 `ConnectionDownEvent{node, reason}` shape).
- **INV-SYNC** — delivery is synchronous with the transition: when the call that caused a
  transition returns (`disconnect_node`, the `mark_down` inside a read/heartbeat/drain task,
  `register_connection` via `connect`/accept), every event it produced has been delivered to every
  subscriber — by the calling thread, or by a concurrent dispatcher the call waited on. Sole
  exception: a transition triggered from INSIDE a callback is delivered after that callback
  returns, on the same thread, before the outermost dispatch returns. pg-purge therefore remains
  strictly synchronous-on-socket-drop (C12). The delivering thread is unspecified
  (read-loop/heartbeat/drain tasks on the single-worker `beamr-dist-send` runtime, sender.rs:149;
  or the `disconnect_node` caller's thread, connection.rs:771-778).
- **INV-UP-VISIBILITY** — `Up(g)` is enqueued after the generation-g connection is installed in
  the table; `get_connection(node)` inside the Up callback returns a connection with
  `generation() >= g` (== g unless the link already bounced again, in which case `Down(g)` follows
  in event order). This is what lets liminal replace its 250ms join-backfill poll.
- **INV-DOWN-VISIBILITY** — `get_connection(node)` inside a `Down(g)` callback never returns the
  generation-g connection (removal completes before the enqueueing guard releases; a concurrent
  dispatcher's lookup blocks on the shard lock until it does — §3.4 pinned dependency).
- **INV-FRAME-ORDER** — no inbound frame from the generation-g socket reaches the control-frame
  handler before `Up(g)` delivery completes: the read loop is spawned only after
  `register_connection`'s dispatch returns (§3.3). (No ordering is promised between a Down and the
  last frames of the closing generation — a dying socket's final frames may be processed after its
  Down, as today.)
- **INV-SUB-DISCIPLINE** — callbacks MUST NOT block, MUST NOT perform socket I/O, and MUST capture
  only `Weak` handles to scheduler state (C7/C9). A blocked callback stalls reads, writes,
  heartbeats, accepts AND concurrent transition callers for EVERY peer. Brief bounded acquisition
  of short-hold locks (process slot mutexes, as the scheduler's own subscriber does) is
  acceptable. Callbacks MAY re-enter any non-async `ConnectionManager` method on the SAME manager,
  including subscribe/unsubscribe and `disconnect_node` (delivered after the current callback,
  §5); they must NOT synchronously drive another manager's lifecycle (§5 cross-manager rule) and
  must not `block_on` async methods.
- **INV-SCHED-FIRST** — the scheduler registers its composed subscriber at construction, before any
  embedder can subscribe; within it, pg-purge strictly precedes noconnection delivery. Embedder
  subscribers, the legacy slot, and any trap-exit process receiving `{'EXIT', _, noconnection}`
  therefore always observe post-purge pg state.
- **INV-NO-REPLAY** — no replay for late subscribers; reconcile via subscribe-then-
  `connected_peers()`, max-by-generation per peer.

---

## 7. Hazards

- **H1 — stale purge vs. fresh inbound joins: CLOSED structurally.** Today there is a window
  between `remove_if` and hook `invoke` (connection.rs:502-508) in which a re-dialed generation's
  read loop can apply fresh PG_UPDATE joins that the stale purge then wipes. Under this spec the
  race is impossible: `Down(g)` is queued before `Up(g+1)` (entry-guard order), `dispatch()` is
  blocking, and generation g+1's read loop spawns only after its installer's dispatch returned —
  i.e. after the purge for g already ran (§3.3, §3.4). Generation-guarding the purge (skip when a
  newer generation is installed) is therefore unnecessary AND was independently rejected: for a
  genuinely bounced peer it would leave the old incarnation's members resident with no cleanup
  path. Pinned by test (§8).
- **H2 — blocking subscriber wedges all distribution; gate adds bounded cross-thread waits.**
  Inline delivery on the single-worker dist runtime (sender.rs:146-152) means one blocking
  callback stalls every peer — inherent to today's hook too. NEW under the blocking gate: a
  transition caller on thread B waits for thread A's in-flight subscriber chain (and vice versa) —
  bounded by INV-SUB-DISCIPLINE, occurs only when two transitions overlap on different threads
  (rare: transitions are connect/disconnect/node-death). This is the deliberate price of INV-SYNC
  (C12); the non-blocking baton escape hatch is recorded (§10 deferred) should a real cluster show
  dist-runtime stalls.
- **H3 — Arc cycle.** The hub lives in `ConnectionManagerInner`, owned by `SharedState`; any stored
  closure strongly capturing `SharedState` (or the `Scheduler`) leaks forever. The built-in
  captures `Weak` (mirror scheduler/mod.rs:861-870, supervision_integration.rs:77-93); rustdoc
  repeats the rule for embedders (C9).
- **H4 — shard-lock reliance.** INV-DOWN-VISIBILITY depends on dashmap `get` blocking on a shard
  write-locked by `entry` (§3.4). Pinned by test (§8, "down-visibility under concurrent dispatch")
  so a dashmap major bump that changes shard semantics fails loudly.
- **H5 — manager teardown emits nothing.** Sessions live at drop get no Down (INV-EXACTLY-ONCE
  caveat). Consumers needing teardown signals hold the Scheduler and know when they drop it. Parity.
- **H6 — displaced-socket buffer loss (parity).** On simultaneous-connect displacement, frames in
  the displaced socket's kernel buffer are dropped with no event, and no Up fires (same session) —
  so an Up-triggered backfill does NOT re-run. Identical to today (no hook fired either, §0); pg
  correctness relies on the canonical socket having carried joins, and work item A's delivery
  contract (node-down backstop) covers controls. Documented in the contract rustdoc.
- **H7 — legacy replace-on-register retained.** Two 0.11-style embedder components registering via
  `register_connection_down` still evict each other (unchanged semantics, deliberately). The R2
  hazard is discharged only for beamr internals; rustdoc steers embedders to
  `subscribe_connection_events`.
- **H8 — reason-atom skew.** 0.11.0 lacks `HeartbeatTimeout`; the legacy event may carry a variant
  a 0.11-built consumer never matched. Pre-existing (additive per package); noted in CHANGELOG.
- **H9 — cross-manager ABBA.** Two managers whose subscribers synchronously tear each other down
  can deadlock on the two gates. Forbidden by contract (§5, INV-SUB-DISCIPLINE); no in-tree
  subscriber crosses managers. Doc-only (no mechanism), revisit if an embedder needs it.

---

## 8. Test plan

Unit: `src/distribution/connection_events_tests.rs` (hub/dispatch/generations) and
`src/scheduler/connection_lifecycle_tests.rs` (composed subscriber; reuses the promoted
supervision_tests fixtures §4, plus `register_test_connection` connection.rs:1011-1024 for
real-socket firing — build a runtime + `set_runtime_handle` per connection.rs:2271 precedent).
E2E: `tests/connection_events_e2e.rs`, `#![cfg(feature = "net")]`
(distribution_mesh_handshake.rs:54), copying the pg_distribution_e2e harness shape (in-process
Schedulers, `DynamicResolver` :26, loopback TCP, `eventually` :115) with the HS-5 60s watchdog on
every multi-node test (distribution_mesh_handshake.rs:93-108). Gate per commit: `cargo fmt --check`,
`cargo check`, `cargo test -p beamr` (lib + integration) default AND `--all-features`,
`cargo clippy --all-targets -D warnings`, zero new `#[allow]`.

Package scenarios:

| # | Scenario | Disposition |
|---|---|---|
| 1–3 | wire EXIT semantics | **Work item A** (needs encoders/decoders). B un-deadens the appliers A will call; no rework. |
| 4 | Node death → noconnection via the REAL hook | **Discharged.** `connection_lifecycle_tests`: `make_shared_state` + `register_scheduler_connection_subscriber`; `register_test_connection` to a loopback peer socket; `add_remote_link` a trapping proc, a non-trapping proc, and a proc linked to a different node; drop the peer socket → read-loop EOF → `mark_down(PeerClosed)` (connection.rs:1057-1059) → hub → composed subscriber. Assert the supervision_tests.rs:925-964 outcomes (trapping gets `{'EXIT', ExternalPid, noconnection}`, non-trapping dies, other-node proc alive) — now fired by the real hook. The direct-call test at :925 is KEPT (applier-level contract). |
| 5 | Trap handler never sees dead node's pg members | **Discharged, two layers.** (a) Structural: same harness + `pg_registry.apply_remote_join` seeding a member for the peer node; a probe subscriber registered AFTER the scheduler's asserts, inside its own Down callback, `remote_members` empty AND the noconnection tuple already in the trap target's mailbox. Deterministic — purge and delivery are sequential statements. (b) Hub-level: recorder subscribers pin registration-order invocation. |
| 6 | No double-fire | **B half discharged.** (i) Exactly-once-per-generation stress: N threads race `mark_down` (mixed reasons) against re-dial `register_connection` installs for one node atom over M iterations; a counting subscriber asserts per generation exactly one Down, Up(g) before Down(g), Down(g) before Up(g+1), strict monotonicity. (ii) Redelivery idempotence at the subscriber seam: drive `handle_connection_event(Down)` twice for one node → exactly one noconnection EXIT (the applier removes the link at supervision_integration.rs:293/:310). Wire-EXIT-vs-noconnection race is **A**, on this substrate. |
| 7 | Multi-subscriber; pg e2e unmodified | **Discharged.** Hub units: two subscribers fire in registration order; unsubscribe true-then-false and stops delivery; subscribe/unsubscribe/`disconnect_node` from inside a callback (reentrancy: delivered after the current callback, before the outermost dispatch returns, no deadlock); legacy slot fires LAST with `{node, reason}` shape; **R2 regression**: `register_connection_down` after scheduler construction still fires AND both internal effects (purge + noconnection) still happen. E2E: `pg_join_visible_on_peer_and_purged_on_node_down` (pg_distribution_e2e.rs:129) and `reconnection_down_up_rejoin_reestablishes_membership_without_stale_resurrection` (:469) pass **UNMODIFIED** — hard gate — plus a variant with an extra embedder-style subscriber attached proving pg semantics unchanged in its presence. |
| 8 | EXIT to Executing target | Wire arm is **A**; B pins the noconnection variants now: target forced Executing via `make_executing` (supervision_tests.rs:129); trapping ⇒ `pending_exit_messages` gains `(PendingExitSource::Remote(..), NoConnection)` drained at store-back (execution/core.rs:355-372; parity with `monitor_down_for_executing_watcher_is_delivered_on_store_back`, supervision_tests.rs:607); non-trapping ⇒ tombstone observed at store-back → full cleanup (execution/core.rs:69-73, :450-458) — the **D9-ruling test**. |
| 9 | Hostile/unknown frames | **Work item A** (decode path; B adds no decode surface). |
| 10 | Watchdog discipline | **Adopted**: 60s watchdog on every multi-node e2e; bounded `eventually` polling elsewhere. |

B-specific additions (judge-demanded and mechanism-pinning):
- **INV-SYNC contention:** thread A dispatches with a deliberately slow test subscriber; thread B
  calls `disconnect_node` for another peer concurrently; assert that when B's call returns, B's
  purge/noconnection effects are already observable (no `eventually`). Pins the blocking gate.
- **INV-FRAME-ORDER:** recorder log shared by an Up subscriber and the control-frame handler; peer
  writes a data frame immediately after install; assert log order is `[up, frame]` — the read loop
  cannot start before Up delivery completes.
- **H1-closure regression:** peer with generation g down while its re-dial installs g+1 and the
  peer immediately sends PG joins on g+1; assert final pg state contains the g+1 joins (purge for
  g provably ran before g+1's read loop existed).
- **Panic containment (test-only):** a panicking subscriber poisons the gate; subsequent events
  are still delivered (poisoned-lock recovery + RAII owner reset).
- **Generation monotonicity across reconnect (e2e):** connect A↔B → `Up(g1)`; `disconnect_node` →
  `Down(g1, ManualDisconnect)` delivered before the call returns; reconnect → `Up(g2)`, `g2 > g1`;
  `peer_creation` identical across the blip. Peer-bounce variant: restart B's scheduler with a new
  creation → `Up(g2)` carries the new `peer_creation`.
- **HS-4 regression (the closed hole):** hold a down-but-unreaped incumbent (the
  connection.rs:2040-2047 technique: flip `down` without reaping), re-dial → recorder sees
  `Down(g_old)` then `Up(g_new)`, in order, exactly once each (reason = recorded `down_reason`, or
  the documented `ReadError` fallback when the test flipped the flag directly).
- **Displacement inherits generation:** simultaneous-connect rig → exactly one Up per pair, zero
  events on displacement, survivor's `generation()` == the Up's.
- **INV-UP-VISIBILITY:** Up callback calls `get_connection(node)` → `Some`, correct
  generation/peer_creation; writes a pg backfill via `write_raw`; peer observes it (liminal
  poll-deletion proof). (The write is deferred to a spawned task per INV-SUB-DISCIPLINE — the
  callback itself must not do socket I/O; the test asserts the handle obtained inside the callback
  is usable.)
- **INV-DOWN-VISIBILITY under concurrent dispatch** (pins H4): subscriber on thread T2 checks
  `get_connection(node)` when handling `Down(g)` while T1 performs the removal — must never observe
  the gen-g connection.
- **connected_peers/last_peer_generation:** snapshot excludes down links; recipe test — subscribe,
  snapshot, reconcile by max generation, no session double-counted.
- **Legacy bridge unmodified:** connection.rs:1620 (ManualDisconnect once), :1652 (write error
  once), :2422 (HeartbeatTimeout) pass as-is.

Note: `remote_link_exit_sends_exit_control` (supervision_tests.rs:876, asserts
`control_router.messages()`) is **A's** obligation to rewrite; B leaves it untouched.

---

## 9. Migration / landing plan (one branch; land BEFORE work item A per R2; every commit passes the full gate bar)

1. **`refactor(distribution): extract connection_events module`** — create
   `connection_events.rs`; move `ConnectionDownReason`/`ConnectionDownEvent`/`ConnectionDownHook`
   verbatim (`invoke` → `pub(crate)`); fix the HeartbeatTimeout doc-lie in passing; add the
   `pub use` re-export in connection.rs and `pub mod connection_events;` in distribution/mod.rs.
   Zero behavior change; all existing tests pass unmodified.
2. **`feat(distribution): connection-event hub with generation-tagged up/down`** — hub + queue +
   blocking-gate dispatch (§2.4); `ConnectionGeneration`/`NodeUp`/`NodeDown`/`ConnectionEvent`/
   `SubscriberId`; `DistConnection::{generation, peer_creation, down_reason}` + accessors;
   `mark_down` one-liner; `ConnectionManagerInner.connection_down_hook` → `events`; rewritten
   `connection_down` and `register_connection` arms (dispatch-before-spawn); `peer_creation`
   plumbed at connect/:908, handle_accepted/:1188, test helper 0; four new `ConnectionManager`
   methods; legacy delegation; `connection_events_tests.rs` (hub units, INV-SYNC contention,
   INV-FRAME-ORDER, exactly-once stress, HS-4 regression, visibility, legacy-last, R2 regression,
   panic containment); **same commit**: `scheduler/connection_lifecycle.rs` with the composed
   subscriber (pg-purge wired; noconnection seam comment), replacing scheduler/mod.rs:871-880.
   Both pg e2e tests green unmodified. (Hub + pg migration atomically — R2's mitigation.)
3. **`feat(scheduler): wire noconnection delivery to the connection-event hub`** — add the
   `supervision_integration::connection_down` call to `handle_connection_event`; delete both
   `#[allow(dead_code)]` + false comments (supervision_integration.rs:277, :326); promote the
   supervision_tests fixtures; `connection_lifecycle_tests.rs` (scenarios 4, 5, 6ii, 8 variants;
   D9-ruling test).
4. **`test(distribution): connection-events e2e`** — `tests/connection_events_e2e.rs`
   (generation monotonicity, peer-bounce vs blip, H1-closure, multi-subscriber with
   embedder-style probe, watchdogged).
5. **`docs+release`** — `docs/adr/012-connection-events-hub.md` (extends ADR-009: the seam stays
   in core and becomes multi-subscriber; single slot retained as compat surface; records the
   INV-SYNC mechanism ruling, the H1 closure, and the GlobalNameRegistry evidence); CHANGELOG;
   version → **0.13.0** (additive feature release from 0.12.1; liminal stays pinned 0.11.0,
   unaffected — liminal Cargo.toml:30 per package). Consumer notes: liminal (on bump:
   `subscribe_connection_events`, Up-triggered backfill, delete the 250ms poll — its only
   functional job per package ground truth); Apollo/haematite (key retry state by
   `(node, generation)`; `peer_creation` is the bounce discriminator; Down is exactly-once at the
   source, so any consumer-side redelivery dedupes on the generation key); frame (conn-down hook
   Wave-0: subscribe + INV contract).

---

## 10. Constraints: discharged vs deferred; risks

| Item | Status |
|---|---|
| C1, C2, C3 | **Relied on, unmodified** — delivery reuses the verified appliers as-is (R4's mitigation). |
| C4, C5, C6, C8, C11 | **Deferred to A** — no codec/queue/wire surface touched here. |
| C7 | **Discharged** — INV-SUB-DISCIPLINE; built-ins verified non-blocking (§0 R3 ruling); dispatch-before-spawn delay bounded and argued (§3.3); H2 records the gate-wait cost and escape hatch. |
| C9 | **Discharged** — Weak capture in the built-in; rustdoc rule; H3. |
| C10 | **Discharged and strengthened** — funnel untouched; entry-guard emission upgrades at-most-once (with the HS-4 loss hole) to exactly-once-per-generation (§3.4). |
| C12 | **Discharged STRICTLY** — purge body unchanged (pg.rs:287-292), first in the composed subscriber, and INV-SYNC guarantees the socket-drop caller does not return before the purge ran; both pg e2e tests unmodified. No contention caveat remains (the blocking gate removed it). |
| C13 | **Discharged** — §3.5 inventory; re-exported paths; no field on `ConnectionDownEvent`; replace semantics kept. |
| C14 | **Discharged** — new logic in new files; both over-limit files net-shrink; poisoned-lock idiom; no unwrap/expect/panic; two `#[allow(dead_code)]` removed, zero added; e2e net-gated; gate incl. `--all-features`. |
| R1, R4, R5, R6 | A-scope; B builds nothing on the orphaned modules and adds no decode arms. |
| R2 | **Discharged** — pg-purge off the replaceable slot; hub + migration in one commit; regression test. |
| R3 | **Discharged by argument + contract** (§0 ruling, H2). |
| D9 | **Resolved with evidence** (§0), pinned by test. |

**Explicit deferred list:** wire LINK/EXIT/MONITOR encode/decode + must-deliver control lane +
dead-node queue cleanup + ControlRouter retirement (A); `global::remove_node` wiring (blocked on a
production `GlobalNameRegistry` — evidence §0/§4); `direction`/`peer_addr` fields on `NodeUp`
(additive behind `#[non_exhaustive]`; deferred because socket identity can change mid-session
under the session-generation model — §0); priority tiers / `subscribe_with_priority` (no
consumer); **non-blocking baton dispatch** as the escape hatch if a real cluster shows
dist-runtime stalls from gate waits (one-function swap in §2.4, at the cost of INV-SYNC weakening
to delivered-before-some-dispatch-returns); generation-guarded pg purge (unnecessary — H1 closed
structurally; independently rejected); `#[deprecated]` on `register_connection_down` (release
decision, blocked on the `-D warnings` allow-bypass rule); formal event replay for late
subscribers (INV-NO-REPLAY + snapshot recipe instead); cross-manager reentrancy support (H9,
doc-forbidden until an embedder needs it).
