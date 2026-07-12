# Readiness Registration API — Composition Commit 6

**Status:** DESIGN OF RECORD — **both pair halves signed.** Vesper Lynd's half
signed with the §3.5-inherits-§4.6 condition (folded @ 3f42cf1); Waffles the
Terrible's half signed 2026-07-12 APPROVE WITH THREE CONDITIONS W1–W3, all
three folded in this revision (W1 → §4.6 poisoned-lock posture + gate panics
under the table lock; W2 → §6 fifth rider row + §3.5 sweep-after-drain order +
§3.3 straggler-refusal assertion; W3 → §3.2 validate-copy-unlock-deliver,
author's choice of the structural wall). Commit 6 builds against THIS revision.
Realizes — does not re-litigate — the certified
`READINESS-CONTRACT-SPEC` §3 shape (b) and `EMBEDDER-COMPOSITION-SPEC` §3.9.
**Scope:** the readiness service as the composition model's first born-composed
service (composition spec §3.9), landing the §3.9 Shared-delivery gate and the
cross-scheduler routing mechanism **as one reviewable unit** (§3.9 formal
constraint). No code lands with this doc.
**Base:** beamr `readiness-registration-api` @ 675ee5e (composition commits 1–5).
**Author:** Artemis Peach (beamr).

**Standing obligations on the commit-6 build (recorded before any code
exists — Waffles' riders, 2026-07-12):**

1. **Strict design-implementation.** The build leg (Vesper Lynd's hands,
   Artemis unreachable) implements THIS revision verbatim. Any point where
   the code wants to contradict this doc — including improvements — STOPS
   and returns for a pair round instead of being improvised. The doc is
   Artemis's voice at the table; drift silences it.
2. **Deferred domain-owner review.** When contact with Artemis restores,
   she receives a retroactive domain-owner review pass over the landed
   diff — this entry is that obligation's record, standing until her pass
   is recorded here with its outcome. The merge commit must note both her
   design authorship and her absence at build time.

Citations are `file:line` into the landed tree at this head. Where the contract
spec's own citations have drifted against landed code, the drift is recorded in
§7, not silently corrected.

---

## 0. What this commit lands, and the one-unit constraint

The certifying pair's §3.9 constraint is formal: **the §3.9 delivery gate and
the cross-scheduler routing mechanism are one reviewable unit.** A `Shared`
readiness service is not a poll thread plus a later routing patch — the routing
identity (§3), the delivery gate (§3.4), and the §4-step-3 shutdown sweep (§3.5)
are the same identity machinery seen from three ends and land together. This is
the departure from the IO rings, whose `Shared` arm was *deferred* to this
commit with a typed refusal (`WithServicesError::SharedRingRoutingDeferred`,
services.rs:196); readiness is the service that brings `Shared` to life and
supplies the routing mechanism the rings will later reuse.

This commit adds, per the composition model:

- a `ServiceMode<ReadinessService>` payload (§1.1), Disabled by default
  everywhere except profiles that request it (composition spec §3.9);
- **all three `SchedulerServices` arms** — `Disabled`, `Owned` (one poll thread
  per scheduler), `Shared` (one poll thread per process) (§1.2);
- the slice-reachable registration / rearm / ACK'd-dereg surface (§1.4);
- the `beamr-readiness` inventory line and its lens answers (§5);
- the §4-step-3 Shared-registration shutdown sweep (§3.5);
- the commit-5 teardown riders: teardown-admission gates on the remaining
  mutating facility ops (§6).

---

## 1. Public Rust API surface

Naming and doc register follow the landed `service.rs` / `services.rs` /
`ring_service.rs` conventions exactly. New source lives in
`crates/beamr/src/scheduler/readiness_service.rs` (sibling of `ring_service.rs`
and `distribution_service.rs`), gated behind a new `readiness` cargo feature
(contract §3.2; itself requiring `threads`).

### 1.1 `ReadinessService` — the `ServiceMode` payload

A concrete, identity-bearing newtype exactly as `RingService`
(ring_service.rs:23) and `DistributionService` (distribution_service.rs:26) are:
its `T` carries the one poll thread, the registration table, the poll-set fd,
and the wakeup handle, and it implements `ServiceIdentity` +`ShutdownService`
(service.rs:52, service.rs:61) so `ServiceMode` surfaces its id and the owner
drives its teardown.

```rust
/// The readiness service carried in a `ServiceMode` (contract §3; spec §3.9).
///
/// Wraps ONE poll thread (kqueue via mio on macOS, epoll on Linux) plus the
/// registration table it delivers from. `Owned` ⇒ one thread for the owning
/// scheduler; a `Shared(Arc<ReadinessService>)` handle cloned into N schedulers
/// is ONE thread for the process (spec §3.9), each consumer routing its own
/// markers home via the route-home identity stamped at registration (§3).
pub(super) struct ReadinessService {
    core: Arc<ReadinessCore>,      // poll thread + table + mio::Poll + Waker
    instance: ServiceInstanceId,   // minted once; propagates through Arc clone
}

impl ReadinessService {
    /// Construct an Owned service and spawn its poll thread, or fail loudly
    /// (EMFILE-class construction failure, §4): a service that cannot build its
    /// poll set is NOT installed, matching the refuse-don't-degrade precedent
    /// (services.rs:184-190).
    pub(super) fn build_owned(route_home: RouteHome) -> Result<Self, ReadinessBuildError>;

    /// A registration handle for a scheduler consuming this service. Owned:
    /// the owner's own handle. Shared: minted per consuming scheduler so each
    /// stamps its own route-home (§3).
    pub(super) fn consumer(&self, route_home: RouteHome) -> ReadinessConsumer;

    pub(super) fn poll_thread_names(&self) -> Vec<String>;   // §5 `actual`
    pub(super) fn poll_fd_classes(&self) -> Vec<&'static str>; // §5 `fd_classes`
    pub(super) fn live_registration_count(&self) -> usize;    // §5 idle-cost lens
}

impl ServiceIdentity for ReadinessService { /* returns self.instance */ }
impl ShutdownService for ReadinessService {
    /// Owner-only teardown (spec §4): stop accepting registrations, signal the
    /// Waker, JOIN the poll thread. After this returns no poll thread of this
    /// service exists at the OS level, so nothing can enqueue a marker into a
    /// torn-down process table (contract §3.3 hard rule). Idempotent.
    fn shutdown(&self);
}
```

`SharedReadiness` is the injectable handle the embedder builds once and clones
into each scheduler — the analogue of `SharedIoRing` (services.rs:137) but,
unlike it, carrying a **live** service (the shared arm is real this commit, not
refused):

```rust
/// Process-wide shared readiness service (spec §3.9): built ONCE, injected into
/// every scheduler that consumes it. Cloning propagates the one
/// `ServiceInstanceId`, so the §5 inventory dedups N `Shared` entries to one
/// thread (service.rs:46-55).
#[derive(Clone)]
pub struct SharedReadiness(Arc<ReadinessService>);

impl SharedReadiness {
    /// Build the one process-wide poll thread. `Err` on EMFILE-class failure
    /// (§4) — the embedder learns the service is unavailable before injecting a
    /// half-built poller into any scheduler.
    pub fn new() -> Result<Self, ReadinessBuildError>;
}
```

### 1.2 `SchedulerServices` arms

A per-service choice mirroring `DirtyChoice` (services.rs:48) and the
`ServiceMode` arms, plus a new `SchedulerServices` field and its builder
methods. Readiness has **no legacy `SchedulerConfig` knob** (it did not exist
before this commit), so `FromConfig` resolves to `Disabled` — the composition
spec §3.9 "Disabled by default everywhere except profiles that request it."

```rust
/// Composition choice for the readiness service (spec §3.9). Unlike the IO
/// rings, the `Shared` arm is LIVE this release: it carries the process-wide
/// poll thread and its cross-scheduler routing (§3), the commit-6 unit.
#[derive(Clone)]
pub(super) enum ReadinessChoice {
    /// No readiness knob exists on `SchedulerConfig`, so this resolves to
    /// `Disabled` (spec §3.9 default-off).
    FromConfig,
    /// Zero threads, zero fds; a readiness register is refused before any
    /// suspension side effect with `ReadinessError::Disabled` (contract §3.2).
    Disabled,
    /// An owned poll thread for THIS scheduler (spec §3.9 Owned).
    Owned,
    /// An injected process-wide poll thread the embedder owns and joins
    /// (spec §3.9 Shared). This scheduler routes its markers home but never
    /// stops the thread (service.rs shutdown_owned semantics).
    Shared(SharedReadiness),
}
```

`SchedulerServices` (services.rs:240) gains one field, `readiness:
ReadinessChoice`, defaulted to `FromConfig` in `from_config()` (⇒ Disabled) and
`Disabled` explicitly in `minimal()` (services.rs:285). **`full_runtime()`
picks `Owned`** — a single standalone scheduler's simple default (spec §3.9);
the aion-shape multi-scheduler profile injects `Shared` (spec §3.9: "multi-
scheduler embedders get Shared in their profile; the full-runtime profile
documents which it picks"). Builder methods follow the services.rs pattern
verbatim:

```rust
pub fn disable_readiness(mut self) -> Self;                 // ReadinessChoice::Disabled
pub fn owned_readiness(mut self) -> Self;                   // ReadinessChoice::Owned
pub fn shared_readiness(mut self, svc: SharedReadiness) -> Self; // Shared
```

Unlike the shared rings, `shared_readiness` is **not** refused by
`SchedulerServices::validate()` (services.rs:428) — the routing mechanism it
needs is exactly what this commit lands. `validate()` gains no readiness arm.

### 1.3 `Interest`, `ReadinessToken`

`Interest` is beamr-owned (a bitflag newtype), NOT a re-export of `mio::Interest`
— the mio backend stays an implementation detail behind the existing lock
(out of scope, §9), so it never appears in the public signature:

```rust
/// Readiness directions a registration arms (contract §3.1).
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Interest(u8);
impl Interest {
    pub const READABLE: Self;
    pub const WRITABLE: Self;
    pub fn both() -> Self;                    // READABLE | WRITABLE
    pub fn is_readable(self) -> bool;
    pub fn is_writable(self) -> bool;
}
```

The token is the internal registration identity (contract §3.1: "the atom
marker is the public contract; the token is the internal registration
identity"). `Copy` so the consumer keeps it in connection state (liminal R5) and
still passes it by value to dereg:

```rust
/// Opaque registration identity with a generation MINTED BY REGISTRATION
/// (contract §3.1/§3.4; never caller-supplied). A stale generation's events are
/// discarded in the service, never delivered (§3.4).
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct ReadinessToken {
    slot: SlotIndex,       // table slot (a reused fd may reuse a slot)
    generation: Generation, // per-slot monotonic; the fd-reuse guard (§4)
}
```

`Generation` is a `u64` minted by a per-slot counter inside the table, bumped on
every (re)use of a slot — the same mint-and-carry discipline
`ServiceInstanceId::mint()` uses for services (service.rs:35), but per
registration slot rather than per service.

**The event→generation binding (first-consumer finding 2), owned here because
§4.1's structural guarantee stands on it.** A `mio::Token` is one `usize`; the
encoding is `token = slot(32 bits) | generation_low(32 bits)` — the low 32 bits
of the record's u64 generation at ARM time. Validation at delivery, under the
table lock: the slot's record must exist, its FULL current generation's low
bits must equal the event token's `generation_low`, and its armed bit must be
set. Truncation posture, stated: a false 32-bit match requires the same slot
to be re-armed 2^32 times while ONE kernel event sits buffered between the
poll thread's `poll()` return and its lock acquisition — a window inside a
single poll iteration, so the wrap is physically unreachable; and even a
hypothetical false match delivers to the slot's CURRENT record — the correct,
live pid of the newest registration — which is a spurious durable-marker wake
the contract already tolerates by design (C2/C4: wake-without-cause is
harmless; the consumer probes and re-parks). Two independent walls; the
guarantee needs only one. **Examined and ACCEPTED by both halves of the pair
(Waffles 2026-07-12: wall (a) decisive alone; wall (b)'s current-record
delivery is the C2/C4-tolerated wake, repaired by the consumer's own rearm
loop).**

### 1.4 Registration / rearm / dereg — the embedder-visible surface

Two consumer-visible surfaces, split by calling context (first-consumer
finding 1):

- **In-slice** (register + rearm, and ONLY these): a facility trait reached
  through the executing process's `ProcessContext`, following the landed
  facility pattern (`context.io_message_facility()` etc., native/context —
  wired by `build_native_services`, supervision_integration.rs). Usable inside
  a NativeHandler slice with no cross-scheduler blocking RPC (Hermes point 4;
  contract C4). The calling scheduler's `SharedState` fills in its own
  route-home automatically — the consumer supplies only
  `(fd, interest, pid, marker)`.
- **Host-side** (deregister, and ONLY it): a public method on `Scheduler`,
  callable from the supervisor/reaper's record-removal path — never required
  to run on the parked pid or inside any slice (Hermes point 3). v1 posture:
  dereg is host-side only; rearm is in-slice only (the C4 drain→rearm→probe→
  Wait loop is a slice loop); register is in-slice (the arming site is the
  connection's own slice).

```rust
/// In-slice facility (native/readiness.rs), set on ProcessContext by
/// build_native_services iff the scheduler's readiness slot is not Disabled.
pub trait ReadinessFacility: Send + Sync {
    /// Contract §3.1 registration; semantics identical to
    /// SharedState::readiness_register below.
    fn register(
        &self,
        fd: RawFd,
        interest: Interest,
        pid: u64,
        marker: Atom,
    ) -> Result<ReadinessToken, ReadinessError>;
    /// One-shot re-arm (contract §3.1); in-slice only.
    fn rearm(&self, token: &ReadinessToken, interest: Interest)
        -> Result<(), ReadinessError>;
}

impl ProcessContext {
    /// None when the scheduler composed readiness Disabled — the typed-absence
    /// pattern every facility uses (the caller refuses without suspension).
    pub fn readiness_facility(&self) -> Option<&dyn ReadinessFacility>;
}

impl Scheduler {
    /// Host-side ACK'd deregister (contract §3.1/§3.4): supervisor/reaper
    /// surface, bounded, dead-pid-safe (C3). See §4.2/§4.6 for the bound and
    /// the failed-thread posture.
    pub fn readiness_deregister(&self, token: ReadinessToken);
}
```

These delegate to the internal `SharedState` seam below (shown for
implementers; visibility `pub(in crate::scheduler)` — the trait and `Scheduler`
method above are the API consumers name).

```rust
impl SharedState {  // reached via ProcessContext in a slice
    /// Register `fd` for `interest`; on readiness, deliver durable `marker` to
    /// `pid` via the same machinery as `enqueue_atom_message` (mod.rs:1946),
    /// inheriting C1–C3 verbatim (contract §3.1). Returns a token whose
    /// generation the service minted. `Disabled` ⇒ `ReadinessError::Disabled`,
    /// refused BEFORE any side effect (contract §3.2), leaving the caller
    /// RUNNABLE (Hermes point 4 / C4). Non-blocking: a table-lock + one mio
    /// `Registry::register` syscall, no wait on the poll thread.
    pub(in crate::scheduler) fn readiness_register(
        &self,
        fd: RawFd,
        interest: Interest,
        pid: u64,
        marker: Atom,
    ) -> Result<ReadinessToken, ReadinessError>;

    /// Re-arm one-shot interest (contract §3.1 one-shot). Idempotent-safe on a
    /// token whose registration was already reaped (`UnknownToken`, harmless).
    pub(in crate::scheduler) fn readiness_rearm(
        &self,
        token: &ReadinessToken,
        interest: Interest,
    ) -> Result<(), ReadinessError>;

    /// ACK'd deregister (contract §3.1/§3.4). Consumes the token. Callable from
    /// a context that is NOT the parked pid (the supervisor/reaper — Hermes
    /// point 3), bounded, safe if `pid` is already dead (C3). Returns only once
    /// the registration can deliver no marker attributable to a DIFFERENT (new)
    /// registration; see §4 for the epoch handshake and its close-before-reuse
    /// guarantee.
    pub(in crate::scheduler) fn readiness_deregister(&self, token: ReadinessToken);
}
```

The `Atom` here is `crate::atom::Atom` (atom/table.rs:11), `Copy`. `RawFd` is
`std::os::fd::RawFd`. The one-shot re-arm (contract §3.1) is the C4 loop:
drain-to-`WouldBlock` → re-arm → park; a triggered direction clears itself and
delivers exactly one marker until re-armed, so coalescing is trivially harmless
(R6). Registration/rearm never returns a "you are now parked-blind" state: on
any failure the caller has a typed refusal in hand and stays runnable, then
takes its C4 final non-blocking probe before `Wait` (contract C1/C4 ordering:
arm interest → final probe of consumer-owned sources → `Wait`).

### 1.5 `ReadinessError` and `ReadinessBuildError`

Following the `WithServicesError` precedent exactly (services.rs:189 —
`Clone, Debug, Eq, PartialEq`, `Display`, `std::error::Error`; typed refusal,
not silent degrade):

```rust
/// Typed refusal for the registration surface (contract §3.2 / Q-B precedent:
/// typed Rust errors at the embedder API, existing atoms preserved at the BIF
/// surface).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadinessError {
    /// The service is Disabled on this scheduler — refused before any side
    /// effect (contract §3.2; composition spec §3.9 default-off).
    Disabled,
    /// The kernel refused the fd registration (EBADF on an fd that closed
    /// underneath, or an ENOMEM/EMFILE-class limit). `errno` is the raw cause.
    Register { errno: i32 },
    /// `rearm`/`deregister` named a token the service no longer holds (its
    /// registration was reaped at pid death, §3.5, or already deregistered).
    /// Harmless — reported so the caller can drop its stale token (Hermes
    /// point 2, re-register-same-fd).
    UnknownToken,
    /// The poll thread died; the service is FAILED (§4.6). Checked on the
    /// atomic flag BEFORE the table lock (W1) — the refusal never touches
    /// the possibly-poisoned lock.
    ServiceFailed,
    /// This scheduler's teardown has closed admission (§6 fifth row, W2):
    /// `register`/`rearm` after `drain_dirty_completions` refuse rather than
    /// stamp a record the §3.5 sweep can no longer see.
    TeardownInProgress,
}

/// Construction-time failure (poll set / Waker could not be built): the service
/// is NOT installed (§4). Distinct from a per-registration `ReadinessError`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadinessBuildError { PollSetUnavailable { errno: i32 } }
```

`Scheduler::with_services` surfaces a build failure of an `Owned` readiness
service as its existing `Err(String)` (mod.rs:898-910), the same channel used
for "genuine spawn/OS failures" (services.rs:186). `SharedReadiness::new`
surfaces it as `ReadinessBuildError` to the embedder directly.

---

## 2. Semantics per contract clause

Each clause names the API element that realizes it and the pin that gates it.
The §2.5 pinning suite is **already green on main** (readiness_contract_tests.rs)
— referenced, not duplicated. New readiness pins are the §3.6 gate set plus the
§3.9 Shared-delivery gate.

| Clause | API element | Pin |
|---|---|---|
| **C1** durable markers survive every race order (contract §2.1) | delivery is exactly `deliver_term_to_mailbox` + `wake_process` — the body of `enqueue_atom_message` (mod.rs:1947-1952) — driven by the poll thread; inherits the three-phase park recheck (core.rs:95-125) | already on main: `c1_marker_to_a_parked_process_wakes_and_is_observed`, `c1_delivery_in_the_store_to_register_gap_is_observed_before_sleep`, `c1_delivery_in_the_register_to_recheck_gap_schedules_exactly_once`, `c1_delivery_while_executing_merges_at_store_back_and_wakes` (readiness_contract_tests.rs:340–443). New: the §3.6 "every C1–C4 pin passes with the service as the marker source" gate — re-runs the C1 shapes with a real fd event as the trigger. |
| **C2** observed-or-runnable, plain parks only (contract §2.2) | registration targets pids parked via `Wait`; the service NEVER targets a gated-suspension pid — `wake_process` itself refuses to wake one (`suspension_blocks_wake`, execution.rs:360) | already on main: `c2_gated_suspension_retains_marker_and_observes_at_completion` (readiness_contract_tests.rs:527). Consumer obligation (liminal parks via `Wait` only) is a liminal-side assertion, not beamr's. |
| **C3** dead-pid semantics (contract §2.3) | delivery uses the `bool` return of the enqueue; a `false` (dead pid) is the eager-reap trigger, and a true-but-never-observed marker drops with the mailbox | already on main: `c3_enqueue_to_a_dead_or_absent_pid_returns_false`, `c3_true_then_death_before_next_slice_drops_the_marker_harmlessly` (readiness_contract_tests.rs:631, 677). Registration bookkeeping tolerates true-but-never-observed by construction (the table entry is reaped at exit, §3.5, not on delivery-observed). |
| **C4** register-before-probe, probe-before-park (contract §2.4) | `readiness_rearm` is the arm step; the consumer takes its own final non-blocking probe of consumer-owned sources, then returns `Wait`. The service adds no mailbox re-probe (contract §2.4: a mailbox re-probe would be dead code) | already on main: `c4_delivery_between_arm_and_final_probe_is_seen_by_the_probe`, `c4_delivery_after_the_final_probe_is_caught_by_the_park_recheck`, `bare_wake_before_registration_is_lost_which_is_why_markers_are_durable` (readiness_contract_tests.rs:754, 823, 889). |

§3 service obligations (contract §3.1–§3.6), each realized:

- **One-shot delivery** (contract §3.1): a triggered direction clears its armed
  bit and delivers one marker until `readiness_rearm`. Pinned on BOTH backends
  (contract §3.6): a kqueue (macOS) and epoll (Linux) contract test asserting
  one-notification-per-armed-direction — never inferring backend uniformity.
- **Acknowledged deregistration** (contract §3.1): `readiness_deregister`'s
  epoch handshake (§4). Pinned by the register/close/recycle churn storm
  (contract §3.6) and the §3.9 cross-scheduler gate.
- **Generation-keyed stale drop** (contract §3.1/§3.4): the poll thread
  validates `(slot, generation, pid, interest)` against the live record under
  the table lock before enqueueing; a mismatch drops. Pinned by the
  deterministic fd-reuse recycle-storm test (contract §3.6; §4 walks the
  interleaving).
- **Delivery = `enqueue_atom_message`** (contract §3.1): no new delivery
  machinery; `SharedState::deliver_readiness_marker(pid, marker)` is the same
  two calls as mod.rs:1947-1952.
- **Poll-set lifecycle under shutdown** (contract §3.3): §3.5 here.
- **fd-reuse safety** (contract §3.4): §4 here.
- **Process-death deregistration** (contract §3.5): eager reap at
  `cleanup_exited_process` (§3.5 here; landed path core.rs:1617 → :1656 → fd
  close :1711).
- **Disabled ⇒ zero threads, typed refusal** (contract §3.2/§3.6):
  `ReadinessChoice::Disabled`, `ReadinessError::Disabled`; §5 assertion.
- **Enabled-idle zero CPU** (contract §3.6): §5 idle-cost lens; the poll thread
  blocks in `poll()` with no timeout, so a fully idle service is a sleeping
  thread — not the 5 ms tick (§9, that is a separate worker-park surface).

---

## 3. §3.9 as one unit — multiplicity, routing, delivery gate, §4-step-3

### 3.1 Multiplicity model (composition spec §3.9)

- `Owned` = one poll thread per scheduler that owns one. `ServiceMode::Owned`,
  joined by `shutdown_owned` (service.rs:150) at teardown. Inventory reports
  `mode: Owned` on that one scheduler.
- `Shared` = one poll thread per process. The embedder builds one
  `SharedReadiness` (§1.1) and `shared_readiness(handle.clone())` into each
  scheduler; every scheduler holds `ServiceMode::Shared(Arc<ReadinessService>)`
  over the one underlying `ReadinessCore`. The propagated `ServiceInstanceId`
  (service.rs:46-55) makes the §5 dedup a plain group-by: N `Shared` entries,
  one thread (`deduped_thread_aggregate`, inventory.rs:278).

### 3.2 The route-home token — what it contains, where it lives

A `Shared` poll thread serves several schedulers but must call
`enqueue_atom_message` on the **correct** one. Registration captures the
delivering scheduler's identity as a **route-home**:

```rust
/// Captured at registration; the identity that routes a marker to the scheduler
/// that armed it (spec §3.9 "registrations carry the delivering scheduler's
/// identity so markers route home").
#[derive(Clone)]
struct RouteHome {
    /// The delivering scheduler's shared state. `Weak`, so once that scheduler
    /// is dropped the upgrade FAILS and the marker is dropped — a marker never
    /// routes to a dead scheduler even if the §3.5 sweep missed it (defence in
    /// depth, spec §5 assertion 5).
    scheduler: Weak<SharedState>,
    /// Per-(scheduler, shared-service) id minted when this scheduler starts
    /// consuming the service. The sweep key (§3.5): shutdown deregisters every
    /// record stamped with this id.
    consumer: ServiceConsumerId,
}
```

`ServiceConsumerId` is a fresh `mint()`-style process-unique token (identical
discipline to `ServiceInstanceId`, service.rs:35), minted per scheduler at
`consumer()` time (§1.1). Where it lives: **the `RouteHome` is stored in each
registration record inside `ReadinessCore`'s table** (behind the poll thread's
lock), not in the consumer — so the poll thread, which only ever holds
`&ReadinessCore`, has everything it needs to (a) upgrade the `Weak` and deliver,
and (b) be swept by consumer id at that scheduler's shutdown. For an `Owned`
service the route-home is the owning scheduler's own `SharedState`; the same
mechanism, trivially one consumer.

Delivery, per poll event, is **validate-copy-unlock-deliver** (Waffles' half,
condition W3 — lock-ordering discipline stated, structural form chosen):

1. Under the table lock: look up the record by slot; require `generation` to
   match (else drop — §4) and the armed bit to be set.
2. Still under the lock: `route.scheduler.upgrade()`; `None` ⇒ the scheduler is
   gone, drop. Copy out the upgraded `Arc<SharedState>`, the `pid`, and the
   `marker`; clear the triggered direction's armed bit (one-shot; awaits
   `rearm`).
3. **Release the table lock.**
4. On the copied `Arc`, call `deliver_readiness_marker(pid, marker)`
   (= `deliver_term_to_mailbox` + `wake_process`, mod.rs:1947-1952).

Why the unlock sits before the deliver call: `deliver_readiness_marker` takes
ANOTHER scheduler's mailbox/run-queue locks. Delivering under the table lock
would make the table lock non-leaf, and any consumer-side path that ever held a
scheduler delivery lock while calling `register`/`rearm` (table lock) would be
an inversion no gate test reliably catches. Rather than resting on a
never-provable "no such path exists" discipline across every future consumer,
the delivery path releases the table lock first — the table lock is a leaf by
construction on the delivery side, and register/rearm/dereg take it as their
only lock. The one consequence is benign and verified against §3.4 and §4.6: a
marker validated pre-tombstone that delivers post-unlock goes to the OLD
current pid — the spurious durable-marker wake the contract already tolerates
(C2/C4) — never to an unrelated new registration, because the generation was
validated under the lock before the copy.

### 3.3 The delivery gate (composition spec §3.9 certification condition)

New acceptance gate, joining the readiness §3.6 set: two schedulers on one
`Shared` poll thread, markers provably delivered to the correct scheduler's
process, **including after one scheduler shuts down** — the survivor's
registrations keep routing correctly and zero markers route to the dead one.
Test shape: build one `SharedReadiness`, two schedulers A and B each with a
registered connection-shaped pid; fire A's fd and B's fd, assert each marker
lands in its own scheduler's mailbox; shut down A (its §3.5 sweep runs); fire
B's fd again, assert B still wakes; assert no delivery is attempted on A (the
sweep removed A's records; the `Weak` upgrade would also fail); and assert a
straggler `register` attempted on A's slot after A's drain closed admission is
**refused typed** (W2) — the §6 fifth-row gate tested from the same end, so
the no-leak property is gated alongside the no-delivery property. This is the
same identity machinery as §3.5, tested from the delivery end (spec §5
assertion 5: "survival isn't the bar, no-delivery-to-the-dead is").

### 3.4 fd-reuse crossing generations under the Shared thread

The generation check (§4) is per record regardless of which consumer owns it, so
a stale event for A's closed fd cannot deliver to B's new registration on a
recycled fd number even under one shared thread: the two registrations are
distinct slots with distinct generations and distinct route-homes. This is the
structural property spec §3.4 names ("never a marker for an unrelated new one").

### 3.5 §4 step-3 integration — the shutdown sweep

Composition spec §4 step 3: *deregister this scheduler's live registrations from
every `Shared` service before joining workers.* The landed teardown
(execution.rs:70-130) has no step-3 sweep yet — the only `Shared`-capable
services before this commit were the deferred rings. This commit inserts it.

**Ordering against the landed shutdown** (execution.rs:70-130), which today runs:
bridge stop (:78) → `io_ring.shutdown_owned()` (:88) → `drain_dirty_completions()`
(:97) → dirty pools `shutdown_owned` (:105-106) → file/standard rings (:111-112)
→ distribution (:121) → `shutdown.store(true)` (:122) → join workers (:124-129).
The readiness additions:

- **Owned poll thread** joins at the `shutdown_if_owned` position (spec §4
  step 4), i.e. a new `self.shared.readiness.shutdown_owned()` alongside the
  other `shutdown_owned` calls (execution.rs:105-121). This joins the thread
  before the `shutdown.store(true)` and worker join — satisfying contract §3.3's
  hard rule (reactor stopped and joined **before** the shutdown flag is set and
  workers are joined, so `enqueue_atom_message` can never fire into a torn-down
  process table).
- **Shared registrations** are swept at the **step-3 position — before joining
  workers, and before the `Shared` handle is released** (releasing the handle,
  service.rs:135, does NOT remove registrations). A new
  `self.shared.deregister_shared_readiness()` runs at the same phase as
  `drain_dirty_completions()` (execution.rs:97) — that call is the dirty-pool
  analogue of step 3 (close intake, wait out in-flight, before worker join);
  the readiness sweep sits beside it, and **strictly after it** (W2): the
  drain closes admission and waits out every in-flight `register`/`rearm`
  (§6, fifth row), so by the time the sweep walks the table every record an
  admitted straggler could have created is present and gets swept — the
  admission gate and the sweep order jointly close the
  register-after-sweep leak window. It calls, on the `Shared` slot only,
  `ReadinessCore::deregister_all_for(consumer_id)`: for each record stamped with
  this scheduler's `ServiceConsumerId`, bump the generation, deregister the fd,
  and epoch-handshake once for the batch (§4) so the shared poll thread can emit
  no further marker to this dying scheduler. **The sweep inherits §4.6's FAILED
  posture: on a FAILED shared service it tombstones the batch and returns
  WITHOUT the epoch wait (the delivery-order wall makes that airtight) — a
  panicked shared poll thread must not fail-stuck any consumer scheduler's
  shutdown (pair condition, folded).** `Owned`'s whole thread is joined, so
  it needs no per-record sweep; a `Disabled` slot no-ops.

Concretely, the step order becomes: … `drain_dirty_completions()` **+
`deregister_shared_readiness()`** (step 3) → `*.shutdown_owned()` including
`readiness.shutdown_owned()` (step 4) → `shutdown.store(true)` → join workers
(step 5). This matches spec §4's numbered order without disturbing the landed
dirty/ring/dist teardown.

The sweep is the VM-side complement of liminal's per-connection dereg
discipline (contract R4 "deliberate redundancy, not duplication"): neither side
depends on the other's diligence. Gated by composition spec §5 assertion 5
(strengthened): after one of two co-resident schedulers dies, zero registrations
target the dead scheduler.

---

## 4. fd lifecycle

### 4.1 fd-reuse race — the interleaving, killed by generation keying

The hazard (contract §3.4): consumer closes fd N, kernel recycles N for an
unrelated socket, a stale poll event for old-N delivers a marker to old-N's pid.
Walk:

1. Connection A registers fd N → slot S, generation G, route-home A, pid P_A.
2. The poll thread's kqueue/epoll dequeues a readiness event for fd N (armed by
   step 1) into its local event buffer — but has not yet taken the table lock.
3. A's connection closes N and `readiness_deregister`s slot S: under the table
   lock, generation S is bumped G→G′ and fd N is removed from the poll set (mio
   `Registry::deregister`). The record is now `Draining` at G′.
4. The kernel recycles N for connection B; B registers fd N → a **new** slot S_B
   (or S reused at a further-bumped generation G″), route-home B, pid P_B.
5. The poll thread finally takes the lock and looks up its buffered event. The
   event's token carries `(slot, generation_low)` bound at ARM time (§1.3 —
   the binding finding 2 demanded be named): G's low bits. It finds either a
   `Draining`/absent record at S, or S reused at G″ whose low bits ≠ G's —
   **generation mismatch, dropped.** No marker crosses to P_B.

The guarantee is structural (spec §3.4): the worst case is a spurious marker to
the OLD pid P_A (idempotent, C4/R6, harmless — and P_A is dead or draining), and
never a marker to the unrelated P_B. Pinned deterministically by the recycle-
storm test (contract §3.6), with liminal's real-kernel T4 as the probabilistic
end-to-end complement.

### 4.2 The dereg epoch handshake (contract §3.1 ACK)

`readiness_deregister(token)`:

1. Lock the table. If the slot's generation ≠ `token.generation`, the record is
   already gone (reaped at pid death, §3.5, or already dereg'd) — return; this is
   the **safe-against-dead-pid** path (C3), non-blocking.
2. Else bump the generation (tombstoning any in-flight event, §4.1), mark the
   slot `Draining`, and `Registry::deregister(fd)`. Read the poll thread's
   `poll_epoch` (an `AtomicU64` bumped at the top of each poll iteration — same
   discipline as the reservation drain's condvar wait, mod.rs:533-552). Unlock.
3. Signal the `mio::Waker` to break the poll thread out of a blocked `poll()`,
   and wait (bounded, on a condvar) until `poll_epoch` advances past the value
   read in step 2 — guaranteeing the thread has finished any iteration that could
   have been mid-delivery for this slot. Then free the slot.

After return, no marker attributable to this token — or to any DIFFERENT
registration on a recycled fd — can be delivered. This is the close-before-reuse
safety §3.4 builds on. The handshake is bounded by one poll iteration (the Waker
guarantees the thread is not blocked indefinitely). It is callable from the
supervisor/reaper, not the parked pid (Hermes point 3).

**Divergence from the contract's "deferred stop callback" language, noted
honestly:** contract §3.1 described OTP's deferred-ack as a callback path. beamr
folds the ACK into the generation-under-lock + epoch-wait above — stronger
(stale cross-delivery is impossible by generation regardless of the wait) and
synchronous. This is within the contract's own framing (§3.4: "generation tokens
close the whole class structurally instead of per-bug"), not a re-litigation.

### 4.3 Registration of an fd that closes underneath

If `fd` is invalid at `readiness_register` (already closed), mio
`Registry::register` returns `EBADF` → `ReadinessError::Register { errno }`, no
record created, caller RUNNABLE (Hermes point 4). If `fd` closes AFTER a
successful register but before any event, the kernel emits no further readiness
for it (or an error/HUP event — see below). Either way no stale cross-delivery
(§4.1).

**Error/HUP/EOF conditions deliver the marker REGARDLESS of the armed
direction** (first-consumer minor; incident-doc requirement 1 names
readable+HUP+error interest): kqueue/epoll report error and hang-up conditions
unconditionally, and the poll thread treats any such condition on a live
generation as readiness — the marker fires even if only `READABLE` (or only
`WRITABLE`) was armed, and the consumer's next slice observes EOF/the error
from its own nonblocking read and deregisters via its R4 table. A consumer's
EOF-teardown path may rely on this wake.

### 4.4 Poll-set membership under scheduler shutdown

- **Owned:** the poll thread is joined by `readiness.shutdown_owned()` (§3.5);
  its entire poll set is destroyed with it. No per-fd dereg needed.
- **Shared:** the thread survives; this scheduler's fds are removed by the
  step-3 sweep (§3.5) before worker join. The survivor's fds stay in the set.

Drop-without-shutdown must not leak the poll thread — same posture as the landed
distribution/NetKernel drop fix: `ReadinessCore`'s `Drop` signals the Waker and
joins the thread if `shutdown` did not already (idempotent, service.rs:150
comment).

### 4.5 EMFILE-class construction failure — refuse, do not half-install

Building the service needs a poll-set fd (kqueue/epoll) and a Waker fd/pipe. If
either allocation fails (EMFILE/ENFILE/ENOMEM), `ReadinessService::build_owned`
/ `SharedReadiness::new` returns `Err(ReadinessBuildError::PollSetUnavailable)`
and **no thread is spawned, no half-built poller is installed** — the
refuse-don't-degrade precedent (services.rs:184-190: "a composition request that
names a capability this release cannot deliver SAFELY is refused loudly and by
name, rather than silently degraded"). `Owned` surfaces it through
`with_services`' `Err(String)` (mod.rs:898); `Shared` to the embedder directly.
A service that cannot guarantee its lifecycle is not installed.

---

### 4.6 Poll-thread death — fail LOUD, never fail-stuck (first-consumer finding 3)

Unexpected poll-thread death is a supervision-integrity failure and must be
observable, not absorbed (the D4 watcher ruling; the vacuum A4 exhibit). The
posture:

- The poll thread body runs under a panic guard whose unwind path marks the
  service **FAILED** (an atomic poisoned flag on `ReadinessCore`) before the
  thread exits. A FAILED service is loud three ways: `register`/`rearm` refuse
  with a new typed `ReadinessError::ServiceFailed`; the §5 inventory entry
  reports `actual: 0` against `configured: 1` (the thread-name list is empty —
  the same truthful-divergence signal every joined/failed service shows); and
  the first refusal names the condition to the consumer, who owns the fatal-
  loud decision (the first consumer refuses at birth and would tear down).
- `readiness_deregister` on a FAILED service tombstones the generation under
  the table lock and **returns immediately, without the epoch wait** — the
  wait's only purpose is to bound fd/slot reuse against an in-flight delivery,
  and no new arming can occur on a FAILED service. This cannot fail-stuck a
  supervisor teardown on a dead thread.
- **Poisoned-lock posture (Waffles' half, condition W1).** The table lock is a
  `std::sync::Mutex` (beamr's house mutex — no parking_lot anywhere in the
  tree), so the worst-case panic — inside the delivery critical section,
  WHILE HOLDING the table lock — poisons it, and the posture must survive
  that exact case: a FAILED remedy that itself needs the lock the panic
  poisoned is a remedy that doesn't ship. The posture: `register`/`rearm`
  check the atomic FAILED flag BEFORE the lock and refuse without touching
  it; the only post-FAILED acquisitions are the dereg tombstone and the §3.5
  sweep, and both recover a poisoned lock via `PoisonError::into_inner()`
  and tombstone on the recovered table. That recovery is sound on ANY torn
  table state because tombstoning is monotone: bumping a generation can only
  make a record staler, never make a stale record live — there is no
  interleaving of the panic's partial writes that a generation bump can
  promote into a wrong delivery.
- Why tombstone-without-wait is airtight (the pair's precision, adopted): the
  impossibility of cross-delivery rests on the §3 delivery order — generation
  check under the table lock → weak upgrade → copy-and-unlock → deliver
  (§3.2) — being the left-hand wall for EVERY delivery path, **including an event already handed past the
  poll thread before the panic**. Thread death is circumstance; the generation
  check is the guarantee. The tombstone bump walls off any subsequent delivery
  attempt regardless of which thread would have made it.

Gate shape: kill the poll thread **while it holds the table lock** (the test
seam panics it inside the delivery critical section — the worst case W1
names, not a convenient lock-free point), assert `ServiceFailed` refusals,
`actual: 0` inventory, and a bounded `readiness_deregister` return that
tombstones through the poisoned lock; positive control on a healthy service.

## 5. The §5 story — inventory and idle-cost lens

### 5.1 Inventory entry

A new service label `READINESS: &str = "readiness"` (inventory.rs:17-30 pattern),
and a `readiness_entry` reading through `ServiceMode<ReadinessService>` exactly
as `ring_entry` / `distribution_entry` do (inventory.rs:144, :192). Pushed by
`build_service_inventory` (inventory.rs:227). Fields:

- `service`: `"readiness"`.
- `mode`: `Owned` (one scheduler) / `Shared` (N schedulers, one instance) /
  `Disabled`.
- `instance`: the propagated `ServiceInstanceId` (dedups Shared to one).
- `configured`: 1 when enabled (the one poll thread), 0 when Disabled.
- `actual`: live poll threads — 1 while running, 0 after an Owned join.
- `thread_names`: `["beamr-readiness-poll"]` — a service-distinct prefix
  `beamr-readiness`, matching the collision-fixing convention
  (`beamr-file-io`, `beamr-standard-io`, `beamr-generic-io`, io/mod.rs:47-51).
  A new `READINESS_POLL_THREAD_PREFIX` constant sits with those.
- `fd_classes`: `["poll"]` (the kqueue/epoll fd) plus `["waker"]` for the Waker
  fd/pipe — populated alongside the Linux fd probe (inventory.rs:66-68 notes
  macOS has no cheap fd probe in commit 1; readiness names them for when it
  lands).

The `deduped_thread_aggregate` (inventory.rs:278) already counts a Shared
instance once — no change needed; the readiness entry flows through it.

### 5.2 Idle-cost lens (contract §9 Q1–Q4; "sleeping costs nothing")

- **One idle registration costs:** one table-record — slot index, generation
  (`u64`), pid (`u64`), marker (`Atom`, `u32`), interest (`u8`), route-home
  (`Weak<SharedState>` + `ServiceConsumerId`) — on the order of tens of bytes,
  plus one kernel poll-set entry (one kqueue/epoll registration). **Zero
  threads, zero CPU** while the fd is not ready: the record sits in the table
  and the kernel holds the fd; nothing runs.
- **The poll thread at rest costs:** one OS thread blocked in `poll()` with **no
  timeout** — a true indefinite sleep, zero CPU, zero wakes when no fd is ready
  and no dereg/shutdown signals the Waker. This is the whole point of shape (b):
  unlike a normal worker's 5 ms park (§9), the readiness poll thread is tickless
  by construction. Memory is O(live registrations); zero disk, zero fsyncs
  (contract §9 Q1).
- **Aggregate ceiling (Q2):** one poll thread per Owned scheduler; exactly one
  per process under Shared, regardless of registration count. N registrations
  never spawn threads. Enforced by the §5.1 inventory assertion.
- **Quiescence (Q3):** the §3.6 enabled-idle soak (one poll thread, zero CPU
  over the soak window, T1-grade methodology per composition spec §7) + the
  Disabled-zero-threads assertion. Mechanical against `service_inventory()` +
  the OS probe.

### 5.3 Hermes' 256-registration ceiling arithmetic (point 5)

256 idle registered connection fds on one service:

- **Threads:** 1 (Owned: on that scheduler; Shared: one for the process). Not
  256, not 256×N.
- **fds:** 256 kernel poll-set registrations + 1 poll-set fd + 1 Waker fd/pipe
  = 258 fds attributable to the service (the 256 socket fds themselves are the
  consumer's, not the service's).
- **Memory:** 256 table records, ~tens of bytes each — single-digit KiB.
- **CPU at idle:** zero (poll thread asleep; §5.2).
- **Inventory line:** one `readiness` entry, `actual: 1`, `configured: 1`,
  `fd_classes: ["poll", "waker"]`; under Shared across, say, 4 schedulers, four
  entries with one shared `instance`, deduped to one thread.

---

## 6. Teardown riders from commit 5's review

Commit 5's review routed the remaining mutating facility ops to commit 6: they
must acquire teardown-admission gates so a mutation admitted before shutdown
closes intake finishes before shutdown returns, and one attempted after is
refused. The landed mechanism is the RAII `DirtyCompletionReservation`
(mod.rs:443-496) — its doc comment already frames the second use: "a mutating
facility operation (spawn family, io-message delivery, link) acquires one and
HOLDS it across its whole mutation, releasing via `Drop`." Commit 6 applies that
exact shape to the ops not yet gated:

| Op | Landed site | Admission pattern |
|---|---|---|
| Supervision `exit_signal` (link-cascade kill) | `process_exit_signal` / `finalize_exited_process` (core.rs:1656) | `try_reserve_dirty_completion()` (mod.rs:503) at entry, hold the reservation across the propagate+finalize mutation, release on `Drop`. A cascade admitted pre-drain completes before shutdown returns; one attempted post-drain is refused. |
| Group-leader set (process 0 / standard-IO) | `send_io_message`'s `standard_io_pid` path (mod.rs:1986) | reserve before the group-leader mutation; the drain waits it out. |
| ETS mutating ops (create/delete/transfer) | `create_table` / `delete_table` / `transfer_or_delete_tables_owned_by` (mod.rs:618-661) | reserve across the registry mutation so an ETS transfer cannot land after teardown returned. |
| Timer arm/cancel | `TimerKind::Deliver` scheduling (native_process.rs:320; timer_integration.rs:118) | reserve across the wheel mutation. NB: this gates the timer *facility mutation*, not readiness — the reply-deadline wake still rides the timer, not this service (Hermes point 6, §9). |
| **Readiness `register`/`rearm` (this commit's OWN ops — Waffles' half, condition W2)** | §1.4 registration surface | reserve across the table+poll-set mutation; post-drain they refuse typed (`ReadinessError::TeardownInProgress`, §1.5). Without this row the doc gates four pre-existing ops but not its own two: between the §3.5 sweep and worker join, a straggler slice could register a fresh fd, stamping a NEW record with the dying scheduler's consumer id AFTER the sweep already ran. The `Weak` upgrade makes it undeliverable (no correctness hole), but the record and its kernel poll-set entry would LEAK in a process-lifetime `Shared` table with no reaper — unbounded idle cost in principle, on exactly the service whose §5 story is costing nothing. |

The gate reuses the single `dirty_completions: Mutex<DirtyCompletionRegistry>`
(mod.rs:355) and its `closed`/`reserved`/`bridges` linearization and
`drain_dirty_completions` wait (mod.rs:533-552) — the same admission registry,
now guarding a broader set of mutations. No new registry type; the name
(`DirtyCompletionRegistry`) is now a slight misnomer for a general
teardown-admission registry — flagged in §7 as naming drift to consider renaming
(e.g. `TeardownAdmissionRegistry`) when the broader use lands.

---

## 7. Contract-fidelity / drift ledger

Honest notes where the contract spec's citations or names have drifted against
landed code at 675ee5e (the contract was verified at 103e5fd; the composition
spec at dbd18d8):

1. **C1 store-back merge line.** Contract §2.4 cites `execution/core.rs:394`
   for the pending-metadata merge; the landed merge of `pending_io_messages`
   into the mailbox is at **core.rs:415** (the `for message in
   metadata.pending_io_messages.drain(..)` loop). Same mechanism, line drift.
2. **Three-phase park.** Contract §2.1 cites `scheduler/execution/core.rs:83-155`
   for store→register→recheck; the landed sequence is at **core.rs:95-125**
   (the Wait arm's register-before-recheck comment block). Consistent, line
   drift.
3. **Bare-wake forbidden.** Contract §2.4 cites `scheduler/execution.rs:311-337`
   for `wake_process`/`wake_notifier`; landed `wake_process` is at
   **execution.rs:348-374**. Consistent, line drift.
4. **cleanup_exited_process fd close.** Contract §3.5 cites
   `execution/core.rs:1478-1536` for "closes process-owned fd resources before
   removing the process body." Landed: `cleanup_exited_process` is at
   **core.rs:1617**, delegating to `finalize_exited_process` (**core.rs:1656**)
   whose body removal + fd close is `release_process_exit_resources`
   (**core.rs:1711-1718**). The readiness eager-reap slots into
   `finalize_exited_process` **before** `release_process_exit_resources` runs
   (cancel-with-ack before fd close, discharging §3.4 on the crash path) — a
   `purge_readiness_state(pid)` call beside the existing
   `purge_suspension_state(pid)` (core.rs:1689). Structure matches the contract;
   line/function names drifted.
5. **Shutdown insertion point.** Contract §3.3 cites `execution.rs:69-93` for
   the ownership-ordered teardown; landed `shutdown` is **execution.rs:70-130**,
   already rewritten by the composition commits to the `shutdown_owned` +
   `drain_dirty_completions` shape. The readiness additions (§3.5) slot into
   this landed shape, not the contract's pre-composition sketch.
6. **`enqueue_atom_message`.** Contract §3.1 refers to "the existing
   `enqueue_atom_message` machinery" on `scheduler/mod.rs`; landed at
   **mod.rs:1946**, body :1947-1952. No drift beyond the line number.

No contradiction between the contract and the composition spec on any
design-decisive point was found; the readiness `Shared` arm being live in
commit 6 (this doc §1.2) is exactly what composition spec §3.9 + §11 step 7
require, and is consistent with the rings' `Shared` being *deferred to* commit 6
(services.rs:196) — deferred-to and delivered-in are the same commit.

---

## 8. Open questions

Only genuinely open items (Q-A..Q-F ruled; §3.9 multiplicity ruled; the
classification channel closed-as-unnecessary per the commit-5 review). Each with
a recommendation.

- **OQ-1 — `readiness` feature name.** Contract §3.1 marked the feature name
  "TBD". *Recommendation:* `readiness`, requiring `threads` (the poll thread and
  `enqueue_atom_message` are `threads`-only). Matches the contract's working
  name; no BEAM-visible surface, so no atom-name question (Q-B precedent).
- **OQ-2 — mio `Registry` vs `Poll` split behind the lock.** mio's `Registry`
  is `Sync` and can register fds while `Poll::poll` blocks, but the dereg epoch
  handshake (§4.2) still needs the Waker to bound the wait. *Recommendation:*
  hold one `mio::Poll` + its cloned `Registry` + one `mio::Waker` in
  `ReadinessCore`; register/rearm/deregister go through the `Registry` under the
  table lock, the Waker bounds the dereg/shutdown handshakes. This keeps "mio via
  the existing lock" (out of scope, §9) intact.
- **OQ-3 — rename `DirtyCompletionRegistry`. RESOLVED: YES, this commit** —
  first consumer and Vesper's half both rule it ("a registry whose name
  misdescribes its guard set is the pin-that-can't-fail failure mode wearing a
  name"); **Waffles' half SIGNED YES 2026-07-12** ("a name should describe
  the property it guards"). Rename to `TeardownAdmissionRegistry`
  (and `DirtyCompletionReservation` → `TeardownAdmission`); low-risk,
  `pub(in crate::scheduler)` only (mod.rs:457).
- **OQ-4 — `full_runtime()` readiness arm. RESOLVED: `Owned`** — first
  consumer no-objection and Vesper's half concur (the standalone-VM reading
  the pair confirmed for distribution); **Waffles' half SIGNED YES
  2026-07-12** (the distribution transfer of that reading, confirmed).
  Multi-scheduler embedders build `SharedReadiness` and inject it.

**First-consumer composition, recorded (liminal):** readiness composes `Owned`
on the CONNECTION scheduler only, `Disabled` on channel/conversation schedulers
(they own no fds) — no `Shared` needed for liminal v1 in either profile;
readiness build failure at server startup is fatal-loud on the consumer side (a
server that cannot park connections is the incident, refused at birth); the
consumer's D2 census pins will assert the `beamr-readiness-poll` inventory
line.

---

## 9. Explicitly out of scope

- **mio / backend choice details** — ruled (mio via the existing lock). This doc
  keeps mio out of the public signature (§1.3) and does not re-open the backend
  decision.
- **Tickless idle** — the normal-worker 5 ms `IDLE_PARK_TIMEOUT`
  (execution.rs:758) is its own named follow-on commit (composition spec §3.8
  Q-F ruling). The readiness poll thread is tickless by construction (§5.2), but
  this doc does not touch the worker park surface or the signed 5 ms bound.
- **Timer surfaces (Hermes point 6)** — the reply-deadline expiry wake
  (contract R1(vi)) rides `TimerKind::Deliver` (timer_integration.rs:118,
  native_process.rs:320) under `Capability::Clock` (native/bifs.rs:42-44), NOT
  this service. The readiness service delivers on fd readiness only; it makes no
  claim on timer expiry and merely coexists. §6's timer rider gates the timer
  *facility mutation* for teardown admission — a distinct concern.
- **Shared distribution** — remains out of scope v1 (composition spec §3.6);
  unaffected by readiness `Shared`.
