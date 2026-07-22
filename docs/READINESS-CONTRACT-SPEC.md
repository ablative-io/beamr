# Readiness Contract & Service Spec

**Status:** DRAFT v0 — skeleton + beamr-side content; liminal-consumer sections
pending (Hermes Crumpet); enif_select evidence pack pending (norn research
session, in flight). Not for implementation until reviewed by Vesper Lynd and
certified per the post-incident governance (Vesper Lynd + Waffles the Terrible).

**Authors:** Artemis Peach (beamr), Hermes Crumpet (liminal)
**Provenance:** docs/stack-review/AION-HOST-RESOURCE-INCIDENT-2026-07-11.md;
stack-devs sync of 2026-07-11; Tom's design ruling of the same day (shape (b)
primary).

---

## 0. Design principle

**Sleeping must cost nothing, everywhere, forever.**

A connection (or any fd-backed resource) with no work must consume zero CPU,
zero scheduler slices, and a bounded, inventoried number of resident threads —
not per idle resource, but in total. The BEAM settled this problem thirty years
ago: socket readiness is a VM service (`enif_select` and its driver-level
ancestor), which is why a stock BEAM node holds a million idle connections for
free. We own the VM. The question this spec answers is not "how does an
embedder build a reactor beside beamr" but "what is beamr's native answer to
readiness" — with the embedder-owned reactor argued honestly as the fallback.

This spec must satisfy standing rules 1–5 (see beamr conventions / the incident
doc): permanent negative resource assertions, no silent tradeoffs, a "how the
original shipped" section (§8), contract-before-code, and the idle/resource-cost
lens answered in full for anything resident this spec creates (§9).

## 1. The two shapes

- **Shape (b) — PRIMARY: beamr-owned readiness service.** A feature-gated,
  explicitly-owned VM service: register `(fd, pid, marker)`, the service polls,
  and readiness is delivered as a durable mailbox marker via the existing
  `enqueue_atom_message` machinery. One poll thread for the whole VM, owned and
  inventoried under the embedder-composition model. mio is already in beamr's
  dependency graph via tokio's `net` feature (verified in Cargo.lock: mio
  1.2.1) — this shape adds zero new crates and requires zero unsafe anywhere.

- **Shape (a) — FALLBACK: embedder-owned reactor.** The consumer (liminal's
  supervisor) owns the poll thread and registration bookkeeping; beamr's
  obligations reduce to the shape-invariant contract of §3 plus a race-safe
  notifier convenience. This shape is acceptable only if (b) loses on the
  merits, with the losing argument documented in §8's decision record. It is
  held to the same standard as (b) — including the observation that it
  re-implements the poll set per embedder, forever, each instance needing its
  own lens answers.

**Decision criteria** (agreed): correctness of the shutdown-lifecycle story,
fd-reuse safety, aggregate idle cost across all current and future embedders,
API commitment weight, and testability. Not a criterion: "what standard Rust
projects do."

## 2. Shape-invariant core contract

These clauses hold under BOTH shapes and are the normative surface consumers
build against. They were converged in the 2026-07-11 sync and verified against
source at beamr 103e5fd. The pinning test suite (§2.5) lands BEFORE any
consumer merges code against this contract.

### 2.1 C1 — Durable markers survive every race order

A term delivered to a process mailbox via `Scheduler::enqueue_atom_message` is
never lost, regardless of where the target is in its execute/park cycle:

- A **parked** process: delivery lands in the mailbox and the wake makes the
  process runnable.
- A process **mid-park** (between store and wait-set registration): the
  three-phase park (store → register → mailbox recheck;
  `scheduler/execution/core.rs:83-155` at 103e5fd) re-checks the mailbox after
  registration, so a delivery in the gap is observed before the process sleeps.
- An **executing** process: delivery goes through pending metadata, is merged
  into the mailbox at store-back, and the process is resumed if it suspended
  meanwhile (`Scheduler::enqueue_atom_message` rustdoc, `scheduler/mod.rs`).

### 2.2 C2 — Observed-or-runnable, scoped to plain parks

For a process parked via `NativeOutcome::Wait` (or a message-wakeable
`request_suspend`): a durable marker enqueued at any moment results in either
the current slice observing it or the process becoming runnable. **No lost-wake
window exists.**

