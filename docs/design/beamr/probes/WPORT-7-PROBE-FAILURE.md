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

---

## OBSERVATIONS — OFFICIAL RUN 2026-07-18 (§1 and §2 sat; status now: RUN)

**Operator/environment/bundle:** identical to the WPORT-6 official run of the
same sitting (Artemis Peach at own hands; headless Chrome 150.0.7871.125 via
the zero-dep CDP driver; Node v26.5.0; Darwin 25.5.0; bundle at main `a399b54`
with wasm-bindgen 0.2.123). **Panic source, per the sitting riders:** the
shipped surface has no panic source (correct by design — the hook is
production, the only source is cfg(test)); the sitting bundle carries a
minimal UNCOMMITTED diff, committed here as evidence in the SAME commit as
these observations (`evidence/2026-07-18/wport7-panic-source.diff`, base
`a399b54`, sha256 `5fd1ac71fda6b227…`): it adds
`WasmVm::install_probe_panic_bif(&self)` registering `probe:panic_now/0`,
panicking with the identical message the cfg(test) wall uses. No reader of a
§1 row below may read it as shipped-surface behavior; the shipped surface
needs and has no panic source. Raw JSONs: `evidence/2026-07-18/wport7-*.json`.

### Section 1 — failure/panic surfacing

- **1a caught — GREEN.** SYNC-entry panic under try/catch: the registered
  panic callback fires BEFORE the trap with message + location; the
  `console.error` line is captured; the caught value's class is
  `RuntimeError` (`wport7-1a-caught.json`).
- **1a uncaught — GREEN.** Without try/catch, `window.onerror` observes the
  trap; the callback still fired first (`wport7-1a-uncaught.json`).
- **1b Worker — GREEN.** Same workload in a dedicated Worker: the Worker's
  own `self.onerror` fires AND the owner page's `worker.onerror` fires; the
  registered panic callback runs before the trap in the Worker context too
  (`wport7-1b-worker.json`). The callback-before-trap ordering claim holds on
  this engine in both contexts.
- **1c queued-turn (the ASYNC-entry case, deliberately not CI) — GREEN.**
  `send_message` returns; the panic fires inside the queued microtask turn;
  the trap surfaces at `window.onerror` (no caller to catch it); callback
  first; the instance is bricked afterward (`wport7-1c-queued-turn.json`).
- **1d post-panic latched-brick — CONFIRMED, WITH AN ON-PLATFORM REFINEMENT
  (board note).** The recovery contract's operative sentence holds exactly:
  every scheduler-touching call re-traps and a fresh `WasmVm` works. The
  refinement: on this toolchain the re-trap is the **wasm-bindgen borrow
  guard** — the aborted `&mut self` borrow never clears, so `run_step`,
  `send_message`, `terminal_error`, AND `await_exit` ALL re-trap synchronously
  with "recursive use of an object detected which would lead to unsafe
  aliasing in rust". The doc-inferred observables (`terminal_error()` → `null`,
  `await_exit` hang) are UNOBSERVABLE from JS: the wrapper guard intercepts
  before the arbiter internals; `terminal_error` never reaches its `null`
  return and `await_exit` never constructs a promise. The internal reasoning
  (`last_error = None`, stuck Draining) stays real but unobservable
  (confirmed identically under the Node baseline). Fresh-`WasmVm` recovery
  verified (`wport7-1d-latched-brick.json`). Routed to the arc board as a
  doc-refinement line: behavior unchanged, contract text's parenthetical
  inference superseded by the platform observation.
- **1e console ordering — RECORDED.** With the default console sink, the
  browser devtools console shows ONE interleaved out/err timeline (vs Node's
  OS-stream split); registering an io_sink callback delivers the sequence in
  order. The module doc's out-of-contract note now carries the actual
  platform spread (`wport7-console-ordering.json`).

### Section 2 — TIMER-STRAND CONFIRM-OR-KILL: **KILLED, all three environments**

Twenty samples per environment of the ruled workload (mid-turn
`erlang:send_after(0, self(), probe)` then park; phase varied by host
macrotask hops), observed via `setTimeout`/`clearTimeout` spies installed
before VM construction (the snapshot freight is cfg(test)-only — the probe's
own prescribed alternative channel; Node baseline initialized the same web
bundle under plain Node):

| Environment | p50 | max | arms during wait | fires/deliveries |
|---|---|---|---|---|
| Browser main-thread | 5.1 ms | 5.2 ms | exactly 1, all 20 | 20/20, exactly once |
| Dedicated Worker | 6.1 ms | 6.3 ms | exactly 1, all 20 | 20/20, exactly once |
| Node baseline | 1.34 ms | 4.40 ms | exactly 1, all 20 | 20/20, exactly once |

Delivery latency for mid-turn 0ms arms is bounded at macrotask scale in every
environment — nowhere near a ~1.024s wheel revolution — with one arm → one
fire → one delivery and zero re-arm churn. That is the ruled KILL criterion
met across the full matrix: **the wasm-side exposure hypothesis is killed**;
the boarded line closes with this artifact as evidence and the native-side
finding stands alone (haematite §4.4 budget untouched). Per the binding
boundary: `crates/beamr/src/timer.rs` was not touched, and no test asserts
timing precision — the distributions above are context, never walls.
(`wport7-strand.json`, `wport7-strand-worker.json`, `wport7-strand-node.json`.)
