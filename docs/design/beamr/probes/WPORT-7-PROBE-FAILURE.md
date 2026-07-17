# WPORT-7 PROBE-FAILURE — real-browser failure/panic surfacing + the timer-strand confirm-or-kill rider

**Status:** `PROBE-FAILURE: NOT RUN — AUTHORED-NOT-RUN`
**Authored by:** WPORT-7 (D12/D14: the probe artifact is a deliverable of that
brief; the run is NOT)
**Gating word:** the run happens post-land, on Tom's or Annabel's word — a
short manual browser + Worker sitting against the real generated wasm bundle.
Observations are appended here as evidence attached to this artifact; they
are **never** CI acceptance. WPORT-9 keeps the permanent browser conformance
gate (arc `:180` names panic surfacing in its wall).

## Why this probe exists

The WPORT-7 CI walls run under the pinned Node wasm-bindgen runner. Node
proves: the typed `SchedulerFailureError` surface on all five legs, the
manual-drain wedge fix and latch symmetry, both observability surfaces, the
promoted ordering contract, and the SYNC-entry panic path (a panicking
cfg(test) BIF whose trap is caught as a JS exception with the registered
panic callback fired first). Node does **not** prove real browser panic
routing (`window.onerror` vs Worker `onerror`), the ASYNC-entry abort case,
real-platform console stream ordering, or the post-panic latched-brick state
on a real engine. Those are this probe's territory — plus the arc `:92`
timer-strand rider, section 2.

## Section 1 — failure/panic surfacing

Build the real wasm bundle (the `beamr-wasm` build output through its
generated bootstrap, not the Node test runner) with a deliberately panicking
workload available (a debug module or a host-registered async NIF whose
completion panics is sufficient; the shipped surface needs no panic source).

### 1a. Browser main-thread panic surfacing

1. Construct a `WasmVm`, call `register_panic_callback` with a callback that
   records its payload, and install a `window.onerror` recorder.
2. Trigger a panic through a SYNC entry (`vm.run_step()` over a panicking
   workload) inside a try/catch; then trigger one WITHOUT a try/catch.
3. Record: the callback payload (message + location, BEFORE the trap), the
   `console.error` line, the caught value's class (`RuntimeError` expected),
   and — uncaught — whether `window.onerror` observes the trap and with what
   message.

### 1b. Worker-context panic surfacing

Same workload inside a dedicated `Worker`. Record whether the Worker's
`onerror` (and the owner page's `worker.onerror`) fires for an uncaught
trap, and whether the registered panic callback still runs first. Engines
differ here; the callback-before-trap ordering is the claim under test.

### 1c. The ASYNC-entry abort case (why it is NOT a CI wall)

The arbiter-callback entry (queued turn) panicking means the trap unwinds
into the host's microtask/timeout dispatch — no caller exists to catch it.
`#[should_panic]` under the Node wasm-bindgen runner is UNPROVEN for this
shape (the harness observes the trap as a test failure, and the poisoned
instance state after an abort makes any in-suite continuation claim
unreliable), so the case is deliberately absent from CI (D8). Here: trigger
a panic inside a QUEUED turn (send to a panicking workload, let the
microtask fire) and record where the trap surfaces (1a/1b recorders), that
the panic callback fired first, and what the host dispatch does afterwards.

### 1d. Post-panic latched-brick confirmation (the D7 recovery contract)

The recovery contract, verbatim as shipped in the `failure.rs` module doc:

> post-panic the instance is latched (borrowed RefCells, stuck Draining);
> every scheduler-touching call re-traps; construct a fresh WasmVm.

The borrow-flag reasoning is INFERRED from toolchain semantics (abort runs
no Drop code, so the live `RefCell` borrow never clears and ArbiterState
stays Draining) — this leg confirms it ON PLATFORM: after 1a's caught panic,
call `run_step`, `send_message`, and `await_exit` and record that each
re-traps (or, for `await_exit`, hangs — `fail()` never ran, `last_error` is
`None`, and `terminal_error()` still answers `null`; the arbiter's error
machinery never observes a panic, by documented design). Then construct a
fresh `WasmVm` and record that it works.

### 1e. Node-vs-browser console ordering (out-of-contract note)

With the default console sink, emit an interleaved out/err sequence and
record how the platform orders it: Node splits `console.log`/`console.error`
across `stdout`/`stderr` (OS streams; relative order may shuffle), a browser
devtools console typically shows one interleaved timeline. The sink module
doc declares cross-stream console order OUT-OF-CONTRACT; this leg documents
the actual platform spread so the note stays honest. Hosts wanting one
ordered stream register a sink callback.

## Section 2 — TIMER-STRAND CONFIRM-OR-KILL (arc `:92` rider)

**Ruled terms, verbatim carriers:** "confirm-or-kill measurement rides the
NEXT wasm-timer-touching brief (WPORT-7+ …) in the WPORT-3 probe pattern —
no behavior change without cert re-sign." WPORT-7 is that brief by the
ruling's own criterion (the deadline-reconcile seam is its sole fallible
drain op and the drain-failure walls inject throwing timer primitives).
**Boundary, binding:** NO TimerWheel cursor "fix", NO edits to
`crates/beamr/src/timer.rs` under ANY observation — cert-resign territory.
Any wheel observation routes back to the board, never into a diff.

