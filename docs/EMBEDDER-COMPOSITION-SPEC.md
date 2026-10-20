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

#### 3.8.1 Wake-edge enumeration (B-181 R1; for Hermes's review before R2's code)

Read from `crates/beamr/src` at beamr main `ee8a41ef` (the scheduler, IO
and timer sources are unchanged from `aeabb126`, the 0.21.0 commit B-181
cites). Every file:line below was opened and read. This section lists every
event that can make work runnable on an idle normal worker, what it does to
`wake_condvar` today, and the edge it gets under B-181.

**Why every source needs an edge, not only a notify.** A worker runs its
checks in `scheduler_loop` (`scheduler/execution.rs:428-471`): shutdown,
its injection queue, worker 0's timer tick and file-IO drain, its share of
`wait_set.woken`, its run queue, then a steal. It takes the `wait_set` lock
only afterwards, in `park_thread` (`execution.rs:994-1020`, lock at `:1000`,
`wait_timeout(IDLE_PARK_TIMEOUT)` at `:1012`), and it waits without checking
anything again. A wake that lands after the checks and before the wait
notifies a condvar nobody is waiting on yet, so the wake is lost. Today the
5 ms timeout (`:982`) recovers it. With no timeout, the worker would sleep
forever. Taking the lock does not help on its own, because the worker does
not re-check under it. So every source below gets the same edge.

**The edge E (R2).** Every wake source takes the `wait_set` lock, advances a
wake sequence in `SharedState`, then calls `notify_all` on `wake_condvar`.
`park_thread` reads the sequence before the worker's checks, and waits only
while, under the `wait_set` lock, the sequence is unchanged. A source that
already pushes to `wait_set.woken` under that lock advances the sequence in
the same critical section. A source that notifies without the lock today
takes it to advance. "Inherits E" below means the source reaches a
primitive in the table that carries E, so it needs no change of its own.

A `wake_process` that finds the pid absent from `wait_set.waiting` notifies
no one, today and under B-181. That is correct. The process is executing,
or it is in its park gap, so no idle worker owns it. The Wait arm's
post-registration recheck (`execution/core.rs:92-178`) and the Suspended
arm's fallback (`core.rs:179-237`) schedule it on the worker that is
parking it.

