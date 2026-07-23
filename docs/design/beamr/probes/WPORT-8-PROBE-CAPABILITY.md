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

## Expected observations

Recorded per the probes pattern (environment, browser + version, OS,
bundle commit, evidence files under `probes/evidence/`); field names must
describe what they hold (the 2026-07-23 emitter ruling). Late-but-delivered
timing tolerances apply throughout; no observation asserts wall-clock
promptness.

## Observations

*(appended at the sitting; never CI acceptance — WPORT-9 keeps the
permanent gate)*
