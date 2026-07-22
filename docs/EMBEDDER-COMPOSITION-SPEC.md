# Embedder Composition Spec — Scheduler Services

**Status:** DESIGN OF RECORD — approved 2026-07-11 by both halves of the
certifying pair independently (Vesper Lynd: APPROVED, two advisories;
Waffles the Terrible: APPROVED, one condition + signing note; both folded at
this head, Q-A..Q-F ruled). Implementation unblocked per §11.
**Author:** Artemis Peach (beamr).
**Provenance:** docs/stack-review/AION-HOST-RESOURCE-INCIDENT-2026-07-11.md
(the required-work list §"Required Beamr work" and acceptance gates are this
doc's contract); impact surface mapped by GPT-5.6-Sol scout session
`b4fc19ad-788b-4896-b754-cd80ab3d0079` (envelope retained at
`~/.norn/delegations/claude-scout.FcBYTt`) — claims sourced from that pass are
marked **[scout]** and re-verified during certification; claims verified
directly by the author against main at dbd18d8 carry file:line only.
**Companion:** docs/READINESS-CONTRACT-SPEC.md — the readiness service (shape
(b)) is designed to be born inside this model (§3.8).

---

## 0. Principle

An embedder gets exactly the VM services it asks for, pays for nothing it
doesn't, can share what it wants to share, and can audit the whole bill.

This is not a request to shrink the full VM by changing defaults silently
(incident doc, "Required Beamr work" preamble). The standalone full runtime
remains one explicit profile away. What must die is the current shape: every
`Scheduler` constructs ~30 ancillary threads regardless of need — dirty CPU
pool sized `num_cpus` (`scheduler/mod.rs:691-695`, zero coerced to one at
`dirty.rs:188-190`), dirty IO pool of 10 (`mod.rs:696-703`), two unconditional
4-thread fallback IO rings on non-Linux (`mod.rs:764-773`,
`io/thread_pool.rs:92-117`), and two distribution runtimes even when
`distribution: None` (`mod.rs:731` → `unwrap_or_default()`;
DistSender `sender.rs:215-225`; NetKernel `distribution/mod.rs:74-87`) — with
NetKernel receiving a second, disjoint ConnectionManager (`mod.rs:780-787`).
Six schedulers in Aion ⇒ 207 of the process's 224 threads.

## 1. As-built service inventory (what exists today)

| # | Service | Threads today (macOS, 10-core) | Eager? | Zero-able? | Shutdown joined? |
|---|---|---:|---|---|---|
| 1 | Normal workers (`beamr-sched-{i}`) | configured (default = cores) | yes | no (0 coerced) | yes |
| 2 | Dirty CPU pool (`dirty-cpu-{i}`) | 10 | yes, incl. replay [scout] | no (`.max(1)`) | yes |
| 3 | Dirty IO pool (`dirty-io-{i}`) | 10 | yes, incl. replay [scout] | no | yes |
| 4 | File-IO fallback ring | 4 | yes (live) | no (0 ⇒ 4) | yes |
| 5 | Standard-IO fallback ring + process 0 | 4 | yes (live) | no | **no** — stops only at last Arc drop [scout] |
| 6 | Generic IO ring + completion bridge | 0 (None) / 4+1 (Some) | **already honest** — `config.io: None` is true absence | n/a | yes (bridge then ring) |
| 7 | DistSender tokio runtime (`beamr-dist-send`) | 1 | yes (live) | no | task aborted; **runtime worker not joined** — Drop spawns an unjoined helper thread (`sender.rs:185-200`) |
| 8 | NetKernel tokio runtime (unnamed) | 1 | yes | no | **no shutdown at all**; Drop spawns an unjoined helper (`distribution/mod.rs:60-70`) |
| 9 | Transient: one `dirty-complete-{pid}` OS thread per dirty call, unjoined | burst | n/a | n/a | never [scout: `execution/core.rs:1422-1451`] |

Passive (no threads, stays eager): JitProfiler/JitCache, TimerWheel, telemetry
counters, PgRegistry [scout]. Replay is partial prior art for absence: it
skips DistSender, generic IO, process 0, and uses disabled rings — but still
builds both dirty pools and NetKernel [scout].

**Naming defect:** rings 4, 5, and 6 all name workers
`beamr-io-thread-pool-{i}` (`io/thread_pool.rs:106-112`) — honest inventory
attribution is impossible until each service gets a distinct prefix (§5).

## 2. The ownership model

### 2.1 ServiceMode

One generic wrapper, used by every ancillary service:

```rust
pub enum ServiceMode<T> {
    Disabled,        // zero threads, zero fds; use is refused explicitly
    Owned(T),        // constructed by, inventoried under, and joined by this Scheduler
    Shared(Arc<T>),  // injected; used, inventoried as "shared", NEVER shut down here
}
```

`shutdown_if_owned` is the single teardown primitive: `Owned` stops and joins;
`Shared` drops the reference; `Disabled` no-ops. A service whose raw `T`
cannot safely encode disabled dispatch gets a facade that refuses before any
queue/suspension side effect (dirty pools are the motivating case, §3.2).

### 2.2 Construction API — additive, not breaking

Adding public fields to `SchedulerConfig` breaks exhaustive external struct
literals, and changing accessor return types is source-breaking [scout]. So:

- **`SchedulerServices`** (new struct, all fields `ServiceMode`-shaped with
  builder methods) + **`Scheduler::with_services(config, services)`** — the
  composition entrypoint.
- **Named profiles** as constructors: `SchedulerServices::full_runtime()`
  (today's behavior, explicitly requested) and `SchedulerServices::minimal()`
  (everything ancillary Disabled). Existing `Scheduler::new(config)` maps to
  a **legacy profile** preserving today's defaults for one release, with its
  eager costs documented in the rustdoc and the migration named in the
  CHANGELOG; the CLI switches to `full_runtime()` explicitly
  (`beamr-cli/src/main.rs:284-288` [scout]).
- Legacy knob mapping: `dirty_cpu_threads: None` = legacy default;
  `Some(n>0)` = Owned(n); `Some(0)` = **Disabled** (today it coerces to 1 —
  behavior change, recorded in §6).
- `distribution: None` = **absent** (§3.6) — type-compatible, behaviorally
  breaking, the point of the exercise. `SchedulerConfig::default()` therefore
  builds no distribution; the full-runtime profile adds it back explicitly.

### 2.3 Normal workers stay outside the model

Normal scheduler workers remain directly scheduler-owned with an explicit
count — they are the service the embedder is asking for by constructing a
Scheduler at all. The incident acceptance text treats them separately ("starts
only the explicitly requested normal workers"). `thread_count: Some(0)`
remains invalid (coerced today, `mod.rs:1170-1175`); a Scheduler with zero
workers is not a meaningful object. **Recommendation, flagged as open
question Q-A for review.**

## 3. Per-service specifications

### 3.1 Sizing defaults stop being host-shaped

`Owned` sizes are explicit. The full-runtime profile keeps today's sizes for
compatibility; nothing else infers from `num_cpus`.

### 3.2 Dirty pools — zero threads, refusal before side effects

The zero-thread hazard is load-bearing [scout, verified against
`execution/core.rs:705-763`]: dirty dispatch registers a **gated suspension**
and then submits. If a disabled pool retained a live bounded sender, the
submit succeeds, no worker ever runs the job, and the process stays parked
forever — C2's gated-suspension scope limit (READINESS-CONTRACT-SPEC §2.2)
means no message can wake it. Therefore:

- `Disabled` dirty pools hold **no channel and no JoinHandles**. Refusal
  happens **before** the suspension is registered and before the transient
  completion thread spawns (`execution/core.rs:1399-1451`).
- New `DirtySubmitError::Disabled`, surfaced as a distinguishable ExecError
  (service-unavailable), not folded into `Badarg` as `PoolUnavailable` is
  today [scout: `execution/core.rs:1454-1467`]. Exact variant naming: open
  question Q-B.
- Same refusal on `submit_task` (maintenance path).
- Gate: a disabled dirty call terminates the calling process with the
  explicit error while an unrelated process on the same scheduler makes
  progress (extends `tests/dirty_scheduler.rs:152-228` [scout]).

Precision (review note, 2026-07-11): the current code already withdraws the
suspension and exits the process on submit failure
(`execution/core.rs:739-742`), so refusal-before-registration is a
**tightening, not a live-bug fix**. The full ordering requirement, stated
normatively: refusal precedes suspension registration AND pool submit AND
bridge spawn. Today failure paths exist after each of those steps, and the
bridge-spawn-failure path (`execution/core.rs:1448`) returns `Err` after the
job was already submitted — that pre-existing edge is what this ordering
rule retires.

### 3.3 File-IO ring

`ServiceMode<CompletionRing>`. Disabled: the file facility is absent from
native services; submission refuses **before registering a suspension** with
a defined error rather than the current absent-facility `badarg`
(`native/context/mod.rs:1254-1268` [scout]) — exact refusal surface is open
question Q-B. Shared: two schedulers submit to one ring; completions are
already pid-routed; the ring is joined by its owner only.

### 3.4 Standard-IO ring + process 0

Disabled means: **no ring, no process 0 registered** (replay prior art:
`mod.rs:852-861` [scout]), and group-leader IO returns a defined error.
**Never** represent live-disabled with `ReplayDisabledRing`: the server's
completion poll loop would hang a normal worker forever
(`io/standard_io.rs:142-197` [scout]). The eager-process-0 pinning test
(`scheduler/tests.rs:516-538` [scout]) updates to profile-conditional.
Shutdown gains an explicit stop for the owned standard ring (today it stops
only at last-Arc drop — inventory-at-shutdown fails on current main [scout]).

### 3.5 Generic IO ring + bridge

Already honest absence (`config.io: None`, `mod.rs:714-730`). Work: add
`Shared`, keep the disabled arm byte-identical, make bridge/ring shutdown
conditional on ownership.

### 3.6 Distribution — one bundle, one ConnectionManager, honest absence

Distribution becomes **one coherent service bundle**: node identity + ONE
ConnectionManager (heartbeat-enabled) + DistSender + the NetKernel facade +
lifecycle hook registration. The second disjoint manager
(`mod.rs:780-787`) is **deleted** — as-built, `connect_node`/`nodes/0`
consult a table that listener/send/pg/control traffic never touches
(verified; also [scout: `distribution/mod.rs:90-127`]). One manager backs
everything. This is its own acceptance line (per Vesper's ruling):
**`distribution: None` ⇒ neither runtime exists, verified at BOTH former
construction sites.**

Absent-distribution semantics (the BIF surface keeps BEAM's standalone-node
behavior, grounded in existing tests [scout: `gate3_bifs`,
`scheduler/tests.rs:468-488`]):

| Surface | Absent behavior |
|---|---|
| `node/0`, local `node/1` | `nonode@nohost` (local Node identity is retained — it's passive) |
| `is_alive/0` | `false` |
| `nodes/0` | `[]` |
| `connect_node/1`, `disconnect_node/1` | `false` |
| remote `send/2` | `noconnection` (existing absent-facility arm, `gate3_bifs/mod.rs:188-212` [scout]) |
| `link_remote` | `RemoteLinkError::NoConnection` |
| `start_distribution_listener` | typed unavailable error (Q-B) |
| local pg | **fully functional**, zero propagation (PgRegistry is local; propagation already no-ops without DistSender, `pg_propagation.rs:35-69` [scout]) |
| connection-event subscribe / control-handler registration | conditional; absent ⇒ typed refusal (`connection_lifecycle.rs:12-29` [scout]) |

Owned distribution teardown is rewritten (§4): stop listener/connection/
heartbeat/drain tasks, then **synchronously join both runtime workers**
before shutdown returns — replacing today's abort-task-only DistSender stop
(`sender.rs:345-349` [scout]), NetKernel's absence of any shutdown, and both
Drop impls' unjoined helper-thread runtime drops (`sender.rs:185-200`,
`distribution/mod.rs:60-70`).

Shared distribution is deferred: a ConnectionManager captures atom table,
resolver, cookie, node identity, subscribers, control handler, and runtime
handle [scout: `connection.rs:328-376`]; cross-scheduler injection needs
identity/compat validation this round doesn't require. `Shared` for
distribution is out of scope v1 — recorded as future work, not silently
absent. (Two schedulers sharing IO rings/dirty pools IS in scope and gated.)

### 3.7 Heartbeat / net-tick

Stays a property of the owned distribution bundle (async task per connection,
no OS thread, `connection.rs:91-115` [scout]) — inventoried as a task-class
line, not a thread line.

### 3.8 The idle tick — the VM's own unsigned idle cost

Found by liminal's Sol scout (session 02468176), verified against source:
an idle normal worker parks on a **5ms `wait_timeout`**
(`execution.rs:706-724`, `park_thread`), i.e. up to ~200 wakes/sec/worker at
complete idle, forever — each wake running a full loop iteration (steal
attempts, ring poll). Aggregate scales with total workers across all
schedulers in the process. This is rule 2 applied to the VM's own heartbeat:
an idle cost with no pinned ceiling, no test, and no sign-off. The tick
exists because two event sources are POLLED by loop iterations rather than
notifying `wake_condvar`: timer-wheel expiry and IO-ring completions
(`execution.rs:339-417`).

Remedy options, decision routed as **Q-F**:
1. **Deadline-aware idle wait (recommended):** park until
   `min(next timer-wheel deadline, ∞)` and make completion delivery notify
   the condvar — tickless idle; a fully idle worker with no armed timers
   sleeps indefinitely. Correctness burden: every event source that today
   relies on the 5ms poll must be enumerated and given a wake edge (the same
   class of analysis the readiness contract just did for connections).
2. **Adaptive backoff:** cheap, bounded improvement (5ms → grows to e.g.
   500ms when idle); keeps the poll crutch, shrinks the floor ~100×; risk:
   latency cliffs on the polled paths.
3. **Documented bound:** keep 5ms, pin it with a test and the certifying
   pair's signature. Honest, but leaves fan noise on the table.

Whichever lands, T1 (liminal's soak) is a **delta assertion** against the
VM's measured floor (certifying-pair ruling, 2026-07-11), so this item can
move independently without invalidating a signed T1. The floor itself gets a
Q1 line and a permanent assertion either way (§9).

**Ruling (Q-F, certifying pair, 2026-07-11): HYBRID.** Commit 1 pins the
5ms floor as a signed bound (option 3 — immediate rule-2 compliance, §7
methodology). Tickless (option 1) proceeds as its **own named commit** in
this workstream, gated on a wake-edge enumeration section that receives the
same review treatment the readiness contract got. If the enumeration finds
an event source that cannot get a clean wake edge, adaptive backoff
(option 2) is the recorded fallback — decided in the open, not a silent
compromise. **Signing note (Waffles the Terrible):** what the pair signs is
the FORMULA and its per-host instantiation — wakes/sec/worker × total
workers across all schedulers in the process — not a bare "5ms"; the
aggregate is where the original sin lived, and a per-worker number without
its multiplier is half a bound. The instantiation names its inputs from the
inventory API itself (total workers = what `service_inventory()` reports,
not what a config file claims — Vesper Lynd's sharpening), so the signed
number and the enforcement mechanism cannot drift apart.

### 3.9 Readiness service (companion spec)

READINESS-CONTRACT-SPEC §3 shape (b) — now CERTIFIED as the shape of
record (certifying pair, 2026-07-11) — instantiates this model on day one:
`ServiceMode<ReadinessService>`, Disabled by default everywhere except
profiles that request it, inventory line, its own spec's §3.6 gates. It is
the model's first born-composed service and its lens answers are already
written.

**Poll-thread multiplicity (certification condition 1):** the signed Q2
bound must state the multiplicity explicitly, because per-scheduler vs
shared is a 6× spread on the incident host. Ruling proposed by this spec:

- `Owned` readiness = one poll thread **per scheduler that owns one** — the
  simple default for single-scheduler embedders.
- `Shared` readiness = one poll thread **per process**, injected into every
  scheduler that consumes it — registrations carry the delivering
  scheduler's identity so markers route home. **Multi-scheduler embedders
  (the aion shape) get Shared in their profile; the full-runtime profile
  documents which it picks.**
- The inventory line reports the multiplicity honestly either way
  (`mode: Owned` on one scheduler vs `mode: Shared` on N of them, one
  thread total), so the signed bound is auditable per deployment: N
  schedulers ⇒ N poll threads under all-Owned, exactly 1 under Shared.

**Shared-delivery gate (certification condition, Waffles the Terrible,
2026-07-11):** the routing claim above ("registrations carry the delivering
scheduler's identity so markers route home") gets its own acceptance gate —
the readiness §3.6 gate set was written when the service faced one
scheduler. New gate: two schedulers on one Shared poll thread, markers
provably delivered to the correct scheduler's process, **including after one
scheduler shuts down** — the surviving scheduler's registrations keep
routing correctly and zero markers route to the dead one. This is the same
identity machinery as §4 step 3 / assertion 5, tested from the delivery
end; it joins the readiness spec's §3.6 gate list when the service
implementation lands (contract §7 sequencing unchanged).

## 4. Shutdown — ownership-ordered, join-complete

Deterministic order (rewriting `execution.rs:69-93`):

1. Stop intake: refuse new registrations/submissions on every owned service.
2. Stop completion/lifecycle tasks (bridge, readiness poll thread, listener
   accept tasks if scheduler-owned — Q-C, dist drain/heartbeat).
3. **Deregister this scheduler's live registrations from every `Shared`
   service** (readiness fds, shared-ring pending completions) before joining
   workers — releasing a handle (step 6) does not remove registrations, and
   a shared service still holding registrations that route markers or
   completions to a dead scheduler is exactly the fragility class the
   generation tokens exist for (review advisory 2, 2026-07-11; the
   VM-side complement of the consumer's per-connection dereg discipline).
4. `shutdown_if_owned` each backend: dirty pools, file ring, standard ring
   (new), generic ring, distribution bundle **including synchronous runtime
   worker joins**.
5. Signal + join normal workers.
6. `Shared` handles: released, never stopped.

Gate (incident doc): *shutdown joins exactly the resources owned by that
scheduler* — asserted by the inventory probe (§5) immediately after
`shutdown()` returns: zero beamr-attributed threads remain for an all-Owned
scheduler; a co-resident scheduler's shared services are untouched.

## 5. Thread-inventory API

```rust
pub struct ServiceInventoryEntry {
    pub service: &'static str,        // "dirty-cpu", "file-io-ring", ...
    pub mode: ServiceModeLabel,       // Disabled | Owned | Shared
    pub instance: ServiceInstanceId,  // process-wide identity of the underlying
                                      // service instance: two entries (from any
                                      // schedulers) with equal `instance` ARE the
                                      // same service — makes Shared dedup mechanical
    pub configured: usize,            // requested worker count
    pub actual: usize,                // live OS threads right now
    pub thread_names: Vec<String>,    // exact OS names, service-distinct
    pub fd_classes: Vec<&'static str> // "io_uring", "listener", "poll", ...
}
pub fn Scheduler::service_inventory(&self) -> Vec<ServiceInventoryEntry>;
```

`ServiceInstanceId` is an opaque process-unique token minted at service
construction (Owned and Shared alike); cloning a `Shared` handle propagates
the token, so co-resident schedulers report the same identity for the same
underlying service and the Q2 process-wide dedup (§9) is a plain group-by.

- Every ring gets a **service-distinct thread-name prefix** (fixing the
  three-way `beamr-io-thread-pool-*` collision); NetKernel's runtime gets a
  name (today: tokio default, unnamed [scout]).
- Transient classes are inventoried as **policies** with counters, not
  threads: the per-dirty-call `dirty-complete-{pid}` burst thread [scout] is
  reported (and refused when dirty is Disabled); replacing it with a shared
  completion mechanism is follow-up work, not this round (Q-D).
- A **platform-aware OS-thread probe** test helper (macOS: named-thread
  deltas; Linux: plus `beamr-io-uring` and ring fds) — none exists today
  [scout]; it is what makes lens Q3 mechanical.

**Permanent negative assertions (rule 1, never retire):**
1. Minimal profile: exact expected thread set, byte-named, nothing else.
2. Every Disabled service: zero threads, zero fds, refusal is typed.
3. `distribution: None`: no runtime exists at either former construction
   site (the two-site acceptance line).
4. Post-shutdown: zero owned beamr-attributed threads (OS probe).
5. Two schedulers sharing a ring/pool: no double-join, no cross-shutdown;
   shared service survives one scheduler's death **with zero registrations
   still targeting the dead scheduler** (readiness fds, shared-ring pending
   completions) — survival isn't the bar, no-delivery-to-the-dead is
   (review advisory 2, 2026-07-11).
6. `service_inventory()` agrees with the OS probe on every profile in the
   test matrix; **process-wide**, the deduped aggregate — Owned entries plus
   each distinct Shared `instance` counted ONCE — agrees with the OS probe
   across co-resident schedulers (review advisory 1, 2026-07-11).

## 6. Compatibility & behavior-change ledger

- Additive API (`with_services`, profiles, `try_*` accessors); existing
  accessors that can't represent absence (`distribution_config()`,
  `distribution_connections()`, dirty pool refs, `mod.rs:1091-1138`) gain
  `try_` variants; the old ones keep working for Owned/legacy profiles and
  get deprecation notes — removal is a later major (no `#[allow]`s: the
  deprecations must not trip our own `-D warnings` gate, staging plan to be
  proven in the first commit).
  - **Amendment (pair-ruled at commit 2): the keep-old-working PRINCIPLE.**
    Keep-old-working applies where the old signature can still be honestly
    satisfied in every constructible mode. Where absence makes the old
    signature a lie — a `&T`-returning accessor on a scheduler whose service
    can now be Disabled — the break is mandatory, loud, and `try_`-named:
    both candidate shapes are compile-time breaks, but the renamed method
    fails at the call site with a replacement name that teaches the new
    contract, while a same-name `Option` return fails one step downstream
    and teaches nothing. The old name stays discoverable via `#[doc(alias)]`
    on the `try_` variant. Applied at commit 2:
    `dirty_cpu_pool()`/`dirty_io_pool()` (returned `&DirtyPool`; a Disabled
    pool has nothing to return, and the no-panic rule forbids the only
    signature-preserving out) are REPLACED by
    `try_dirty_cpu_pool()`/`try_dirty_io_pool()`. Commits 3–5 apply the same
    test to each accessor they touch instead of re-routing the question.
- **Behavior changes, stated loudly in CHANGELOG:** `distribution: None`
  becomes absent (default configs lose empty-resolver distribution; CLI opts
  into full-runtime); `dirty_*_threads: Some(0)` becomes Disabled instead of
  1.
- beamr-wasm: unaffected (cooperative scheduler, no threaded composition
  [scout: `beamr-wasm/src/lib.rs:59-70`]).
- `threads` without `net` remains unsupported-but-Cargo-expressible
  (pre-existing); this work must not deepen the mismatch [scout] — new
  distribution absence arms are `net`-gated the same way as today (Q-E).
- Pinned tests updating: default-distribution resolver access, default dirty
  counts, eager process 0, SharedState leak regression [scout, §"Tests"] —
  each moves to the explicit profile it was implicitly assuming.

## 7. Acceptance gates (supersets the incident doc's list)

**Soak-gate methodology (certification condition 2):** every soak/idle
gate in this spec states T1-grade methodology — measurement duration,
sampling source, host state, and a baseline delta rather than an absolute
ceiling — so certified numbers are reproducible, not one-shot readings.
This applies to the §3.8 tick-floor measurement, the minimal-profile idle
assertion, and the readiness service's enabled-idle soak.

All eight incident-doc gates, plus: the two-site distribution absence
assertion; the gated-suspension-safe dirty refusal (§3.2 gate); the
standard-IO no-hang gate (disabled ⇒ no process 0, no group-leader hang); the
shutdown-inventory gate (§4); the shared-service no-double-join gate; the
feature-matrix compile/test sweep (default; threads+net minus jit/telemetry;
cooperative wasm; telemetry) [scout]. Every gate lands as a permanent test,
not an incident-closure checklist.

## 8. How the original shipped (rule 3)

No single decision shipped 30 threads. Each service was added defensibly:
dirty pools mirrored BEAM's dirty schedulers; the fallback rings served real
file/stdio needs on macOS; the distribution runtimes made networking work;
`unwrap_or_default()` was the path of least resistance for a config nobody
set; the `.max(1)` coercion "protected" against a footgun. The missing
control was the aggregate: no ceiling, no test, no sign-off ever considered
the SUM, because no review lens forced the question (rule 5's Q2 now does)
and no inventory made the sum visible (§5 now does). The second
ConnectionManager shipped the same way — locally reasonable, never reconciled
globally. The gates that would have caught it: assertion 1 (minimal-profile
exact thread set) fails the moment any service goes eager; assertion 3 fails
on the day `distribution: None` builds a runtime; lens Q2 blocks the review
that adds service #31.

## 9. Idle/resource-cost lens answers (canonical v1.1)

- **Q1 (idle cost):** a minimal-profile scheduler: N requested normal
  workers, zero ancillary threads, zero fds beyond stdio the embedder gave
  it, zero disk/fsyncs — **plus, honestly stated: the idle tick (§3.8), up
  to ~200 wakes/sec/worker — signed as a bound in commit 1 per the Q-F
  ruling, tickless as the named follow-on.** The tick is this doc's own
  by-design cost and gets the full Q4 treatment; a "parked worker costs
  nothing" claim is NOT made until the tick is tickless. Pinned by assertion 1 plus a new idle-wake-rate assertion.
  Full-runtime profile: today's budget, now explicit, enumerated by
  `service_inventory()` and asserted against the OS.
- **Q2 (aggregate ceiling):** the inventory IS the enforcement: per
  scheduler, aggregate = Σ that scheduler's inventory entries;
  **process-wide, aggregate = Σ Owned entries + each distinct Shared
  service-instance counted ONCE** (Σ over all entries would count a shared
  4-thread ring N times across N schedulers — review advisory 1,
  2026-07-11). The `instance` identity on each entry (§5) makes the dedup
  mechanical, and assertion 6 asserts the deduped aggregate against the OS
  probe. Sharing exists precisely so N schedulers stop paying N× (one
  ring/pool serves many). No service infers size from the host. The
  readiness service's Q2 story is in its own spec.
- **Q3 (quiescence test):** assertions 1–6 (§5), all new with this diff, all
  mechanical against `service_inventory()` + the OS probe, all failing if Q1
  or Q2 were wrong.
- **Q4 (by-design costs):** the full-runtime profile deliberately keeps
  today's thread budget for standalone use — bound: the §1 table, now
  enforced by inventory assertion; test: assertion 6 on the full-runtime
  profile; sign-off: the certifying pair (Vesper Lynd + Waffles the
  Terrible), Tom briefed. The legacy `Scheduler::new` profile carries the
  same numbers for one release as a migration bridge — same three citations.

## 10. Open questions — RULED (certifying pair, 2026-07-11; both passes
independent, verdicts merged)

- **Q-A — RULED: CONFIRMED.** Normal workers stay outside the ServiceMode
  model (§2.3). A zero-worker scheduler is meaningless and the park
  semantics assume workers exist.
- **Q-B — RULED: recommendation stands.** Typed Rust errors at the embedder
  API (`DirtySubmitError::Disabled` + service-unavailable ExecError, and
  the analogous surfaces for file/standard IO and the absent listener);
  existing atoms preserved at the BIF surface; no new BEAM-visible atoms
  this round.
- **Q-C — RULED: agreed.** Scheduler-owned listener when distribution is
  Owned, joined in §4 step 2 (today caller-owned, unjoined —
  `mod.rs:1126-1138`).
- **Q-D — RULED: agreed.** Refuse-when-disabled + inventory-as-policy this
  round; shared completion mechanism as a named follow-up. The burst-thread
  policy carries a counter so lens Q2 sees it.
- **Q-E — RULED: agreed.** `threads`-without-`net` documented as
  unsupported; this work must not deepen the mismatch.
- **Q-F — RULED: HYBRID** (full text in §3.8). Commit 1 signs the 5ms floor
  as a bound (formula + inventory-sourced per-host instantiation); tickless
  proceeds as its own named commit gated on a contract-grade wake-edge
  enumeration; adaptive backoff is the recorded fallback if any source
  can't get a clean wake edge. T1 stays a delta assertion throughout.

## 11. Sequencing

1. ~~This doc through review (Vesper) + certification (pair)~~ — DONE
   2026-07-11: approved by both halves independently; advisories and
   conditions folded at this head.
2. Commit 1: `ServiceMode` + inventory API + OS probe + assertions on
   CURRENT behavior (pins the as-built budget; proves the probe) **+ the
   signed 5ms idle-tick bound per the Q-F ruling** (formula +
   inventory-sourced instantiation, §3.8).
3. Commit 2: dirty pools (zero-thread + refusal) — the gated-suspension
   safety is the scariest arm, it goes first with its gate.
4. Commit 3: IO rings (file/standard/generic) + naming prefixes + process-0
   conditionality.
5. Commit 4: distribution bundle — single manager (delete second), honest
   None, teardown rewrite, runtime joins.
6. Commit 5: `with_services` + profiles + CLI migration + accessor staging.
7. Commit 6: readiness service lands against this model per its own spec's
   sequencing (after the joint contract's §2.5 pinning suite is green on
   main), carrying the Shared-delivery gate (§3.9).
8. Named follow-on commit (this workstream, not this round's tail): tickless
   idle per the Q-F ruling — wake-edge enumeration section first, reviewed
   at contract grade; adaptive backoff recorded as fallback.
Each commit through the full gate bar + norn review passes; the branch lands
by review, never exploratory on main.