### Hypothesis under test (board finding 2026-07-17)

The shared `TimerWheel` strands behind-cursor inserts for a full 1024-tick
revolution (~1.024s): insert never consults the cursor, the sweep only moves
forward, and not-yet-due entries are skipped in place with no rounds
counter. The wasm-side exposure is INFERRED: the cooperative scheduler
drives the same wheel and `erlang:send_after(0, …)` is live since WPORT-3,
so a MID-TURN 0ms arm may land behind the cursor with the unified one-shot
re-arming an overdue deadline repeatedly — ~1s of macrotask churn.

### Measurement plan

1. Workload: a bytecode process that, mid-turn (after the wheel cursor has
   advanced within the drain), arms `erlang:send_after(0, self(), probe)`
   and parks in receive; repeat across a spread of arm instants relative to
   turn starts (a loop driven by ordinary sends is enough to vary phase).
2. Observables (all COUNTED, `UnifiedDeadlineSnapshot` freight — never
   derived): `next_arms`, `rearms_earlier`, `executions`, `cancellations`
   across the interval between the 0ms arm and its delivery; wall-clock
   delivery latency of the `probe` message; host timer activity (devtools
   performance panel / `setTimeout` spy counting one-shots per second).
3. Environments: Node runner (baseline), browser tab foreground, Worker.

### Confirm criteria

A 0ms mid-turn arm whose delivery latency clusters near a full revolution
(~1s) with the unified one-shot re-arming an overdue deadline repeatedly
(counter churn: `executions`/`next_arms` climbing across the wait, one-shot
cadence ~per-macrotask) CONFIRMS the strand on the wasm side. Route the
observation to the board: the fix is cert-resign territory (haematite §4.4
budget on the native side; the wasm exposure gets its own boarded line).

### Kill criteria

Delivery latency for mid-turn 0ms arms bounded well under a revolution
(macrotask-scale, no re-arm churn; counters showing one arm → one fire →
one delivery) across all three environments KILLS the wasm-side exposure
hypothesis; the boarded line closes with this artifact as evidence and the
native-side finding stands alone.

**No test asserts timing precision** (the WPORT-3 acceptance law binds here
too): the probe records distributions, never wall-clock walls.

## Close note — one bundled sitting for three OPEN pillars

Three accumulated probes now await the same manual browser + Worker sitting:
the WPORT-3 deadline pillar (`WPORT-3-PROBE-THROTTLE.md`), the WPORT-6 fetch
pillar (`WPORT-6-PROBE-FETCH.md`), and this failure/panic pillar. This brief
INVITES one bundled sitting covering all three on Tom's or Annabel's word —
sequencing stays unowned by all three briefs; observations attach to each
artifact separately and are never CI acceptance. WPORT-9 remains the
permanent gate for the conformance claims.

Late observations append below this line; the probe stays authored-not-run
until then.
