# WPORT-8 PROBE-CAPABILITY — real-browser capability adapters on the real bundle

**Status:** `AUTHORED-NOT-RUN` — the run is gated on an official sitting
(D5: at least one official sitting recorded as probes-pattern evidence
BEFORE WPORT-8 closes; the tear may PASS with this probe authored and the
sitting scheduled — closure waits on the observations).
**Authored by:** WPORT-8 R9 (the probe artifact is a build deliverable; the
run is a sitting).
**Sitting scope rider (A4/rider 2, binding):** the sitting INCLUDES the
worker-shaped end-to-end — a real BEAM handler in the REAL generated bundle
performing a fetch and a KV round trip with the response assembled from
both, through worker-shaped capability wiring — landing the literal claim
amendment A4 moved out of the Miniflare leg.

## Why this probe exists

The Node walls prove the adapters against the in-memory host under the
wasm-bindgen runner: the closed error vocabulary on both sides of the
boundary, refuse-before-suspend, per-caller arm selection (A3), the
one-turn completion law, process-death auto-abort with counted dead-pid
completions, and the KV round trip. Node does NOT prove: real browser
`fetch` semantics (real `Response` objects, real header casing, real
network failure shapes), a real `AbortController` aborting a real in-flight
network request, an IndexedDB-backed KV adapter satisfying the same
capability shape as Workers KV and the in-memory host (the D8/TQ1 shape
claim), or the real generated bundle serving BEAM bytecode that calls
`wasm_fetch`/`wasm_kv`. Those are this probe's territory.

## Protocol

Build the real bundle (the `beamr-wasm` build output through its generated
bootstrap — never the Node test runner), embedding a BEAM handler module
that: (1) issues `wasm_fetch:request/1` against a sitting-controlled URL;
(2) performs `wasm_kv:put/2` + `wasm_kv:get/1` + `wasm_kv:list_by_prefix/1`;
(3) assembles its exit value from BOTH results. Serve the bundle from a
local origin (the WPORT-6 PROBE-FETCH serving pattern).

### Leg 1 — worker-shaped end-to-end (the rider)

1. Construct the VM from the real bundle; register a fetch capability over
   the browser's `fetch` with an AbortController-backed abort hook on the
   slot, and a KV capability over an IndexedDB adapter — BOTH shaped
   exactly as `examples/edge-worker/src/worker.js` registers them.
2. Spawn the embedded BEAM handler; observe: the real network request in
   the DevTools/CDP network log; the KV round trip against IndexedDB; the
   exit value assembling both ({ok, ...} map with fetched body + KV value).
3. Record: the exit JSON verbatim, the network log shape, and the
   IndexedDB contents after settle.

### Leg 2 — error legs on the real platform

1. Rejection: point `wasm_fetch:request/1` at an unroutable origin —
   observe `{error, {rejected, _}}` with the browser's real failure detail.
2. Cancellation: arm a request against a sitting-controlled slow endpooint,
   kill the calling process (or drive the host abort); observe
   `{error, {cancelled, _}}` (host-abort path) or the death-sweep abort at
   the network layer (the request disappears from the network log), and the
   counted dead-pid completion in the bridge counters.
3. Refusal: a VM with no KV capability registered — observe the synchronous
   `{error, {capability_missing, kv}}` with zero arbiter counter motion.

### Leg 3 — NO-POLLING under real completion

With one request in flight and the VM otherwise idle, instrument
`setTimeout`/`setInterval`/`requestAnimationFrame` (saved originals, the
F-0d shim pattern): zero recurring callbacks while awaiting the completion;
the settle produces exactly one arbiter turn. EARLY-UNDER-CACHED-CLOCK
applies: no wall asserts absolute arm counts across the real fire.

## Sitting-scope truing note (2026-07-24 — amends the endorsed protocol; CONFIRMED by Artemis Peach, 2026-07-23 DM, verified at the bytes)

Folded on the sitting-kit branch before the runsheet ships (Waffles'
condition), so the artifact promises exactly what the sitting evidence
delivers:

1. **Counter observations are Node-wall territory.** The bridge counters
   (`dead_pid_completions`) and the arbiter turn counters increment in the
   real bundle too, but their observation surface (the `counters()`
   accessors) is `cfg(test)`-gated and the counter group is `pub(crate)` —
   nothing is JS-visible, so they are not citable by sitting evidence. They are covered by the landed suite by exact
   name: `dying_caller_auto_aborts_in_flight_and_late_completion_is_counted`
   and `late_abort_after_completion_is_a_harmless_noop` (the dead-pid
   counter), `fetch_success_from_true_idle_delivers_the_codec_map_in_one_turn`
   (the one-turn law). Leg 2's "counted dead-pid completion in the bridge
   counters" and any counter reading of leg 3's one-turn sentence are
   therefore NOT sitting observations.