**Scope limit (normative):** a process parked under a *gated* suspension
(`request_await_suspend`, an in-flight dirty call, a hook suspend) keeps the
marker in its mailbox but stays parked until its own completion event arrives —
deliberately, since waking it would re-execute the parked call and double its
side effect. A readiness consumer MUST NOT be built on a gated-suspend process.
Connection processes park via `Wait` and are in the strong case.

### 2.3 C3 — Dead-pid semantics

`enqueue_atom_message` returns `false` iff no live process exists for the pid
(nothing enqueued; harmless). **`true` means delivered, not will-be-observed:**
a pid that dies between a true-returning enqueue and its next slice drops the
marker with its mailbox. Registration bookkeeping (either shape) must tolerate
true-but-never-observed.

### 2.4 C4 — Consumer discipline (register-before-probe, probe-before-park)

The consumer's slice shape, both shapes:

1. Drain bounded work until `WouldBlock`.
2. Arm (or re-arm) readiness interest **before** the final probe.
3. Final non-blocking probe (close the arm-vs-event race from the consumer
   side).
4. Return `NativeOutcome::Wait`.

Markers are idempotent: N readiness events may coalesce to one marker plus one
drain; the drain loop, not the marker count, is the unit of progress.

**Scope of the final probe (pinning-suite finding, 2026-07-11):** step 3 is
load-bearing ONLY for consumer-owned event sources — socket buffers,
subscription inboxes, reply queues — whose state can change mid-slice
invisibly to the VM. It is structurally incapable of observing VM-mailbox
markers: a marker delivered mid-slice lands in `pending_io_messages` and
merges into the mailbox only at store-back (`execution/core.rs:394`), so no
same-slice recv/probe can see it. Mailbox markers are protected by C1's
store-back merge + recheck, not by the probe. Consumers must NOT add a
mailbox re-probe to step 3 and believe it is the safety mechanism — it would
be dead code that misattributes where the guarantee lives. The §2.5 suite
pins both mechanisms independently.

A bare `wake_notifier`/`wake_process` without a durable marker is **forbidden**
as a readiness signal (no-op on a not-yet-registered pid;
`scheduler/execution.rs:311-337`).

### 2.5 Pinning suite (beamr deliverable, lands first)

- C1 in all three timing positions (parked / mid-park gap / executing), each
  deterministic, not schedule-hopeful.
- C2 strong case + the gated-suspension scope limit (marker retained, park
  preserved, observed at completion).
- C3 both cases: `false` on dead pid; `true` then death-before-slice drops the
  marker without wedging anything.
- Negative: bare wake before wait-set registration is lost (pins WHY the
  contract demands durable markers).
- **C4's race-closing ordering, deterministic (review advisory 2):** a
  delivery landing in the exact window between interest-arm and the final
  probe is not lost — pinned in-crate with a consumer-shaped native process
  and an injected delivery at that precise interleaving, deterministically
  (not schedule-hopefully). This is the single most load-bearing line of
  consumer discipline and it is pinned where the harness can control the
  interleaving: beamr.

## 3. Shape (b): the beamr readiness service

> Sections 3.1–3.6 are the beamr-side design. The OTP prior art cited below
> is from the enif_select/driver_select evidence pack produced by GPT-5.6-Sol
> research session `233a223e-c3e0-4b79-9aba-bf617d8d40b5` (envelope retained
> at `~/.norn/delegations/claude-research.ue74HX`; OTP claims pinned to
> upstream commit 9c288883, each with source/doc line citations in the pack).
> Design-decisive claims were spot-checked against the cited OTP sources.

### 3.1 API sketch

```rust
// Feature "readiness" (name TBD), on Scheduler or a service handle:
fn readiness_register(
    &self,
    fd: RawFd,
    interest: Interest,          // READABLE | WRITABLE | both
    pid: u64,
    marker: Atom,                // durable marker delivered on readiness
) -> Result<ReadinessToken, ReadinessError>;

fn readiness_rearm(&self, token: &ReadinessToken, interest: Interest) -> Result<(), ReadinessError>;
/// Returns only once the registration can deliver no further marker —
/// the deregister-ACK the close safety of §3.4 builds on.
fn readiness_deregister(&self, token: ReadinessToken);
```

