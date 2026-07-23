# WPORT-8 capability sitting kit

Everything the official sitting of
`docs/design/beamr/probes/WPORT-8-PROBE-CAPABILITY.md` needs, staged in one
directory. The page drives all three protocol legs against the **real
generated bundle** and writes the evidence JSONs itself — the one manual
observation left is the network-log checklist below.

Scope is the probe's three legs exactly, as amended by the sitting-scope
truing note in the probe doc (counters are Node-wall territory; leg 2
cancellation is the host-abort arm; leg 3's mechanism is the timer-shim
record).

## What "the real bundle" means here (serving shape)

The kit serves the **wasm-pack pkg + the generated `bootstrap.js`
directly** — the same shape the WPORT-6 official sitting ran. The
single-file `beamr.bundle.mjs` packaging is NOT used: its no-argument
`init()` path is broken at the packager (`decodeEmbeddedBase64` /
`WASM_BASE64` are substituted into wasm-bindgen glue that runs in a
separate blob-URL module scope — ReferenceError on load; discovered by
this kit's first browser smoke, 2026-07-24, routed to its own lane).
The single-file packaging is distribution convenience; the thing under
test — the real generated bundle with the embedded handler — is what's
served. The evidence environment block records this (`serving_shape`).

## Prerequisites

- Rust toolchain with the `wasm32-unknown-unknown` target
- `wasm-pack` (kit smoke-verified with 0.15.0)
- Node ≥ 20 (kit smoke-verified with v26.5.0)
- A real browser (Chrome/Chromium recommended — its network log shows
  `(canceled)` for aborted requests explicitly)

## The runsheet — three commands

From the repository root, on the sitting-kit branch:

```sh
node docs/design/beamr/probes/kits/wport8-capability/serve.mjs
```

(First run builds the real bundle — wasm-pack web target with the sitting
handler embedded — then serves; later runs reuse it. Delete
`crates/beamr-wasm/pkg/` to force a rebuild.)

```sh
open http://127.0.0.1:8787/
```

In the page: **open DevTools → Network first**, then press
"Run all three legs". Watch the leg list go green; evidence JSONs stream
into `evidence-out/` as each leg completes (the serve terminal logs each
write).

```sh
node docs/design/beamr/probes/kits/wport8-capability/collect.mjs
```

Copies the evidence into
`docs/design/beamr/probes/evidence/<date>-capability-sitting/` and prints
the explicit `git add` line. Review before staging — the kit never runs
git for you.

## The manual observation: network-log checklist

Confirm in DevTools → Network while/after the run (this is the only
observation the page cannot capture itself):

1. **Leg 1**: exactly one request to `/probe/ok`, status 200. No repeats.
2. **Leg 2a**: one request to `http://127.0.0.1:9/` failing before any
   response (Chrome shows `net::ERR_UNSAFE_PORT` — the unsafe-port list
   blocks it before a connection attempt; observed at the official run).
3. **Leg 2b**: one request to `/probe/slow?ms=30000` shown **(canceled)**
   ~400 ms in — the host abort fired mid-flight; the request disappears
   from the wire without a response.
4. **Leg 3**: one request to `/probe/slow?ms=1500`, completing normally
   ~1.5 s in. No request repeats anywhere in the run (no retries, no
   polling fetches).

Record what you saw in the sitting observations section of the probe doc
(a sentence per line above is enough; the evidence JSONs carry the rest).

## What each evidence file holds

- `wport8-leg1-end-to-end.json` — the A4/rider-2 literal claim: a real
  BEAM handler in the real bundle did one fetch AND a KV
  put→get→list_by_prefix against IndexedDB, and its exit envelope
  assembles both (`fetch_response` + `kv_stored_value` +
  `kv_keys_under_prefix`, lexicographic). Includes the IndexedDB contents
  read back after settle.
- `wport8-leg2-rejected.json` — `{error, {rejected, Detail}}` with the
  browser's real failure text, from the port-9 target (mechanism as
  observed: Chrome's unsafe-port list blocks it pre-connection —
  `net::ERR_UNSAFE_PORT`; truly unroutable IPs can hang for minutes —
  wrong for a sitting).
- `wport8-leg2-cancelled-host-abort.json` — `{error, {cancelled, Detail}}`
  after the page's AbortController fired at 400 ms (the truing note's
  host-abort arm).
- `wport8-leg2-refusal-uninjected-kv.json` —
  `{error, {capability_missing, kv}}` from a VM with fetch registered but
  no KV — the refusal is module-scoped and synchronous.
- `wport8-leg3-no-polling-timer-shim.json` — the F-0d shim record across
  the in-flight window: every `setTimeout`/`setInterval`/`rAF` arm between
  spawn and settle, with per-API counts. The harness itself uses only
  saved pre-shim originals (disclosed in-file), so the events are the
  bundle's and the platform's alone.
- `wport8-suite.json` — leg outcomes + timestamps for the whole run.

Every file embeds the auto-captured environment block: browser UA,
bundle commit + branch + dirty flag, node/wasm-pack versions, OS, origin.

## How the page drives the VM (and why that shape)

`await_exit` registered while a process is parked on a capability op
resolves `"idle"` — the WPORT-2 settled-idle contract behaving correctly.
The page therefore probes `await_exit` in a loop where **each probe is
woken by a capability-promise settlement** (the page owns every capability
promise), never by a timer — so the harness itself is NO-POLLING-honest
and the timer-shim log stays meaningful. A 90 s watchdog (saved original
`setTimeout`) turns a genuinely stuck leg into a recorded timeout instead
of a hang.

Host-policy notes (invisible to the VM, which sees the edge-worker-shaped
capability objects verbatim): the KV namespace is cleared at leg-1 start
for a deterministic listing, then pre-seeded with one key so
`list_by_prefix` provably sees more than the leg's own write; the fetch
capability keeps a controller registry so leg 2b can fire the host abort.

## Status

Smoke-verified end to end twice — a smoke run is NOT the official
sitting:

- **Node** (real bundle, real handler): all five entries deliver the
  exact contract shapes.
- **Real browser** (headless Chrome 150, `?autorun=1`): all five legs
  recorded — leg 1 assembled real fetch headers + the IndexedDB round
  trip with lexicographic keys; rejection detail `Failed to fetch` and
  cancellation detail `signal is aborted without reason` are the
  browser's own texts; leg 3 recorded ZERO timer-shim events across a
  ~1.5 s in-flight window.

The official sitting is Tom's run, at his hands, in a real browser with
the network-log checklist; its evidence goes to the probe doc's
observations section and `probes/evidence/`.