2. **Leg 2 cancellation is the host-abort arm.** The sitting drives the
   host abort (the protocol's "or"); the observations are the typed
   `{error, {cancelled, _}}` exit and the request's disappearance from the
   network log. The death-sweep arm stays proven by the Node walls above.
3. **Leg 3's mechanism is the timer-shim record.** The observation is the
   F-0d shim log — zero recurring host callbacks across the in-flight
   window — not a turn-counter check.

## Expected observations

Recorded per the probes pattern (environment, browser + version, OS,
bundle commit, evidence files under `probes/evidence/`); field names must
describe what they hold (the 2026-07-23 emitter ruling). Late-but-delivered
timing tolerances apply throughout; no observation asserts wall-clock
promptness.

## Observations

*(never CI acceptance — WPORT-9 keeps the permanent gate)*

---

## OBSERVATIONS — OFFICIAL RUN 2026-07-23 (status now: RUN)

**Operator:** Tom Whiting, official run at his own hands on his box (kit
authored by the WPORT-8 seat, smoke-verified twice and disclosed as not
the sitting — the WPORT-6 authored-then-official precedent).
**Environment:** Chrome 150 (real desktop, MacIntel), macOS (darwin
25.3.0), Node v26.4.0 serving, wasm-pack 0.15.0; bundle commit `4bf2ab7`
on `wport8/sitting-kit`. **Serving shape (recorded in every evidence
file):** wasm-pack pkg + generated `bootstrap.js` served directly — the
WPORT-6 sitting shape; the single-file `beamr.bundle.mjs` was bypassed
because its no-argument `init()` path is broken at the packager
(blob-scope ReferenceError; discovered by this kit's first browser smoke,
routed to its own micro-lane). **Evidence:**
`probes/evidence/2026-07-23-capability-sitting/` (six files, committed
`91d1a94`). **Harness shape (banked per the domain owner's word):** the
page drives `await_exit` probes woken by capability-promise settlements,
never timers — `await_exit` on a capability-parked process resolves
`"idle"` (the WPORT-2 settled-idle contract behaving correctly), so the
sitting harness itself is NO-POLLING-honest.

Per the protocol's legs, as amended by the sitting-scope truing note:

1. **Leg 1 — worker-shaped end-to-end (the A4/rider-2 literal claim) —
   GREEN.** A real BEAM handler in the real generated bundle performed one
   real fetch (status 200, real header casing, `x-wport8-probe: ok`) and a
   full IndexedDB KV round trip (put→get byte-exact `browser-idb-value`;
   `list_by_prefix` lexicographic `[wport8:preexisting, wport8:sitting]`),
   and its exit value assembles BOTH results in one map
   (`wport8-leg1-end-to-end.json`; IndexedDB contents read back after
   settle match the listing). Network log: exactly one `/probe/ok`
   request.
2. **Leg 2, rejection — GREEN, mechanism trued.** The typed arm held
   exactly — `{error, {rejected, <<"Failed to fetch">>}}`, the browser's
   real failure detail — but the mechanism was NOT connection-refused as
   the kit's rationale assumed: Chrome blocks the `127.0.0.1:9` target via
   its unsafe-port list (`net::ERR_UNSAFE_PORT` in the network log) BEFORE
   any connection attempt (`wport8-leg2-rejected.json`). Within the leg's
   intent — a real platform failure shape through the typed vocabulary —
   and recorded as it actually was.
3. **Leg 2, cancellation (host-abort arm per the truing note) — GREEN.**
   Host abort fired at 400 ms into a 30 s request;
   `{error, {cancelled, <<"signal is aborted without reason">>}}` through
   the normal seam; the request shows `(canceled)` in the network log with
   no response on the wire (`wport8-leg2-cancelled-host-abort.json`).
4. **Leg 2, refusal — GREEN.** A VM with fetch registered but no KV:
   synchronous `{error, {capability_missing, kv}}`
   (`wport8-leg2-refusal-uninjected-kv.json`).
5. **Leg 3 — NO-POLLING under real completion — GREEN.** ZERO timer-shim
   events across the ~1.5 s in-flight window (`setTimeout`/`setInterval`/
   `requestAnimationFrame` all 0; the suite-wide shim total for the entire
   run is 0), and the settle delivered the D8 response map
   (`wport8-leg3-no-polling-timer-shim.json`). Per
   EARLY-UNDER-CACHED-CLOCK, no absolute arm counts were asserted — the
   observation is the recorded shim log.

Tom's network-log checklist: one request per leg, no repeats anywhere in
the run. This lands the D5 official sitting including the A4/rider-2
worker-shaped end-to-end; the status line at the top of this file is
superseded by this section.