- **One-shot delivery — confirmed as OTP's normative contract**: one
  notification per selected direction per registration; another notification
  requires another select call (erl_nif docs, enif_select; ERTS separates
  requested `events` from armed `active_events` and clears triggered bits
  before send). The consumer loop OTP prescribes is exactly C4: drain to
  WouldBlock → re-arm → park. One-shot removes the level-triggered storm
  class and makes marker idempotence trivial. (OTP later moved hot fds to a
  non-ONESHOT internal pollset purely as a syscall optimization while
  preserving the externally-one-shot semantic — the optimization lesson: keep
  any fast path invisible behind the one-in-flight contract.)
- **Deregistration is an acknowledged operation** (the ERL_NIF_SELECT_STOP
  lesson): OTP treats stop as a handshake — cancel directions, eject the fd
  from pollsets (waking the poller if it might still hold the fd), and only
  then dissolve the fd relation, via a direct or deferred stop callback.
  beamr's equivalent: `readiness_deregister` returns only after the poll
  thread can no longer emit an event for that token (deferred-ack path when
  the poller is mid-poll). Close-before-dereg-ack is the documented consumer
  violation R5 guards against.
- `ReadinessToken` carries a **generation** minted at register time; stale
  events for a dead generation are dropped in the service, never delivered
  (§3.4). The atom marker is the public contract; the token is the internal
  registration identity — matching OTP's split between the delivered message
  (`{select, Obj, Ref, ready_*}` / caller-chosen term) and the VM-side
  per-fd state that actually gates delivery. A durable atom is valid as the
  message precisely because R6 makes it idempotent ("at least one readiness
  fact is pending", not "one atom per kernel event").
- Delivery is exactly `enqueue_atom_message(pid, marker)` — the service adds
  no new delivery machinery and inherits C1–C3 verbatim. (OTP's equivalent
  drop-if-recipient-dead behavior at send time is what C3 already gives us.)

### 3.2 Ownership under the composition model

The service is born inside the embedder-composition redesign (the sibling
workstream deliverable), not bolted beside it:

- **Feature-gated** (compile-time) and **config-gated** (disabled / owned /
  injected-shared, like every other service after the redesign). Disabled = 
  zero threads, zero fds, registration returns `ReadinessError::Disabled`.
- **Inventory line from birth:** the service reports exactly one poll thread
  (plus its wakeup pipe/eventfd) through the thread-inventory API. The
  inventory test asserts the OS-visible thread name matches.
- **Lens answers written before code** (§9).

### 3.3 Poll-set lifecycle under scheduler shutdown ⏳

The known-delicate spot (this codebase's shutdown-join ordering was the fragile
part of the 0.13.0 e2e work). Requirements:

- `Scheduler::shutdown` joins the poll thread deterministically; a poll set
  must never outlive its scheduler (a resident orphan is exactly the rule-5
  failure class).
- Shutdown with live registrations: defined order — stop accepting
  registrations, drain/discard pending events, deregister all, join. No marker
  is delivered after `shutdown` returns (C3 makes late markers harmless, but
  the service must not depend on that for correctness).
- **Shutdown symmetry of R4's principle (review advisory 1):** a parked
  connection must NEVER need to run a final slice for scheduler shutdown to
  complete. Teardown owns deregistration and cleanup itself — it must not
  wake parked processes "to let them clean up"; that pattern is one careless
  implementation away from a shutdown hang, and T5 asserts its absence.
- Drop-without-shutdown must not leak the thread (same posture as the
  NetKernel drop fix at 103e5fd).
- **OTP offers no reusable prior art here — deliberately diverge.** ERTS
  poll threads run an infinite loop and pollsets have no destruction
  sequence; whole-VM halt relies on OS-process exit to reclaim them. That is
  unavailable to a reusable embedded `Scheduler`, so the ordering above is
  ours to own, with one hard rule the evidence pack makes explicit: the
  reactor must be stopped and joined **before** the scheduler shutdown flag
  is set and workers are joined, so `enqueue_atom_message` can never fire
  into a torn-down process table. The compatible insertion point is the
  existing ownership-ordered teardown in `execution.rs:69-93` (rewritten by
  the composition spec §4), ahead of worker join.

### 3.4 fd-reuse and stale-delivery safety

The hazard: consumer closes fd N (or crashes), kernel recycles N for an
unrelated socket, a stale poll event for old-N delivers a marker to the old
registration's pid.

- Registrations are keyed by generation token, not fd number; on every poll
  event the service requires `(slot, generation, pid, interest)` to match the
  live record before enqueueing — stale generations drop in the service,
  never delivered.
- **This is a deliberate improvement over OTP, not a transcription.** OTP
  attaches no generation to the fd: its guard is serialized per-fd state
  (selected vs active masks, owner identity, steal/deselect cleanup) **plus
  the client obeying STOP-before-close** — close behind ERTS's back and
  fd-keyed state cannot distinguish old fd 42 from new fd 42. The area's
  history is a fragility ledger (bad-fd handling fixed during enif_select's
  own development; cleared-one-shot results filtered; fallback-stop
  corrected; scheduler-local select state needing two later cleanup fixes;
  a hot-fd migration heuristic keyed only by fd regressing under concurrent
  accept and gaining owner-awareness in OTP PR #10323). Generation tokens
  close the whole class structurally instead of per-bug.
- **Deregister-ACK-before-close is the documented consumer obligation**
  (§3.1; R5 enforces it in liminal); the service additionally guards:
  close-vs-poll races on a still-registered fd produce at worst a spurious
  marker for the OLD registration (idempotent, C4/R6), never a marker for an
  unrelated new one — the generation check makes this a structural property,
  and the §3.6 recycle-storm gate plus liminal's T4 pin it from both sides.

### 3.5 Process-death deregistration

Registrations owned by a dead pid are reaped **eagerly at the centralized
exit path**: `cleanup_exited_process` already closes process-owned fd
resources before removing the process body (`execution/core.rs:1478-1536`),
and readiness cancellation slots there — cancel (with ack) BEFORE the fd
close, which also discharges the §3.4 obligation on the crash path where no
consumer code runs. Lazy reaping on next event remains as the backstop (C3
makes delivery harmless; the reap bounds table growth — a rule-5 answer).

This deliberately diverges from OTP, where a registration is NOT owned by
the recipient process — it rides the NIF resource, ERTS merely drops the
pending message if the recipient died, and leak prevention is delegated to
the client via `enif_monitor_process` + a down callback. beamr's pids own
their registrations directly; the VM cleans up, not the embedder. (Hermes'
R4 externally-killed-pid finding is the same principle on the consumer side:
dereg must never require the dead process to run a slice.)

### 3.6 Acceptance gates (shape b)

- Disabled service: zero threads (inventory + OS assertion), registration
  refused explicitly.
- Enabled idle service: exactly one poll thread, zero CPU at idle over a soak
  window (the beamr sibling of liminal's 11-idle-worker soak).
- Register/close/recycle churn storm: no stale marker crosses generations
  (deterministic fd-reuse test, not schedule-hopeful).
- Shutdown under load with live registrations: clean join, no post-shutdown
  delivery, no leaked thread (OS-visible).
- Every C1–C4 pinning test passes with the service as the marker source.
- The one-shot + generation semantics are asserted at the VM contract level
  on BOTH backends we ship (kqueue via mio on macOS, epoll on Linux) — OTP's
  OpenBSD non-one-shot pollset bug is the lesson: never infer backend
  uniformity from the abstraction; test the contract, not the backend.
- Q1–Q4 lens answers in this doc match measured behavior.

## 4. Shape (a): embedder-owned reactor — fallback argument

> **Owner: Hermes Crumpet.** Held to the same "sleeping costs nothing" standard;
> must answer the same §3.3–§3.5 hazards from the liminal side, plus the
> N-embedders-N-reimplementations aggregate cost under lens Q2.

**What (a) is.** Liminal's connection supervisor constructs and owns one
reactor thread for all connection sockets. It performs kqueue/epoll
registration through a safe wrapper crate, keys registrations by
(pid, generation), and delivers readiness exactly as shape (b) does: a durable
marker via `enqueue_atom_message`. The C1–C4 contract is consumed identically;
the only difference is which repo owns the poll set.

**Honest merits.**
1. *No new beamr public API commitment.* The service surface of §3.1 is a
   forever-API for every future embedder; (a) defers that commitment until a
   second consumer proves the shape. (Counterweight: frame is already the
   named second consumer, and aion's worker runtime a third — the "wait for a
   second consumer" argument is weaker here than it usually is.)
2. *Consumer-local iteration.* Registration bookkeeping bugs are fixed in
   liminal's release cadence, not the VM's.
3. *Blast radius.* A reactor defect degrades liminal-server; a VM-service
   defect degrades every embedder simultaneously.

**Costs under the agreed decision criteria.**
1. *Aggregate idle cost (lens Q2 — the decisive one).* Every embedder that
   holds fds re-implements the poll set: liminal today, frame's node surface
   next, any future embedder after that. N implementations × one thread each,
   each needing its own registration table, its own fd-reuse guard, its own
   shutdown-join story, its own lens answers, its own soak test. Shape (b)
   answers Q1–Q4 once, in the VM, for everyone forever. Under the campaign's
   own doctrine, (a) is rule-5 debt issued at architectural interest rates.
2. *Dependency graph.* mio is already compiled into every beamr net build via
   tokio; (a) adds a NEW direct dependency to liminal's workspace (mio or
   polling) to duplicate machinery the graph already contains. Zero-new-deps
   favors (b) outright.
3. *Testability of the hard parts.* §3.3 (poll-set vs scheduler shutdown) and
   §3.4 (fd reuse) need deterministic tests against scheduler internals —
   park/wake injection points that only beamr's test harness reaches. Under
   (a), liminal can only test those races schedule-hopefully from outside the
   VM; under (b) they are in-crate deterministic tests. The hardest
   correctness obligations land where the test tooling is weakest — the
   opposite of what we want.
4. *Shutdown lifecycle is not actually simpler under (a).* The reactor thread
   must still join deterministically with the beamr scheduler teardown it does
   not own; (a) moves §3.3 across a repo boundary rather than removing it.

**When (a) wins.** Only if §3.3 cannot be made deterministic inside beamr
(the poll thread provably cannot join cleanly under the scheduler's existing
shutdown ordering), or if the certifying pair judges the §3.1 API commitment
premature despite the named consumers. If (b) fails its §3.6 gates during
implementation, (a) is the pre-argued fallback and this section becomes the
implementation spec: same C1–C4 consumption, same §5 consumer discipline,
same §6 churn gates, with the registration table and generation guard
implemented in `liminal-server`'s supervisor and inventoried through liminal's
lens answers.

**Recommendation from the (a) author:** shape (b), unless its §3.6
shutdown-lifecycle gate fails on the merits. I designed (a) first and
preferred it for seam-narrowness; the decision criteria — argued from
"sleeping must cost nothing, everywhere, forever" rather than from Rust
convention — reverse that preference. Preserving this argument per the §8
decision record either way.

## 5. Liminal consumer requirements

> **Owner: Hermes Crumpet.** Verified against liminal main at 218a378 /
> published 0.2.3. R-numbers are the consumer-side normative requirements;
> the liminal design doc implements them and cites this section.

### 5.1 The slice shape (replaces the no-sleep Continue discipline)

Current shape (`liminal-server .../connection/process.rs:86-158`): drain
controls → service socket → pump subscriptions → drain outbound → return
`Continue` unconditionally. The incident-critical property: idle connections
are permanently runnable.

New shape, per C4:

1. Drain queued control messages (existing path, already marker-driven).
2. Service inbound socket work, bounded (existing read/apply budget).
3. Pump subscriptions into the outbound writer, bounded and headroom-aware
   (existing `DELIVERY_SLICE_BUDGET` + held-delivery machinery, unchanged).
4. Drain the outbound writer with partial-write tracking (existing).
5. If work remains that can progress WITHOUT a new external event — complete
   frame already in the read buffer, queued control, held delivery WITH
   outbound headroom, outbound residue stopped by the slice budget rather
   than `WouldBlock` — return `NativeOutcome::Continue`: the budget, not
   readiness, is what stopped us.
6. Otherwise every remaining work item is blocked on an external event
   (empty socket, `WouldBlock`'d writer, empty inboxes): arm/re-arm interest
   per R2 (READABLE always; WRITABLE iff blocked residue), take the **final
   non-blocking probe** across the event-free sources (read buffer, control
   queue, subscription inboxes), and if it observes nothing return
   `NativeOutcome::Wait`. If the probe observes work, drain it or return
   `Continue` — never `Wait` with known work pending (C4). R2's tri-state is
   what makes the step-5/step-6 split decidable — the drain's return value
   IS the budget-vs-`WouldBlock` discriminator.

Connection processes park via plain `Wait` only — never a gated suspension
(C2 scope limit is a normative obligation on liminal, asserted by test).

### 5.2 Requirements

- **R1 — Complete wake-source coverage.** A parked connection must be woken
  by every source that can create work for it: (i) inbound socket bytes;
  (ii) EOF/HUP; (iii) control messages incl. server push and shutdown
  (existing control-atom path, already C1-conformant —
  `supervisor.rs:430-447` — preserved unchanged); (iv) a subscription inbox
  becoming non-empty (R3); (v) outbound-writable after a blocked drain (R2);
  (vi) **conversation participant replies — reply availability OR
  reply-deadline expiry.** The expiry half is Waffles the Terrible's
  certification finding (2026-07-11, adopted joint): the pending-reply
  timer's expiry must itself deliver the connection's READY marker,
  because nothing else guarantees a wake — without it a parked connection
  whose reply times out under zero other traffic never wakes, the client
  waits indefinitely, and R7's static counter reads the failure as
  health (a quiescence assertion cannot see a MISSING wake). T2's
  per-source matrix covers both halves. *The availability half was found
  by the independent Sol scout cross-check (session 02468176), which
  verified (i)–(v) and exposed what the author missed: today a reply-requested
  `ConversationMessage` blocks INSIDE the connection slice for up to 5 s
  (`connection/apply.rs` reply drain) — under the current busy-loop that
  merely wastes a worker; under parking it is a scheduler-wedge (four
  concurrent reply waits block all four workers) and a park-correctness
  hazard. The consumer design therefore converts the reply drain to a
  pending-reply continuation: the participant's reply delivery fires the
  connection's marker (same install-before-recheck ordering as R3) and the
  next slice writes the reply frame. This removes the last bounded-blocking
  wait from the slice path.* The scout also confirmed both R3 legs: local
  publish writes the inbox without messaging the connection, and the remote
  cluster leg wakes only the subscriber process — neither reaches the
  connection pid today (`delivery.rs:1-9`, `subscription.rs:99-128`).
- **R2 — Outbound writer tri-state.** `OutboundWriter::drain` currently
  returns the same `Ok(())` for drained-empty and blocked-with-residue
  (`connection/outbound.rs:156-192`). It must distinguish them so WRITABLE
  interest is armed iff residue remains. Interest is one-shot per §3.1:
  re-armed on each blocked drain, never held level-triggered.
- **R3 — Subscription inbox notifier.** The channel core's per-subscriber
  inbox gains an on-empty-to-non-empty notifier slot, installed by the server
  at subscribe time, that delivers the connection's durable marker (an
  `enqueue_atom_message` call — cheap, non-blocking, safe on the publishing
  actor's slice). Install-before-final-recheck ordering closes the
  publish-vs-park race from the liminal side (the C1 mid-park recheck closes
  it from the VM side). The remote/cluster delivery leg
  (`SubscriberProcess::accept_remote_frame`) fires the same notifier. The old
  assumption that the connection re-polls every slice
  (`connection/delivery.rs:1-12`) is deleted with prejudice.
- **R4 — Registration lifecycle: register once, deregister on every
  termination path.** Registration happens at connection spawn (post stream
  handoff). Deregistration is table-driven over the complete termination set,
  verified against source: EOF/`ReadStatus::Closed`; `ProcessStatus::Close`
  (client Disconnect, `RespondThenClose`); outbound overflow/write-error
  teardown; `mark_crashed` paths; `ForceClose` control; **externally-killed
  pids observed only by `reap_crashed`** (`supervisor.rs` reap loop — dereg
  must not require the dead process to run a slice; the supervisor/reaper
  owns it); scheduler shutdown (§3.3 ordering). Under shape (b), C3 +
  §3.5 lazy reaping is the backstop, not the mechanism: liminal still owns
  explicit dereg on every path above. Likewise the service's dead-scheduler
  registration sweep (composition spec advisory 2) and this consumer-side
  table are **deliberate redundancy, not duplication**: neither side depends
  on the other's diligence — the consumer deregisters per connection on
  every termination path; the service sweeps whatever a wedged consumer
  leaves behind. The consumer design doc records the same statement at its
  §1.2(5).
- **R5 — Generation keying.** The connection's registration token (with its
  generation) lives in connection state; a recycled fd or reused pid never
  resolves to a stale registration (§3.4). Deregister-before-close is honored
  on every path in R4's table.
- **R6 — Single idempotent marker.** One readiness atom per connection, not
  one per source. Any marker (or N coalesced markers) triggers one full slice
  that services ALL work sources (5.1 steps 1–4). Progress is the drain
  loop's property, not the marker count's (C4). Rationale: eliminates
  marker-taxonomy races and makes duplicate delivery structurally harmless.
- **R7 — Quiescence instrumentation.** A test-only per-connection slice
  counter, exposed for the §6 gates and the 11-idle-worker soak: parked
  connections' counters must not advance without an event. This is the
  permanent rule-1 negative assertion for liminal.
- **R8 — Worker front door parity.** Aion-style push connections (PushClient
  consumers) use the same slice shape and the same wake sources; the
  worker-front-door services redesign (liminal workstream, separate doc)
  changes what services exist behind the connection, not how it parks.

### 5.3 Two shape-invariant liminal fixes this contract assumes

R2 (tri-state) and R3 (inbox notifier) are liminal-side prerequisites under
BOTH shapes — they are part of the liminal workstream regardless of the
shape decision and carry their own tests independent of §3.

## 6. Churn-driven acceptance tests (cross-repo)

> **Owner: Hermes Crumpet** (liminal exercises real sockets), with the beamr
> deterministic harness from §3.6 as the in-crate floor. These are the
> liminal-side gates; each is a permanent rule-1 assertion, not
> incident-scoped.

- **T1 — The 11-idle-worker soak** (the incident's own gate), shaped as a
  **delta assertion** per the certifying pair (2026-07-11): measure the
  server at true baseline (zero connections) and with 11 registered,
  connected, idle workers, over the same soak window with recorded
  methodology (duration, sampling source, host state) so the delta is
  reproducible rather than a one-shot reading. Certify that liminal's
  contribution ABOVE the VM's own idle floor is ~zero; every parked
  connection's R7 slice counter is static for the entire window. The floor
  itself (beamr's ~5 ms scheduler tick, ~4,800 wakes/s at perfect idle on
  the incident host) is a first-class Q1 item owned by the beamr
  composition lane, with its own bound and sign-off. The delta stays true
  when the floor moves, so a signed T1 survives the floor being cheapened
  later. Runs on macOS (the incident host) as the operational gate.
- **T2 — Parked-wake matrix**: for each R1 wake source (inbound bytes → Pong
  round-trip; server push → correlated reply completes; publish →
  subscription Deliver arrives; EOF/HUP → dereg + teardown; blocked-then-
  writable → exact residue bytes flush in order): park (counter static),
  fire the source exactly once, assert exactly one wake, correct behavior,
  and re-park (counter static again).
- **T3 — Churn beyond worker count**: connections ≫ scheduler workers (e.g.
  64 over 4) doing connect/publish/subscribe/disconnect bursts: all
  correlated replies correct, no starvation, and — the busy-poll regression
  guard — full quiescence (all counters static) in the gaps between bursts.
- **T4 — Real-kernel fd reuse**: close connection A and immediately accept
  connection B (kernel plausibly recycles the fd) under sustained event
  traffic; assert B never observes a wake or frame attributable to A's
  registration (generation guard end-to-end). Probabilistic here, repeated;
  the deterministic variant is beamr's §3.6 recycle-storm test.
- **T5 — Shutdown under churn**: server shutdown mid-traffic with live parked
  and active connections: drain semantics preserved, all connections
  deregistered (R4), no marker delivered post-shutdown, all threads joined
  (OS-visible), no leaked registrations (table empty assertion).
  T5 pins the §3.3 shutdown-symmetry rule from the consumer side: it parks
  connections that are NEVER woken and requires shutdown to complete anyway
  — supervisor/reaper-owned teardown, no final slice, per advisory 1.
  T5's empty-table assertion is the consumer end of R4's deliberate
  redundancy; the service end (zero registrations targeting a dead
  scheduler) is gated in the composition spec's strengthened assertion 5.
- **T6 — Duplicate/coalesced markers**: inject N markers for one event;
  exactly one drain pass, no duplicate frame application, no counter
  inflation (R6).
- **T7 — Slow-reader under park** (extends the existing headroom tests):
  a parked slow reader with queued outbound wakes only on writable
  readiness, flushes byte-exact residue, re-parks; never spins.

## 7. Sequencing

1. This doc converges (both authors satisfied) → routes to Vesper Lynd.
2. beamr lands the §2.5 pinning suite (contract tests) on a focused branch.
3. Shape decision certified (Vesper + Waffles independently, prose to Tom).
4. Service (or reactor) implementation, gates green, norn review passes in
   addition to the mandatory battery.
5. Liminal consumer merges only after §2.5 is green on main beamr.

## 8. How the original shipped (rule 3) + decision record

**beamr half:** the busy-poll's amplification shipped as thirty reasonable
threads: each eager service (dirty pools, fallback rings, distribution
runtimes) was added one defensible decision at a time, none with a ceiling, a
test, or sign-off on the aggregate. The missing control was precisely lens Q2
(aggregate ceiling across instances) — no review round was ever forced to ask
what the sum was, and the thread-inventory API plus its permanent negative
assertions is the gate that would have caught it.

**liminal half:** the permanent-runnable loop shipped in three self-aware
steps, none of them ignorant. `ff8d863` (SRV-003 review fixes) replaced a
10 ms sleep with permanent requeue — a correct fix for the local defect (a
sleeping connection blocked one of four scheduler threads) that traded it for
an unbounded global one, and said so in a comment. `bb81724` (H1) then built
the delivery pump ON the busy loop and cited it as a feature: "no wakeup
plumbing needed — the connection already runs every slice." The repo ledger
recorded "busy-polls by design." I wrote the latter two. The missing controls,
in order of proximity: no idle-cost negative assertion existed anywhere in the
suite (rule 1 — T1/R7 are that gate now); the adversarial review battery that
caught six real bugs the same week ran correctness and house-rules lenses only
— no lens ever asked "what does this cost when idle?" (rule 5 — the
idle/resource-cost lens is that gate now); and "by design" was accepted as
authorization with no bound, no test, and no sign-off (rule 2 — the certifying
pair is that gate now). Documentation of a defect is not authorization for it;
we documented it twice and called it design both times.

**Decision record:** *(pending — filled when the shape decision is made, with
the losing shape's argument preserved.)*

## 9. Idle/resource-cost lens answers (canonical text v1.1, applied to shape b)

- **Q1 (idle cost):** one OS thread, parked in the poller (zero CPU when no fd
  is ready); memory O(live registrations); zero disk, zero fsyncs. Pinned
  ceiling: 1 thread, asserted against OS thread names in the inventory test;
  idle-CPU soak gate in §3.6.
- **Q2 (aggregate ceiling):** the ceiling IS the point — one poll thread per
  scheduler (or one shared, per the composition model's injected form)
  regardless of connection count, versus shape (a)'s one-reactor-per-embedder-
  forever. Enforced by the inventory assertion; N registrations never spawn
  threads.
- **Q3 (quiescence test):** §3.6 enabled-idle soak + disabled-zero-threads
  assertion; both new with this diff and both fail if Q1/Q2 answers are wrong.
  Mechanical check against the thread-inventory API, not reviewer estimate.
- **Q4 (by-design costs):** the one accepted resident cost (one poll thread
  when enabled) carries: this bound (here), the pinning tests (§3.6), and
  sign-off by the certifying pair (Vesper Lynd + Waffles the Terrible), Tom
  briefed. Nothing else is accepted by design.
