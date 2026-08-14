# RUN RECORD — #106 process-global VM census + notify-coverage measurement

Lane: #106 "Idle steal-wake cost: designed and SIGNED per-worker — the
defect candidate relocates to composition (6 VMs/process)." Two halves,
both ruled at Waffles' desk 2026-08-14.

## Half 1 — the park-unbounded fix, KILLED by its own precondition

The candidate fix (workers 1..N park unbounded; per-VM idle cost collapses
from N workers to 1) was gated on measuring notify coverage first. Measured
at beamr 6167f74, execution.rs + execution/core.rs + every producer:

- TWO stealable-work producers fire NO notify:
  - execution/core.rs:83 — `SliceOutcome::Requeue` pushes the process back
    onto the running worker's own local queue, no notify_all.
  - execution/core.rs:175 — the Wait arm's self-wake recheck pushes, no
    notify.
  Both are correct TODAY only because the pusher pops its own queue next
  loop and parked siblings re-scan within 5ms on the timed wake.
- Everything else IS covered: all inject pushes notify (spawning.rs:466→467,
  487), all woken-list pushes notify (spawning.rs:454→455, mod.rs:2441→2442,
  execution.rs:472→473, core.rs:234→235, four supervision_integration
  sites), suspension-event wakes notify. `dirty_completions_changed` is a
  separate teardown-admission condvar, not a work-wake path. Timers are
  worker 0's poll by design.
- Consequence: unbounded park would not deadlock, but requeue-heavy load
  would get a parallelism-collapse window (N cores' work on one) until the
  next notify-bearing event — a cliff that appears only under load.

RULED (Waffles): the fix is refused as-is. The viable version (sleeper-aware
notify on the Requeue hot path) changes what the SIGNED §3.8 constant means
and is a signature-holder question (Waffles + Vesper) presented together
with the bound restatement, only if measured idle cost ever justifies it.
Until then the timed wake stands.

Cross-feed to #87 (candidate, NOT a claim): the notify-free Requeue fits
8.5–10.6ms ≈ two 5ms park cycles. Arithmetic fit is not a mechanism; to be
measured inside #87 before believed.

## Half 2 — the census (commit bb96894), Waffles' weighted-highest

The gap: inventory.rs was per-Scheduler only; nothing in-process could
answer "how many VMs does this process hold" — the only instrument was
decomposing per-index thread names from outside (Vesper's door). Six VMs in
one aion process was nobody's intent and nothing announced it.

Shipped, with both ruled sharpenings applied:

- `ProcessVmCensus { vm_count, scheduler_worker_count }` +
  `process_vm_census()` + RAII `VmCensusRegistration`, in inventory.rs,
  re-exported from `beamr::scheduler`.
- POPULATION HONESTY (sharpening 1): the census counts schedulers
  CONSTRUCTED AND NOT YET DROPPED, not "running" — shutdown does NOT
  deregister, a leaked scheduler counts forever, and the rustdoc says
  exactly what question the number answers. Deregistration is Drop-only, so
  the population holds by construction.
- NO SILENT-ZERO PATH (sharpening 1b): both counters live in ONE packed
  AtomicU64 (VM count high 32, worker count low 32) — readers always see
  consistent pairs and there is no lock to poison; the failure mode is
  unrepresentable rather than handled. The u32 packing bound raises loudly
  from the Result-returning constructor.
- NUMBERS ONLY (sharpening 2): no threshold, no warning level. The §3.8
  multiplication (IDLE_WAKES_PER_SEC_PER_WORKER × scheduler_worker_count)
  is named in the rustdoc as the CONSUMER'S arithmetic.
- Registration value is the worker count each VM ACTUALLY spawned
  (threads.len() at the single construction funnel), not a config claim.

Positive control: `crates/beamr/tests/process_vm_census.rs`, deliberately
its own test binary (cargo gives it its own OS process, so absolute census
asserts are race-free; the file's header forbids adding tests to it). One
test walks 0/0 → two VMs {2,4} → shutdown one → STILL {2,4} → drop → {1,3}
→ drop → {0,0}.

## Falsifier (two arms, each biting one ruled claim)

Committed-first; restores checkout-safe, sha-verified both files.

- Arm A — Drop decrement removed (models leak / unpaired registration):
  rc 101 at the post-drop assert (census stuck). The pairing claim bites.
  Log: `falsifier-armA-drop-pairing.log`.
- Arm B — shutdown made to deregister (added a deregister helper + one call
  in Scheduler::shutdown; models silently changing the population to
  "running"): rc 101 at exactly "shutdown must not deregister: the census
  counts constructed-not-dropped". The population pin bites. Log:
  `falsifier-armB-shutdown-population.log`.

## Battery (canon 8-leg, at bb96894)

Runner: gate-logs/103/battery-RUNNER.sh reused byte-identical, in-repo per
precedent. Marker: **COMPLETE (derived: 8/8, pin stable)**, tree census 0
both ends, per-leg rc all 0. Closed 2026-08-14T13:22:37Z.

Axes (NAMED: result-lines / passed / failed / ignored), pre-registered in
`PREDICTION.md` before any leg output was read:

| leg                | prior             | predicted         | measured          |
|--------------------|-------------------|-------------------|-------------------|
| tests              | 74 / 2114 / 0 / 0 | 75 / 2115 / 0 / 0 | 75 / 2115 / 0 / 0 |
| tests-all-features | 74 / 2124 / 0 / 0 | 75 / 2125 / 0 / 0 | 75 / 2125 / 0 / 0 |

Both exact, confirmed by test NAME in each leg's own log.

Semver: additive public API (one struct, one function) — 0.18.x-class,
no break.
