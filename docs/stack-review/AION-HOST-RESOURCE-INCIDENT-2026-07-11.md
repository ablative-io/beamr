# Aion host resource incident — Beamr handoff (2026-07-11)

For Vesper and the implementing team. This is a focused incident note against
Beamr `main` at `103e5fd` / the Beamr 0.13.0 used by Aion. It is not a general
architecture review.

## Executive finding

Beamr did **not** invent Aion's idle CPU loop. Liminal returns
`NativeOutcome::Continue` after a nonblocking socket reports `WouldBlock`, and
Beamr correctly implements `Continue` as immediate requeue. The primary CPU fix
belongs in Liminal's connection-readiness design.

Beamr did, however, turn that defect into a much larger host-resource event.
Every `Scheduler` eagerly constructs dirty pools, two macOS fallback I/O pools,
and two distribution runtimes even when an embedder does not use them. Aion had
six Beamr schedulers. Their source-derived thread budget explains 207 of the
224 threads observed in the Aion process.

The distinction matters: preserve Beamr's scheduling semantics and FIFO
fairness fix; make embedded runtime services explicit and composable.

## Incident evidence

The host investigation found:

- A freshly relaunched Aion server settled at approximately 350–427% CPU with
  11 connected but idle workers.
- Four connection-scheduler workers dominated the process. In the captured
  stackshot, the four accounted for 89.7% of Aion's accumulated thread CPU.
- Ten macOS Aion CPU-resource reports from 7–11 July sampled the same
  `ConnectionProcess` path.
- The Aion process had 224 threads.
- Aion contained one Beamr version, crates.io 0.13.0. It already includes the
  FIFO owner-queue repair released in 0.12.1.

The WindowServer watchdog termination was a separate immediate failure: its
main thread stopped checking in while the host was saturated. The chronic Aion
load made the machine fragile, but this note does not attribute WindowServer's
specific stalled call to Beamr.

## Why the connection never parks

Beamr's contract is explicit:

- `NativeOutcome::Continue` means "Re-queue immediately" and `Wait` means
  "park until a message arrives" (`crates/beamr/src/native/native_process.rs:49-60`).
- `run_native_slice` maps those outcomes directly to `Requeue` and `Wait`
  (`crates/beamr/src/scheduler/execution/native_slice.rs:95-107`).
- The `Requeue` arm stores the process and immediately pushes its PID back onto
  a run queue (`crates/beamr/src/scheduler/execution/core.rs:74-82`).

Therefore a native handler returning `Continue` on socket `WouldBlock` remains
runnable forever by contract. Beamr's FIFO change fixed the older starvation
failure by sharing slices fairly among such processes. Current regression tests
deliberately model a Liminal-shaped always-`Continue` handler and prove
fairness/message progress, not idle quiescence
(`crates/beamr/src/scheduler/tests.rs:3294-3439`). The repair democratized the
spin; it did not stop it.

The correct readiness shape already fits Beamr:

1. A reactor registers socket readiness.
2. The handler drains bounded work to `WouldBlock`, arms/rearms readiness, then
   returns `NativeOutcome::Wait`.
3. Readiness enqueues a durable mailbox marker with
   `Scheduler::enqueue_atom_message`.
4. The next slice drains the marker and socket work, then parks again.

`enqueue_atom_message` handles delivery while the process is executing
(`scheduler/mod.rs:1320-1347`), and the three-phase park path registers the
waiter and rechecks its mailbox (`scheduler/execution/core.rs:83-155`). Do not
use bare `wake_notifier` as an edge notification unless readiness is also kept
in durable/sticky state: it calls `wake_process`, and a wake arriving before
wait-set registration is otherwise a no-op (`scheduler/execution.rs:21-31` and
`:311-337`).

## Beamr's independent amplification

On this 10-core Apple Silicon host, each threaded `Scheduler` eagerly attempts
to create the following before normal scheduler workers are counted:

| Service | Threads | Source |
|---|---:|---|
| Dirty CPU pool | 10 | `scheduler/mod.rs:684-703` |
| Dirty I/O pool | 10 | `scheduler/dirty.rs:23-24,176-209` |
| File I/O fallback ring | 4 | `scheduler/mod.rs:764-773`, `io/thread_pool.rs:92-117` |
| Standard-I/O fallback ring | 4 | same construction path |
| `DistSender` Tokio runtime | 1 | `distribution/sender.rs:215-225` |
| `NetKernel` Tokio runtime | 1 | `distribution/mod.rs:74-87` |

That is 30 ancillary threads per scheduler. The dirty pools are eager and
coerce a configured zero to one. The two fallback rings are unconditional on
macOS. `SchedulerConfig::distribution == None` is converted to an empty default
resolver rather than disabling distribution (`scheduler/mod.rs:731` and
`distribution/mod.rs:139-145`), so both one-worker runtimes are still attempted.

The six Aion schedulers had 27 normal workers in total
(`10 + 10 + 4 + 1 + 1 + 1`). The resulting Beamr budget is therefore
`6 × 30 + 27 = 207` threads, closely matching the observed 224-thread process
after ordinary host/application helpers are included.

Tokio runtime creation is fallible, so the two runtime workers are attempted
rather than guaranteed by type. They were present in the incident inventory.
`SchedulerConfig::io = Some(...)` would add another fallback ring and completion
thread; that optional path was not active here.

## Required Beamr work

This is an embedder-composition problem, not a request to shrink the full VM by
silently changing defaults.

1. Make scheduler-owned services explicit: normal workers, dirty CPU, dirty
   I/O, file I/O, standard I/O, distribution sender and net kernel each need an
   honest `disabled`, `owned`, or injected/shared form. Exact policy and sizes
   must be chosen by the embedder rather than hardcoded for all deployments.
2. Make `distribution: None` mean absent. A separate explicit default
   distribution configuration can preserve the standalone/full-runtime path.
3. Allow disabled dirty pools to own zero threads and reject unsupported dirty
   dispatch clearly, rather than silently coercing zero to one.
4. Permit multiple embedded schedulers to share suitable service pools, or
   provide a minimal runtime profile that constructs only requested services.
5. Expose a runtime service/thread inventory so an application can report the
   actual budget instead of reconstructing it from thread names.
6. Add a race-safe message/atom notifier convenience API and document that a
   bare wake requires independently durable state.

## Acceptance gates

- A minimal embedded scheduler starts only the explicitly requested normal
  workers; disabled services create zero workers.
- `distribution: None` creates neither distribution runtime.
- Disabled dirty dispatch returns a specific error and cannot block or panic a
  normal scheduler.
- File and standard-I/O backends can be disabled, lazily created, injected or
  shared; they are not unconditionally eight threads per scheduler on macOS.
- Two embedded schedulers can share selected services without double-owning or
  double-joining them.
- Shutdown joins exactly the resources owned by that scheduler.
- The public inventory agrees with OS thread names/counts in tests.
- Readiness delivered before wait registration and immediately after
  registration is preserved by a durable marker.
- Existing FIFO fairness and always-runnable-process regression tests remain.

Do not treat a polling sleep, fewer normal scheduler threads, or reverting FIFO
fairness as a fix. Those only conceal or redistribute the Liminal busy loop.
