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

## 3. Shape (b): the beamr readiness service

> Sections 3.1–3.6 are the beamr-side design. Items marked ⏳ await the
> enif_select evidence pack (norn research, in flight) — the trigger model,
> stop lifecycle, and fd-reuse guards deliberately steal OTP's scar tissue
> rather than rediscovering it.

### 3.1 API sketch (subject to ⏳ evidence)

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
fn readiness_deregister(&self, token: ReadinessToken);
```

- **One-shot delivery** (⏳ confirm against enif_select): each arm fires at
  most one marker; the consumer re-arms per C4. One-shot removes the
  level-triggered storm class and makes marker idempotence trivial.
- `ReadinessToken` carries a **generation** minted at register time; stale
  events for a dead generation are dropped in the service, never delivered
  (§3.4).
- Delivery is exactly `enqueue_atom_message(pid, marker)` — the service adds
  no new delivery machinery and inherits C1–C3 verbatim.

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
- Drop-without-shutdown must not leak the thread (same posture as the
  NetKernel drop fix at 103e5fd).
- ⏳ OTP's ordering of poll-set teardown relative to schedulers.

### 3.4 fd-reuse and stale-delivery safety ⏳

The hazard: consumer closes fd N (or crashes), kernel recycles N for an
unrelated socket, a stale poll event for old-N delivers a marker to the old
registration's pid.

- Registrations are keyed by generation token, not fd number; events resolve
  through the live-registration table and stale generations drop.
- **Deregister-before-close is the documented consumer obligation** (the
  enif_select STOP lesson, ⏳ exact semantics); the service additionally
  guards: close-vs-poll races on a still-registered fd must produce at worst a
  spurious marker for the OLD registration (idempotent, C4), never a marker
  for an unrelated new one.
- ⏳ OTP's historical bugs here and the guard they settled on.

### 3.5 Process-death deregistration

Registrations owned by a dead pid are reaped: (a) lazily, on the next event
for that registration (C3 already makes delivery harmless; the reap bounds
table growth — a rule-5 answer, not just hygiene); and (b) eagerly if the
composition work exposes a process-exit hook the service can subscribe to
without new coupling. ⏳ OTP reaps via the owner-death path — confirm shape.

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
- Q1–Q4 lens answers in this doc match measured behavior.

## 4. Shape (a): embedder-owned reactor — fallback argument

> **Owner: Hermes Crumpet.** Held to the same "sleeping costs nothing" standard;
> must answer the same §3.3–§3.5 hazards from the liminal side, plus the
> N-embedders-N-reimplementations aggregate cost under lens Q2.

*(pending)*

## 5. Liminal consumer requirements

> **Owner: Hermes Crumpet.** Registration discipline (register-before-probe,
> generation keys, dereg on every termination path), bounded-drain-then-Wait
> slice shape, outbound tri-state + pump inbox-notify (the two shape-invariant
> liminal-side fixes this contract assumes), TempDir/store lifecycle
> interactions if any.

*(pending)*

## 6. Churn-driven acceptance tests (cross-repo)

> **Owner: Hermes Crumpet** (liminal exercises real sockets), with the beamr
> deterministic harness from §3.6 as the in-crate floor. Connect/write/
> disconnect churn beyond worker count; fd-reuse under real kernel recycling;
> shutdown-lifecycle under churn.

*(pending)*

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

**liminal half:** *(pending — Hermes; the ff8d863 → bb81724 → "busy-polls by
design" ledger chain, per his own account in-channel.)*

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