**Invariant (Hermes's review, 21 Oct).** Every wake under B-181 is either
`notify_all` on `wake_condvar` or a wake of worker 0 by name. No source uses
`notify_one`, and no later targeted wake may reach some worker other than
worker 0. The reason is that only worker 0 ticks the timer wheel and drains
file-IO completions (`execution.rs:433-439`). If a single notify reached any
other worker, that worker would find nothing of its own and park again.
Worker 0 would stay parked with no bound, and a deadline or a completion
could be stranded with it. Any narrowing of the herd (see "wakes per
message", B-181 R3) must keep this invariant.

| # | Event that makes work runnable | Where it happens | `wake_condvar` today (lock) | Edge under B-181 |
|---|---|---|---|---|
| W1 | A parked process is woken by a message or signal: the common primitive `wake_process` | `scheduler/execution.rs:510-536`, with its test twin under `cfg(all(test, feature = "readiness"))` at `:538-573`. Waking: `:531-535` / `:564-572` | `notify_all`, under the `wait_set` lock, only when the pid moves from `waiting` to `woken` | E inside the same critical section. Both copies change, because the twin is a second copy of the same code |
| W2 | Local process-to-process send | `SchedulerLocalSendFacility::send_local`, `scheduler/supervision_integration.rs:928-1083` (wake at `:1081`) | through W1 | inherits E |
| W3 | Embedder wakes and sends: `Scheduler::wake_process` and `wake_notifier` (mailbox senders call the closure, `mailbox/mod.rs:316-324`), `enqueue_atom_message`, `send_to_mailbox` | `execution.rs:23-31`; `scheduler/mod.rs:2507-2513`, `:2534-2542` | through W1 | inherits E |
| W4 | Link exit signal to a trapping process; the `exit/2` BIF; a remote exit signal | `process_exit_signal`, `supervision_integration.rs:332` (wakes `:416`, `:467`); `exit_signal`, `:2527-2585` (wakes `:2558`, `:2580`); `process_remote_exit_signal`, `remote_supervision.rs:57` (wakes `:90`, `:110`) | through W1 | inherits E |
| W5 | Monitor `DOWN` delivery; `ETS-TRANSFER` delivery; a group-leader IO message | `deliver_single_down_and_wake`, `supervision_integration.rs:526-540` (`:539`); `deliver_ets_transfer`, `:138-180` (`:178`); `IoMessageFacility::send_message`, `:1091-1117` (`:1116`) | through W1 | inherits E |
| W6 | A spawn is injected into a worker's queue | `enqueue_spawn_with_trap_exit`, `scheduler/spawning.rs:438-508`. Non-trap-exit path: `inject_queues[index].push` then `notify_all` at `:503-504` | `notify_all` with **no lock** | E: push, then lock, advance, notify. The target worker drains its queue at the top of the loop (`execution.rs:432`, `:481-488`) |
| W7 | Trap-exit spawn, placed straight into `woken` | `spawning.rs:490-492` | `notify_all`, under the lock | E in the same critical section |
| W8 | Pending spawns materialised by a code purge | `drain_pending_spawns`, `spawning.rs:511-527` (`:523-525`), called from `module_management.rs:88`, `:101` | `notify_all`, under the lock | E in the same critical section |
| W9 | BIF spawns (`spawn`, `spawn_link`, `spawn_monitor`, lambda, native): all go to worker 0's `woken` | `spawn`, `supervision_integration.rs:1341` (`:1412-1416`); `spawn_lambda`, `:1453` (`:1516-1520`); `spawn_native_flagged`, `:1619` (`:1665-1669`); `insert_and_wake`, `:2001-2014` (`:2009-2013`) | `woken` pushed under the lock, then `notify_all` **after the lock is released** | E: advance inside the existing critical section, notify after |
| W10 | A timer fires: worker 0 ticks the wheel | `tick_timers`, `scheduler/timer_integration.rs:93-96`, then `expire_timers`, `:111-132`. ReceiveTimeout: `mark_fired_receive_timer`, `:327-346`. Deliver: `deliver_term_to_mailbox` then W1, `:126-128` | Firing notifies: `:341-345` under the lock, Deliver through W1. **Reaching** the deadline is polled: nothing wakes worker 0 when a deadline arrives, only its 5 ms timeout | Worker 0 parks until the earliest armed deadline (`TimerWheel::earliest_deadline`, `timer.rs:235-238`, exists already) and parks with no bound when none is armed. Fires inherit E through `mark_fired_receive_timer` and W1 |
| W11 | A timer is **armed** (not named in the brief). Without an edge here, a parked worker 0 would sleep past a new, earlier deadline | Receive-after: `register_receive_timer`, `timer_integration.rs:40-57`, from the Wait arm `core.rs:85`. Native timers: `ProcessContext::schedule_timer` and its variants, `native/context/mod.rs:1074-1150`; `NativeProcess::send_after`, `native/native_process.rs:353-368`. Raw public access: `Scheduler::timers()`, `scheduler/mod.rs:2124-2126`. All of these funnel into `TimerWheel::schedule_reserved_at`, `timer.rs:142-175` | none | E when the new deadline is earlier than the one worker 0 waits for. **Placement:** the edge must sit in the wheel, as a notifier the wheel holds and calls from `schedule_reserved_at`. The native contexts hold only `Arc<Mutex<TimerWheel>>` (`core.rs:1497`, `execution/native_slice.rs:78`, `supervision_integration.rs:628`) and cannot reach `wait_set`. An embedder scheduling through `timers()` is covered only this way. **Lock order:** the edge takes `wait_set` while holding `timers`, so `park_thread` must read the deadline before it takes `wait_set` and never while holding it. The sequence covers an arming in that gap |
| W12 | A timer is cancelled | `TimerWheel::cancel` / `cancel_at`, `timer.rs:191-199`; liminal cancels through `timers()` (`liminal-server/src/server/connection/supervisor/runtime_close.rs:42-46`) | none | No edge. A cancelled earliest deadline makes worker 0 time out once with nothing to fire, then park again for the next deadline. This is harmless, but it is a wake that no fired deadline accounts for (see the bound note below the table) |
| W13 | A file-IO completion is **published** by the ring. This is the polled source | Non-Linux: `ThreadPoolRing` `worker_loop`, `io/thread_pool.rs:240-258` (send `:250-251`). Linux: `IoUringRing` `drain_cqes`, `io/uring.rs:455-474` (send `:470`), and `complete_error`, `:532-545` (`:539`). Linux with io_uring unavailable: `FailedRing::submit`, `io/ring.rs:205-222` (push `:220`), which completes on the **submitting** worker, possibly not worker 0 | none. Worker 0 polls `poll_completions(0 ms)` on every loop iteration (`execution.rs:575-606`, poll at `:581-584`) | E at the publication point in **all three** implementations, through a notifier the ring holds. The scheduler installs it when it builds its Owned file ring (`scheduler/mod.rs:1549-1558`). A Shared file ring is refused by `validate()` (comment at `mod.rs:1540-1544`), so each ring has exactly one consumer. Note: on Linux the ring thread reaps completions on its own 10 ms tick (`uring.rs:23`, `:222`), so publication there is still polled inside the ring. That is outside this brief, and the edge at publication is clean either way |
| W14 | A file-IO completion is **delivered** to its process: worker 0's drain | `deliver_file_io_result`, `execution.rs:617-628`; active-mode UDP and TCP, `:699`, `:774`, `:800`; the orphan path at registration, `supervision_integration.rs:1200-1210` | through W1 | inherits E |
| W15 | A generic-IO completion: the bridge thread delivers it | The bridge poller `io/bridge.rs:92-97` (its own thread, 100 ms poll); `dispatch_completion`, `:130-145`; `IoWakeTarget::wake_with_io_result`, `scheduler/mod.rs:2547-2561` (W1); `send_io_message`, `:2563-2584` (W1, or for process 0: `woken` push and `notify_all` under the lock, `:2577-2581`) | through W1; `:2577-2581` under the lock | inherits E; `:2577-2581` gets E in its critical section. (This is the `mod.rs:2580` B-181 cites, and it is the standard-IO pid's delivery, not shutdown) |
| W16 | A standard-IO completion | Consumed only inside `StandardIoServer::wait_for_completion`, `io/standard_io.rs:216-224` (10 ms `POLL_TIMEOUT`, `:19`), which runs inside a slice. Its only caller, `run_available` (`:91-96`), has **no caller in the repository** (searched: not found) | n/a | No normal-worker wake source, so no edge. Process 0 is woken by its messages through W1 and W15 |
| W17 | A dirty-call completion resumes the suspended process | The completion bridge thread, `execution/core.rs:1549-1586`: `publish_suspension_result`, then `resume_suspended` at `:1583`, which is `timer_integration.rs:389-414`; embedder `wake_with_dirty_result`, `execution.rs:398-408` | `resume_suspended` calls `notify_all` under the lock, `timer_integration.rs:403-410` | E in `resume_suspended`'s critical section. (The replay arm, `core.rs:1481-1493`, runs on the worker itself, which is busy) |
| W18 | A host-await completion or a hook resume | `wake_with_result`, `execution.rs:363-376`; `wake_with_result_for`, `:385-394` (both W1); `resume_process`, `:51-67`, which calls `resume_suspended` (W17) | through W1 and W17 | inherits E |
| W19 | The Suspended arm's fallback unpark (published before registration) | `execution/core.rs:228-237` | `notify_all`, under the lock | E in the same critical section. It runs on the busy worker, which is its own target, but it keeps the rule "every push to `woken` advances the sequence" without exceptions |
| W20 | Readiness service delivery | The poll thread waits in `poll(&mut events, None)`, `scheduler/readiness/core.rs:379`: already tickless. Delivery is `readiness/core/delivery.rs:66-68`, then `deliver_readiness_marker`, `scheduler/mod.rs:760-770` (W1) | through W1 | inherits E. The poll thread is unchanged (Boundaries) |
| W21 | Distribution delivery: a remote message to a local pid | The control-frame handler, `supervision_integration.rs:80-112`, then `ControlDelivery::deliver_payload`, `:867-897` (wake at `:895`); remote exit signals are W4 | through W1 | inherits E. No missing edge was found, so the distribution manager is unchanged |
| W22 | Shutdown | `Scheduler::shutdown`, `execution.rs:126-131` (stores the flag, then `notify_all` at `:131`); replay failure, `fail_replay_timer`, `timer_integration.rs:78-89` (`:86-88`) | `notify_all` with **no lock** | E: store the flag, then lock, advance, notify. `park_thread` already checks `shutdown` before it takes the lock (`execution.rs:997-999`). The sequence closes the gap between that check and the wait |
| W23 | **Work stealing**: work is queued on a busy worker while another worker is idle | `try_steal`, `scheduler/steal.rs:26-53`, called at `execution.rs:451-469`. Local queue pushes: `execution.rs:485` (injected), `:957` (woken), `core.rs:83` (Requeue after a slice), `core.rs:175` (Wait self-wake) | none, and no push takes `wait_set`. An idle worker finds stealable work only on its 5 ms timeout | **No clean edge**: see the open decision below |
| W24 | **Replay scheduler park** (`shared.replay_mode`) | `execution.rs:442-449` (`next_replay_pid`, `:928-939`). Worker 0 consumes one recorded `TimerExpiry` per loop iteration (`execution.rs:434-435`, then `timer_integration.rs:59-76`, then `replay/driver.rs:422-433`). The cursor advances in `next_schedule`, `driver.rs:384-403`, and `next_timer_expiry`, through `advance_cursor`, `:486-488` | none. The `replay/` module has no notify. A worker whose recorded turn arrives finds out on its 5 ms timeout | E on every cursor advance, after the driver lock is released, plus W6-W9 and W22. **What determinism needs:** the log, never the clock, decides when a replay worker leaves its park. Worker 0 leaves at once unless the cursor holds a `Schedule` for another worker. It never waits while the cursor holds a `TimerExpiry`, the end of the log, or any other kind, all of which its next tick consumes or refuses exactly as today. Worker i > 0 leaves only when the cursor holds its own `Schedule` whose pid is in the process table. A spurious wake re-checks the cursor under the sequence and parks again without running a loop iteration, because every worker-0 iteration consumes a log event |
| — | (Not a worker) the host waiter `run_until_exit` | `execution.rs:188-203`, which waits on `wake_condvar` with a 10 ms timeout | No exit notifies it. It polls | Out of scope. It is listed because `notify_all` on `wake_condvar` reaches it too, and because its 10 ms wait is a fixed wait of its own |

**Source without a clean wake edge: W23, work stealing. Open decision, for
this source only (Q-F ruling's recorded fallback: adaptive backoff).** No
push onto a worker's run queue notifies anyone. Under the 5 ms timeout an
idle worker steals within 5 ms. Under an unbounded park it would not steal
until some unrelated E woke it. The process does not hang, because the owner
worker runs its own queue. But the work stays serialised behind the owner,
and that matters most when the owner blocks inside a slice: a synchronous
distribution send is bounded only by `WRITE_TIMEOUT`
(`block_on_distribution_send_with_write_deadline`,
`supervision_integration.rs:803-830`), and an embedder hook or native may
block too. Three options, decided in the open by the review:

1. **A surplus edge.** A push that leaves a worker's queue holding more than
   the process it is about to run, while the count of parked workers (kept
   under `wait_set`) is non-zero, runs E. The cost is one atomic read per
   push on the hot path, plus a lock and a herd wake only when there is
   both surplus and an idle worker. **Decided: this option**, with the
   idle-count ordering below in place of "the count of parked workers".
2. **Adaptive backoff for stealing only (the Q-F fallback).** A worker other
   than worker 0, whose last steal found nothing, parks with a timeout that
   grows from 5 ms to a cap, and parks with no bound only when every queue
   was empty at its last look. This keeps a poll, and Tom's rule against
   waits of a set length applies to it.
3. **No edge.** Accept serialisation behind a blocked owner as the cost of
   zero idle wakes, and state it in the R4 amendment.

**Bound for R4, re-signed.** Waffles re-signed the bound on Cambium on
21 October 2026 at 04:49 (Melbourne), post `c38bfbeb`, replacing the 04:31
line: zero idle wakes for a scheduler with no armed timer and no IO in
flight; with armed timers, one wake per deadline worker 0 reaches, fired or
cancelled; in replay, one per cursor advance; over the total workers
`service_inventory()` reports; enforced by R3's tests. R3's measurement
counts each wake under its cause (fired, cancelled, cursor), so "idle" is
zero because every wake is accounted for.

#### 3.8.2 Conditions from the review, binding on R2 and R3

Hermes approved this enumeration on Cambium on 21 October 2026 at 04:50
(Melbourne), post `1440ba67`, with the conditions below, C1 completed from
its second reading (04:54, post `5ecebafd`). R2's code starts
only with them written here.

**C1. W23's surplus edge uses an idle count, raised before the sequence
read.** A count of *parked* workers loses a wake: a worker that has made its
steal attempt but not yet counted itself parked is read as zero by a
concurrent push, gets no E, and then parks with no bound while the surplus
waits. So each worker raises an idle count with `fetch_add(SeqCst)` and then
issues `fence(SeqCst)` *before* it reads the wake sequence, which is before
its checks and its steal. It lowers the count on every way out of that
window: when it leaves the park, and equally when it finds work without
parking; a count left high would make every surplus push wake the herd
for ever, against the signed bound. A pusher pushes, then issues
`fence(SeqCst)` and reads the idle count, and runs E when the count is
non-zero and its queue holds surplus. E's advance of the sequence is a
Release store (or is made under the `wait_set` mutex), and the worker's
read of it is Acquire or stronger. The ordering argument, in the single
total order the two SeqCst fences give (a Dekker pattern: it holds only
with a SeqCst fence on both sides; on ARM64 an Acquire load may compile to
`ldapr`, which can move ahead of an earlier SeqCst RMW's store, and the
worker's fence is what keeps its steal after its raise): either the
pusher's fence follows the worker's, so the pusher reads the raised count
and runs E, and then E's advance either precedes the worker's sequence
read (the worker's Acquire read of the Release advance makes the push
visible to its steal) or follows it (the worker's park finds the sequence
changed under the lock and returns at once); or the pusher's fence
precedes the worker's, so the push precedes the worker's fence and the
steal after it sees the pushed work. A Requeue after a slice (`core.rs:83`) with
nothing else in the queue is not surplus. Its reds: after a run in which workers find work without parking, the idle
count equals the number of parked workers; and two workers, the owner
blocked inside a slice on a hook the test holds, a second process pushed
behind it; the second process runs on the other worker while the owner is
still blocked, gated by the hook, never by a clock.

**C2. W11's edge compares with a published `parked_until`, never a scan.**
`TimerWheel::earliest_deadline` (`timer.rs:235-238`) is a min over every
entry, O(n). Worker 0 reads it once per park, which is fine. It is not the
arming edge's comparison: liminal arms timers per connection, and a scan
per arm under the timers lock is not acceptable. Worker 0 publishes the
deadline it parks until into the wheel, `parked_until: Option<Instant>`, set
under the timers lock when it reads the deadline and cleared when it leaves
the park. `schedule_reserved_at` runs E only when `parked_until` is set and
the new deadline is earlier: O(1), and only when worker 0 would otherwise
oversleep.

**C3. Lock order: timers, then `wait_set`, never the reverse.** No path
takes the timers lock while holding `wait_set`. The build checks this by a
search across the scheduler and names every site that takes both, in order.
`expire_timers` into `mark_fired_receive_timer` already takes timers then
`wait_set`. `park_thread` reads the deadline and sets `parked_until` before
it takes `wait_set`, and never while holding it.

**C4. `tick_at`'s one-lap bound keeps every deadline exact.** The bounded
catch-up is correct only because `expire_bucket` fires an entry by
`expires_at <= now` and leaves later entries in place. Its red: a timer
armed more than one lap out, worker 0 parked for longer than a lap before
the deadline; the timer fires once, at its deadline, never early on the
bounded walk, and `current_tick` lands on `target_tick` after it.

W9's advance sits inside its existing critical section with the notify
after it; the sequence is the guard. W16 needs no edge. W24's replay rule
stands: the log decides, never the clock.

#### 3.8.3 Amendment to the signed bound (B-181 R4)

The Q-F ruling above stands as written. This amendment replaces the bound it
signed.

**The 5 ms floor is retired.** Its timed park existed only to poll the timer
wheel and the file-IO ring. A timed park whose only purpose is to poll is a
wait of a set length, and Tom's rule forbids every wait of a set length. The
floor was signed on 2026-07-11, before that rule was given. With the wake
edges of §3.8.1 and the conditions of §3.8.2 in place, nothing is left for
the poll to find: every source either wakes a parked worker or is waited for
by its own deadline. `IDLE_PARK_TIMEOUT` and `IDLE_WAKES_PER_SEC_PER_WORKER`
are removed with the floor.

**The bound now signed** keeps the formula the signing note asked for, wakes
per worker times the total workers across every scheduler in the process,
with total workers what `service_inventory()` reports:

- a scheduler with no armed timer and no IO in flight: **zero idle wakes**.
  Every normal worker parks with no timeout;
- with armed timers: **one wake per deadline worker 0 reaches**, whether the
  timer at that deadline fired or was cancelled before it. Worker 0 parks
  until the earliest armed deadline and no longer; every other worker parks
  with no timeout. Arming an earlier timer while worker 0 is parked wakes it
  once (§3.8.2 C2), and that wake is the new deadline's;
- in replay: **one wake per cursor advance**. The log decides when a replay
  worker leaves its park, never the clock (W24).

A wake caused by work (a message, a spawn, an exit, a completion, a surplus
push) is not an idle wake: it is the work arriving. `notify_all` still wakes
every parked worker of the scheduler for each such edge, for the reason
§3.8.1's invariant gives, and the cost of that herd per message is measured,
not bounded, here.

**Enforced by R3's tests** in `crates/beamr/src/scheduler/tickless_idle_tests.rs`:
an idle scheduler observes every worker parked unbounded; one armed timer
costs worker 0 exactly one timed-out park and every other worker none; a
file completion resumes its process with every worker parked unbounded; an
earlier arm wakes a parked worker 0 once and a later arm runs no edge; and
`tick_at`'s one-lap catch-up fires a timer more than a lap out once, at its
deadline. `crates/beamr/tests/timer_resident_cost.rs` and
`crates/beamr/tests/thread_inventory.rs` pin the same parks through the public
seams.

Re-signed: Waffles, 21 October 2026 04:49 (Melbourne), on Cambium, post
`c38bfbebea87c2c877f87f1eec949b4785a0ba1279ef40fccc020e93f5297ca2`,
replacing the 04:31 signature (post `8feade12`).

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
