# WPORT-9 ground pack — browser conformance and the permanent NO-POLLING gate

**Arc:** PILLAR-BEAMR-WASM, final rung. **Author:** Osiris Yogo (beamr seat).
**Evidence pin:** `293d368` (main at the WPORT-8 close + packager-fix landing).
Every cite below is at `293d368` unless a historical pin is named explicitly.
**Tear:** Artemis Peach, domain owner. Per gates `RUNBOOK.md` at gates main
`e6b5a43`: tears pin to heads — the ready-for-tear declaration freezes the
bytes; a moved head re-declares before any verdict binds.

This pack grounds the LAST rung: it decides nothing itself. Section 6 poses
the open tear questions; the WPORT-9 brief encodes the domain owner's
dispositions, and the R-numbers earn them at the bytes.

## 1. The frozen boundary, verbatim

The arc doc's WPORT-9 section (`docs/design/beamr/WASM-PORT-ARC.md:177-190`):

**Size** L. **Dependencies** WPORT-2 through WPORT-8 (all CLOSED — the last,
WPORT-8, at this pin's own landing).

**Scope (`:182`):** "Establish permanent browser/Worker conformance CI using
the real generated wasm bundle and a real Beamr workload. Exercise
construction, artifact loading, supported BIFs, output and failure paths,
and every wake class. Make the NO-POLLING wall a permanent release gate:
idle means no recurring callbacks."

**Binding law (`:186`):** "NO-POLLING applies here as a permanent F-0d
conformance gate. A polling scheduler is a TEAR CONDITION. Any timer whose
job is 'check whether something changed' is a design error."

**Acceptance shape (`:188`):** "CI packages and instantiates
`beamr_wasm_bg.wasm`, loads and executes real Beamr bytecode, and does not
replace the VM with a JS stub. The suite independently proves mailbox send,
cast, async-NIF/Promise completion, receive timeout, native deadline, native
completion, and trapped-exit wakes; artifact fetching; supported and
unsupported BIF behaviour; output; process error; pump/turn error; and panic
surfacing. Callback instrumentation observes **zero recurring host callbacks
while idle**, including while only future deadlines exist. The gate fails on
rAF rechecks, intervals, repeating timeouts, synchronous bounded pump loops,
or any ready process that waits for an external manual pump."

**Named socket/gate filled (`:190`):** "Permanent F-0d / NO-POLLING browser
conformance gate."

The boundary-evidence paragraph (`:184`) is pinned at `d9de35e` and is STALE
on its primary leg — "the current workflow runs only `cargo check` and has no
wasm runtime … test" described the pre-WPORT-2 workflow. Per the arc's own
supersession discipline (believed state is not citable state, both ways),
§3 below re-verifies the entire current state at `293d368`. One `:184` leg
remains live and is carried forward: the edge-worker Miniflare test's
in-memory VM stub (re-verified in §3.7).

## 2. The GO directive (Waffles the Terrible, 2026-07-24)

Formal GO with two standing constraints, binding on this pack and the brief:

**G1 — hermetic-or-classed, exhaustively.** The gate is PERMANENT CI, so
everything the sittings proved manually must either be pinned by a hermetic
wall or explicitly classed as sitting-territory with the reasoning recorded.
The WPORT-8 sitting-scope truing note is the classing precedent. No gate leg
may depend on a live network or a real browser unless the workflow provably
provides it.

**G2 — the packaged single-file bundle's no-arg init path is a first-class
conformance leg.** The packager fix landed at `314bc7c` means the gate can
finally drive the REAL single-file bundle by name; it is the only leg of its
kind that has never had a permanent home.

## 3. Current-state evidence (all cites at `293d368`)

### 3.1 The permanent workflow already carries the Node-side conformance suite

`.github/workflows/cooperative-wasm.yml` is no longer the `cargo check`-only
workflow the arc's `:184` describes. At `293d368` it has four legs
(`cooperative-wasm.yml:14-29`): the beamr cooperative-closure check
(`:17-18`), the beamr-wasm manifest-closure check (`:19-20`), wasm-bindgen
CLI install pinned at 0.2.123 (`:21-22`), and the full wasm suite executed
under `wasm-bindgen-test-runner` on Node (`:23-37`). The suite is pinned by
EXACT-NAME carriers — an 80-entry `expected_tests` array (`:41-122`), each
name grepped as a full passing line (`:124-129`) — and an EXACT-COUNT
carrier: "test result: ok. 80 passed; 0 failed; 0 ignored; 0 filtered out"
(`:130-135`). A mechanical borrow wall guards the artifact loader
(`:137-142`). Count carriers move WITH names per commit (house law since
WPORT-2).

What the 80 already prove is inventoried in §3.3-§3.4. The headline for this
brief: the F-0d idle wall and every wake class ALREADY have Node walls in
permanent CI. WPORT-9's gap is NOT "no wasm runtime in CI" — it is the four
gaps in §3.2.

### 3.2 The four verified gaps at `293d368`

1. **No real-bundle leg.** The workflow compiles the crate as a wasm-bindgen
   TEST artifact. Nothing in CI runs `wasm-pack build`, evaluates the
   generated `bootstrap.js`, instantiates `beamr_wasm_bg.wasm` through the
   shipped glue, or drives the packaged single-file bundle. The acceptance
   shape's "packages and instantiates `beamr_wasm_bg.wasm`" is unmet in
   permanent CI (it is met manually — four official sittings, §3.8).
2. **No browser or Worker leg.** Every CI wall runs under Node. All
   browser-context proof is sitting-based (§3.8); the Worker-shaped proof is
   the edge-worker Miniflare suite, which is not wired into any workflow
   (§3.7) — and its VM is a JS stub, exactly what the acceptance shape
   forbids the conformance suite to rely on.
3. **No-arg single-file init has no permanent home (G2).** The two-host
   harness exists as LANE EVIDENCE
   (`docs/design/beamr/briefs/evidence/fix-bundle-packager-noarg/noarg-init-harness.mjs`)
   and runs nowhere permanently. Regression here would today be caught only
   by a human re-running lane evidence.
4. **No release-gate wiring.** The repo's release battery is `gates.json`
   (four legs: fmt, clippy, wasm32-check, `cargo test --workspace` —
   `gates.json:3-8`). The wasm suite's 80 walls are NOT in the battery: a
   release cut from a box without the GitHub workflow never executes them.
   "Make the NO-POLLING wall a permanent release gate" is unmet on the
   release path. (`wasm32-check` type-checks the crate; it executes
   nothing.)

### 3.3 F-0d instrumentation surface

Three observation mechanisms exist today, with three different reaches:

**(a) The counter tier (Node walls; test-cfg-gated readers).** Production
counters increment in shipping code — arbiter `CallbackCounters
{requests, queued_now, executions, cancellations}`
(`crates/beamr-wasm/src/lib.rs:555-560`, incremented at `queue_turn`
`:668-670` and `execute_queued_turn` `:712-715`), deadline
`DeadlineCounters` (`:571-585`), capability `dead_pid_completions`
(`capability.rs:159`, increments `:512`, `:541`) — but every READER is
`#[cfg(all(test, target_arch = "wasm32"))]` and crate-private:
`arbiter_counters()` `lib.rs:1856`, `unified_deadline_snapshot()` `:1863`,
`capability_counters()` `:1884`, `CapabilityBridge::counters()`
`capability.rs:276-277`. Nothing is JS-visible — the truing note's classing
(§3.8) is a direct consequence. The house "exactly one turn" idiom:
`requests+1 && queued_now==1` pre-drain, `executions+1 && queued_now==0`
post-macrotask, from the `assert_true_idle` baseline (`lib.rs:3142`).

**(b) The host-primitive seam.** The arbiter probes REAL host globals at
construction — `HostPrimitives::probe()` (`lib.rs:518-529`) captures
`queueMicrotask`/`setTimeout`/`clearTimeout` from the live global
(constructor call `lib.rs:79`). The failure suite exploits exactly this
seam: delegating throw-capable doubles installed over the named globals
BEFORE `WasmVm::new` (`failure.rs:252`, `:279`, identity-gated to the
arbiter's own callback at `:287-288`, `:302`). The same seam is what an
external F-0d shim wraps.

**(c) The external timer shim (kit; JS-side, bundle-blind).** The sitting
kit's page installs recording wrappers over
`setTimeout`/`setInterval`/`clearTimeout`/`clearInterval`/`requestAnimationFrame`
BEFORE the bundle module evaluates (`page/index.html:33-56`), exposing
`window.__sitting.timerShimEvents`. This is the only mechanism that
observes the REAL bundle (it needs no test cfg, no crate access) and the
only one honest for a real-bundle gate leg — "the shim log IS leg 3's
evidence mechanism" (`index.html:34-37`).

**The armed-future-deadline idle wall EXISTS in Node CI:**
`idle_vm_schedules_zero_recurring_callbacks` (`lib.rs:2333-2382`) asserts
counter-drift zero across a real host macrotask at true idle (`:2337-2347`)
AND with a one-hour `receive_after` deadline armed (`:2349-2382`: deadline
snapshot byte-equal, arbiter counters unmoved). The acceptance shape's
"including while only future deadlines exist" has a Node wall; what it does
not have is a real-bundle/browser twin.

### 3.4 Wake-class inventory (Node walls at `293d368`)

Every wake class the acceptance shape names has an arbiter-re-entry wall in
the 80, each driving from true idle and asserting the §3.3(a) one-turn
idiom:

| Wake class | Wall | Pin |
|---|---|---|
| Mailbox send | `idle_to_runnable_burst_queues_one_arbiter_turn` | `lib.rs:2386` |
| Cast | `cast_from_true_idle_queues_one_turn_and_coalesces_a_burst` | `lib.rs:3402` |
| Promise/async-NIF completion | `promise_completion_from_true_idle_queues_one_turn_with_durable_result` | `lib.rs:3542` |
| Capability completion | `fetch_success_from_true_idle_delivers_the_codec_map_in_one_turn` | `capability_tests.rs:388` |
| Receive timeout | `await_exit_waits_for_armed_receive_timer` | `lib.rs:2493` |
| Native deadline | `await_exit_waits_for_armed_native_deliver_timer` | `lib.rs:2910` |
| Native completion | `native_completion_envelope_wakes_parked_handler_through_the_arbiter` (+ direct-injection sibling `:3738`) | `lib.rs:3666` |
| Trapped exit | `trapped_exit_wakes_linked_supervisor_without_external_pump` | `lib.rs:3800` |
| Spawn edge | `bytecode_spawn_runs_child_under_the_cooperative_scheduler` (+ failure-leg routing proof `failure_tests.rs:301`) | `lib.rs:4021` |

Output, process error, pump/turn error, and panic surfacing likewise carry
Node walls in the 80 (the `io_sink::`, `failure::` families —
§3.1's carrier list). The conformance question WPORT-9 must answer is not
"do walls exist" but "which of these claims must ALSO be proven through the
real bundle in a real browser/Worker, and which are host-independent logic
the Node walls already pin" — that split is §4's subject.

### 3.5 Bundle production pipeline

`crates/beamr-wasm/build.rs` emits four artifacts into
`OUT_DIR/beamr-wasm-bundle/` unconditionally (empty module list allowed):
`modules.bin` (`build.rs:38`), `manifest.json` (`:39`), `bootstrap.js`
(`:40-45`, path re-exported via `cargo:rustc-env=BEAMR_WASM_BOOTSTRAP`), and
`package-bundle.mjs` (`:46`). Module sweep: `bundle_source_dir()` reads
`BEAMR_WASM_BUNDLE_DIR` (fallback `BEAMR_EMBED_DIR`, `:51-55`);
`collect_modules()` (`:57-78`) is a flat non-recursive `read_dir` that DOES
filter by extension (non-`.beam` skipped, `:62-64`) and sorts by name for
determinism (`:76`).

Bootstrap exports (`bootstrap_js()` `:153-179`, runtime tail
`BOOTSTRAP_RUNTIME` `:232-292`): `WasmVm` re-export (`:254`),
`bundledModules()` (`:256-258`), `initBeamr(wasmPathOrModule)` (`:260-263`),
`createPreloadedVm(wasmPathOrModule)` (`:265-274` — `init` argument
OPTIONAL; the no-arg call is the packaged bundle's load-bearing path),
`spawnPreloaded` (`:276-280`), `awaitExit(vm, pid)` (`:282-284`), plus a
`globalThis.BeamrBundle` attachment (`:286-291`).

Single-file packaging: `PACKAGE_SCRIPT` (`:294-337`) is a Node CLI
(`node package-bundle.mjs <wasm-pack-pkg-dir> [output]`) that reads
`bootstrap.js` + glue + `beamr_wasm_bg.wasm`, base64-encodes the wasm,
asserts and rewrites the bootstrap's exact wasm-bindgen import line
(`:320-327`), and emits `beamr.bundle.mjs` whose `importWasmBindgen()` —
fixed at `314bc7c` — builds a SELF-CONTAINED glue source (decode helper +
re-declared `WASM_BASE64` prepended) imported via
`data:text/javascript,${encodeURIComponent(source)}` (`:328`); the generated
comment documents the 2026-07-24 blob-scope defect it replaces.

The ONLY documented command sequence lives in the edge-worker README
(`examples/edge-worker/README.md:9-14`): the `BEAMR_WASM_BUNDLE_DIR=…
wasm-pack build crates/beamr-wasm --target web --out-dir pkg` step and the
generated-packager step. The sitting kit's `serve.mjs` reproduces step (a)
programmatically (`serve.mjs:39-58`: wasm-pack with
`BEAMR_WASM_BUNDLE_DIR: HANDLER_DIR`, newest-mtime bootstrap glob + copy) —
the repo's one committed, executable auto-build of the real bundle.

### 3.6 The no-arg single-file leg (lane evidence, ready to promote)

`docs/design/beamr/briefs/evidence/fix-bundle-packager-noarg/` holds the
harness plus four committed outputs (red + green, both hosts).
`noarg-init-harness.mjs` drives `createPreloadedVm()` WITH NO ARGUMENT by
name in both hosts — its header law (`:3-4`): it never passes wasm bytes,
"that approximation is how the defect stayed latent." Node leg: in-process
dynamic import via `pathToFileURL` (`:29-44`). Browser leg: stdlib HTTP
server on an ephemeral 127.0.0.1 port serving an inline module page +
`/bundle.mjs`, result POSTed back (`:47-93`); Chrome acquired via
`CHROME_BIN` env with a hardcoded macOS fallback (`:19-20`), launched
`--headless=new` (`:77-80`), 60 s timeout. Exit 0 iff BOTH legs resolve a
VM whose `spawn` surface exists. The committed red outputs pin the two
distinct host signatures (`ERR_UNSUPPORTED_ESM_URL_SCHEME` for `blob:` in
Node; `ReferenceError: decodeEmbeddedBase64` in Chrome); the greens pin
three embedded modules loading. Promotion notes: the harness is
loopback-only (no live network — G1-clean on that axis) but needs a real
browser binary for its second leg, and `CHROME_BIN` is the existing seam
for a CI-provided one.

### 3.7 Edge-worker state (the Worker-shaped consumer)

One test file, `examples/edge-worker/test/worker.test.mjs` (7 tests, Node's
built-in runner driving Miniflare directly — `package.json:9`; Miniflare
`^4.20260609.0`, `package.json:11-14`). The tests run the REAL
`src/worker.js` against an IN-MEMORY STUB bundle: `workerScript()` reads the
real worker source and string-replaces its `beamr.bundle.mjs` import with an
inline stub (`worker.test.mjs:6-97`). The stub's surface is `spawn` +
`await_exit` + the two capability registrations (`:9-85`) — the old
`run_step`/`take_exit_result`/`runUntilExit` pump surface is GONE (WPORT-2
compliance; `README.md:45` records it). This is the one `:184` boundary leg
still live: the arc's acceptance shape forbids the CONFORMANCE suite to rely
on a VM stub, and today's only Worker-shaped proof does.

Three WPORT-8 legs (A4 classing) ride the stub with REAL host edges: a KV
round-trip against a real Miniflare KV namespace (`:154-168`; capability
object `src/worker.js:106-116`), a fetch-abort leg wiring a real
AbortController (`:170-179`; `src/worker.js:82-104`), and the typed
`capability_missing` refusal→HTTP 502 mapping (`:181-192`;
`src/worker.js:45-54`). A fourth leg is the STUB-FIDELITY PIN
(`:194-248`): it regex-extracts every `pub fn` from
`crates/beamr-wasm/src/lib.rs` and asserts every `vm.<name>(` call in
`worker.js` matches an exported method by name and arity — the stub cannot
silently drift from the real surface, but it also cannot prove behavior.

**Not wired into any pipeline:** zero references to `examples/edge-worker`,
`miniflare`, or `worker.test` in `.github/workflows/` or `gates.json` —
verified repo-wide. The suite runs only via local `npm test`. The README's
own words defer the real end-to-end to the probes (`README.md:87-89`), and
its build steps (`README.md:5-24`) are the two-command bundle recipe: the
`BEAMR_WASM_BUNDLE_DIR=… wasm-pack build … --out-dir pkg` step and the
generated `package-bundle.mjs` step producing `beamr.bundle.mjs` — the
artifact whose no-arg init path G2 promotes.

### 3.8 Sitting census — what only real browsers have proven

Five probe docs, four official-run sets, every one stamped with the same
boundary sentence family — "observations … are never CI acceptance"
(THROTTLE `:9-10`, FETCH `:6-8`, FAILURE `:148-150`, CAPABILITY `:104-105`
— the last verbatim: "*(never CI acceptance — WPORT-9 keeps the permanent
gate)*"). The evidence tree: `probes/evidence/2026-07-18-live-desktop/` +
`2026-07-18-remote-display/` (WPORT-3 throttle, two environments),
`2026-07-18/` (WPORT-6 + WPORT-7 combined CDP sitting),
`2026-07-23-early-fire/` (EARLY-FIRE triage),
`2026-07-23-capability-sitting/` (WPORT-8, Tom's hands).

**Browser-only facts, by probe** (the G1 classing raw material):

- **THROTTLE** (two sittings, `:99-152`, `:156-225`): background-tab timer
  throttling magnitudes (+104 ms/+553 ms live; +954/+962 ms remote);
  intensive-throttle ZERO extra wakeups across ~344 s minimized; dedicated
  Worker timers essentially UNTHROTTLED by page backgrounding on this
  engine. Mechanism is CDP window manipulation + real engine policy —
  unreachable from Node.
- **EARLY-FIRE** (`:47-53`, `:96-117`, TORN PASS): EARLY-UNDER-CACHED-CLOCK
  is a HOST fact (libuv/loop cached ms-granularity clocks), witnessed 3/400
  samples early, worst 0.479 ms, cascade self-quenching at chain length 1.
  Its arc-wide consequence is a standing law on every future wall
  (`:143-149`): no absolute host-arm counts, no timer-promptness assertions
  across a real fire — "WPORT-9's conformance walls inherit this review"
  (`:149`).
- **FETCH** (`:71-104`): real browser `fetch()`, real status/`ArrayBuffer`
  bodies off the network, same-origin resolution, CDP network shape
  (exactly four requests, deps before dependants), real-404 typed
  rejection. Plus the provenance HONEST GAP sentence (§3.10).
- **FAILURE** (`:157-235`): `window.onerror` vs Worker dual `onerror`
  routing; the ASYNC-entry abort case (deliberately NOT a CI wall,
  `:51-61`); the latched-brick's real mechanism = wasm-bindgen borrow guard
  with the doc-inferred observables UNOBSERVABLE from JS; browser console
  interleave vs Node stream split; the timer-strand rider KILLED across all
  three environments at macrotask scale.
- **CAPABILITY** (`:109-162`): real `fetch`/`Response` header casing; real
  IndexedDB KV round trip; `net::ERR_UNSAFE_PORT` (Chrome refuses
  127.0.0.1:9 from its unsafe-port list BEFORE any connection); real
  `AbortController` killing a real in-flight request (`(canceled)` in the
  network log); F-0d shim ZERO timer events across the ~1.5 s in-flight
  window.

**The classing precedent (G1):** the CAPABILITY sitting-scope truing note
(`:70-93`, CONFIRMED by the domain owner) draws the line exactly: counter
observations are Node-wall territory (the `counters()` accessors are
`cfg(test)`-gated and `pub(crate)` — nothing JS-visible, so not citable by
sitting evidence, `:76-86`); the host-abort arm is the sitting's
cancellation claim while the death-sweep arm stays Node-proven (`:87-90`);
leg 3's mechanism is the timer-shim record, not a counter read (`:91-93`).

**Standing WPORT-9 assignments found in the corpus:** the browser-BIF
profile is "a contract consumed by WPORT-8 and WPORT-9" (arc `:125`); the
JS-visible provenance claim would earn its wall here (arc `:131`, FETCH
`:105-111`); WPORT-8's D5 explicitly reserved the permanent-CI-browser-leg
decision for WPORT-9 (`WPORT-8.json:15`, `:135`); panic surfacing is named
in the wall (FAILURE `:8-11`).

### 3.9 The release battery and the doctrine it answers to

`gates.json` at `293d368` defines the four-leg battery (fmt / clippy /
wasm32-check / workspace tests). House doctrine: gates `RUNBOOK.md` at gates
main `e6b5a43` — worktrees under `<repo>/.worktrees/`, DEFAULT target dirs,
no temp-dir builds, cold-cache standard for release evidence (clippy log
must name both closure crates), gate evidence paths must not be gitignored
(gates `8440732`), and tears pin to heads.

### 3.10 The WPORT-6 module-provenance honest gap, explicitly at this door

The arc records (`WASM-PORT-ARC.md:131`): "module-provenance an HONEST GAP
(`ModuleOrigin::Fetched` not JS-reachable; Rust/Node tests carry it; a
JS-visible provenance claim would have to earn its wall at WPORT-9)." This
pack surfaces it as TQ-owned scope (§6): in or out, decided at the tear, not
by silence.

### 3.11 The BEAMR_WASM_BUNDLE_DIR sweep hazard (banked board line)

Banked at the WPORT-8 close (disclosed 2026-07-24). Trued at the bytes: the
sweep DOES filter by extension (`collect_modules()` skips non-`.beam`,
`build.rs:62-64`) but performs NO validity check — any file named `*.beam`
has its raw bytes read (`:69`) and zstd-packed verbatim (`:90-93`), so
sweeping a directory containing `malformed_not_beam.beam` (the deliberate
WPORT-6 fixture) embeds it, and the failure surfaces only at
`vm.load_module()` runtime ("invalid BEAM file format" at first init). The
close banked hardening (manifest or validity filter) "onto WPORT-9's
conformance-gate conversation." §6 carries it as a tear question.

## 4. Decision analysis

### 4.1 The delta, stated once

The frozen scope has three demands; §3 shows exactly one is already met.
"Exercise … every wake class" etc. is met in permanent CI by the Node-side
80 (§3.1, §3.4) — those walls are host-independent logic pinned by exact
name, and nothing in this rung should re-litigate them. The unmet demands
are: (a) "using the real generated wasm bundle" — no CI leg touches any
generated artifact (§3.2 gaps 1-3); (b) "browser/Worker conformance" — no
CI leg leaves Node (§3.2 gap 2); (c) "permanent RELEASE gate" — the gates
battery never executes a wasm wall (§3.2 gap 4). WPORT-9 is therefore an
ADDITIVE rung: new legs beside the 80, not a rebuild of them.

### 4.2 The conformance matrix

Two independent axes — WHICH artifact executes, and WHICH host executes it:

| | Node | Real browser (headless) | Worker shape |
|---|---|---|---|
| **Test artifact** (crate under wasm-bindgen-test) | EXISTS — the 80 (§3.1) | possible: same suite under a browser runner (§4.3-B) | — |
| **Real bundle** (wasm-pack pkg + generated `bootstrap.js`) | possible (driver imports pkg; §4.3-C) | possible: the kit-derived shape (§4.3-C) | possible: page-spawned Worker (§4.3-D) |
| **Packaged single-file** (`beamr.bundle.mjs`, NO-ARG init) | harness leg 1, ready (§3.6) | harness leg 2, ready (§3.6) | possible: real bundle under Miniflare replacing the stub (§4.3-D) |

The scope's operative words pick rows two and three: "packages and
instantiates `beamr_wasm_bg.wasm` … does not replace the VM with a JS
stub." Which CELLS constitute the gate is the tear's decision (TQ1); the
candidates below are ordered by how much standing asset each rides on.

### 4.3 Candidate legs and what each earns

**A. Release-gate wiring (gap 4; no new proof, pure permanence).** Add the
wasm suite to `gates.json` as its own leg (the workflow's `:34-37` command
verbatim: `wasm-bindgen-test-runner` over `cargo test` for the crate). This
single move makes the EXISTING F-0d idle wall — including the
armed-future-deadline half (§3.3) — a release gate, satisfying the scope's
last sentence for the Node tier before any new wall is written. Cost: the
release box needs wasm-bindgen-cli (version-pinned; the workflow pins
0.2.123) — fail-loud if absent, per the gates doctrine (evidence paths and
tools are never silently skipped).

**B. The suite under a real engine.** `wasm-bindgen-test-runner` can drive
a real headless browser (chromedriver seam) instead of Node. One
configuration flip re-proves ALL 80 walls — every counter, every wake
class, the armed-deadline idle wall — on a real engine. This is the
cheapest possible browser conformance per wall, but it exercises the TEST
artifact, not the generated bundle, so it cannot discharge (a) alone.

**C. The real-bundle conformance driver (the rung's centerpiece).** A
committed harness in the noarg-harness mold (driverless: spawn headless
Chrome, serve loopback, page POSTs machine-checkable results — §3.6's
pattern, deliberately NOT the uncommitted CDP driver, §5) that: builds the
pkg programmatically (serve.mjs precedent, §3.5), embeds a dedicated
conformance workload (committed `.erl` + `.beam`, house fixture pattern),
installs the F-0d shim BEFORE the bundle evaluates (kit precedent,
§3.3(c)), drives the workload through the settlement-event loop (the
settled-idle contract — timer-driven probing would pollute the shim record
and is itself the polling the gate exists to kill), and asserts: real init,
real `.beam` execution, one exit map carrying every claimed surface, and
ZERO shim events across the idle and armed-deadline windows.
EARLY-UNDER-CACHED-CLOCK governs: window claims are "zero events between
settlement points," never arm-count or promptness assertions (§3.8). The
same driver runs the workload in Node against the single-file bundle,
subsuming G2's promotion (§3.6) — or the noarg harness promotes standalone
alongside it (TQ2).

**D. The Worker shape.** Three candidate readings of "browser/Worker
conformance" (TQ3): (i) the C-harness page also spawns a dedicated Worker
running the same bundle + workload (the FAILURE sitting's Worker legs, made
permanent and driverless); (ii) the edge-worker suite runs under Miniflare
with the REAL `beamr.bundle.mjs` substituted for the stub — one leg
retiring the acceptance shape's stub complaint AND proving the packaged
artifact in the platform class that motivated WPORT-8's selection; (iii)
both. Option (ii) makes CI depend on the Miniflare npm tree (§5).

### 4.4 The three-tier classing (G1, generalized from the truing note)

Every conformance claim in the acceptance shape falls into exactly one
tier, and the brief must place each explicitly:

1. **Hermetic, Node-wall tier** — host-independent logic: counters,
   one-turn arithmetic, vocabulary closure, error legs, ordering. Already
   pinned by the 80; the gate change is wiring (A), not new walls.
2. **Hermetic, real-engine tier** — claims about the generated artifacts
   and engine-generic behavior, provable on a workflow-provisioned browser
   with loopback-only network: init paths, real `.beam` execution through
   the bundle, F-0d shim silence, panic reaching the page surface,
   output sink delivery. Legs B/C/D live here.
3. **Sitting territory** — engine POLICY and environment facts: throttle
   magnitudes and thresholds, Worker-timer throttling divergence,
   `ERR_UNSAFE_PORT` and browser-authored detail texts, real IndexedDB
   backends, console interleave, the ASYNC-entry abort case (already ruled
   non-CI), multi-engine variation. These stay probe-attached observations
   with the "never CI acceptance" stamp — the gate must not assert them,
   and the brief records WHY per G1 (§3.8's census is the ledger's raw
   material).

### 4.5 What "permanent" must mean mechanically

Whatever cells TQ1 selects inherit the house carrier discipline: exact-name
+ exact-count carriers moving with names per commit (§3.1), fail-loud on
absent tools/browsers (never skip-and-pass — a browser leg that silently
downgrades to Node is a stub in disguise), and version echoes in the log
(the workflow already echoes toolchain versions, `:26-30`). A leg that can
be skipped is not a gate.

## 5. Hazards and boundaries

- **The uncommitted CDP driver.** THROTTLE/FETCH/FAILURE cite a "committed
  zero-dep CDP driver at `harness/`" — verified ABSENT from main at
  `293d368` (git ls-files; the docs' claim is untrue at the bytes). The
  dir exists untracked on the operator's checkout; its resolution is the
  domain owner's open call (flagged 2026-07-24). WPORT-9 must not cite or
  depend on it: the committed browser-driving precedent is the noarg
  harness's driverless spawn+POST pattern (§3.6).
- **Panic legs brick the VM** (borrow-guard latch, FAILURE `:190-205`):
  any real-bundle panic leg takes a fresh VM per leg and asserts only
  JS-observable surfaces; the ASYNC-entry abort case stays out of CI by
  standing ruling (FAILURE `:51-61`).
- **Shim self-observation:** the C-harness must route its OWN timers
  through saved originals (kit precedent, `index.html:39-45`) or the shim
  record indicts the harness, not the bundle.
- **EARLY-UNDER-CACHED-CLOCK** is inherited review on every new wall
  (EARLY-FIRE `:143-149`): no absolute host-arm counts, no promptness
  assertions across real fires — in Node AND browser legs.
- **Settled-idle contract:** `await_exit` resolves `"idle"` for a process
  parked on a capability op; every real-bundle driver uses the
  settlement-event loop. A driver that polls is a tear condition twice
  over.
- **Browser version drift:** CI proves "a real engine, version echoed,"
  not an engine matrix; multi-engine and throttle policy remain sitting
  territory (tier 3). Chrome on runners is unpinned upstream — the leg
  logs the version it got and asserts none of tier 3.
- **npm supply surface:** option D(ii) puts Miniflare's dependency tree on
  the gate path; today `gates.json`/workflow have zero npm dependencies.
  Tear decision, not a default.
- **Exited/errored vocabulary is NOT this rung's.** WPORT-7's board
  finding (arc `:146`) assigns uncaught-native-BIF classification "to a
  future brief that owns the exited/errored vocabulary" — WPORT-9
  exercises process-error paths but does not own that reclassification;
  pulling it in requires the domain owner's word, else it stays banked.
- **Evidence immutability and carriers:** committed sitting evidence is
  never rewritten; new legs' counts get their own carriers; the 80-name
  array moves only with names.
- **Toolchain provisioning:** wasm-pack enters CI for the first time in
  candidates C/D; version-pin and echo per §4.5. Build-dir doctrine per
  gates RUNBOOK (§3.9) applies to any harness build step.

## 6. Open tear questions

**TQ1 — Gate composition.** Which cells of the §4.2 matrix constitute the
permanent gate? The pack's read of the frozen scope: A (release wiring) +
C (real-bundle browser driver) are non-optional — (a) and (c) are unmet
without them; B and D are the leverage/scope dials. Does the domain owner
ratify A+C as the floor, and where do B and D land?

**TQ2 — G2's home.** Does the no-arg single-file leg live INSIDE the C
harness (one driver, three artifacts) or does `noarg-init-harness.mjs`
promote standalone beside it? One driver is less surface; standalone keeps
the lane evidence's exact red/green semantics untouched.

**TQ3 — The Worker reading.** Which of §4.3-D's three readings satisfies
"browser/Worker conformance" — page-spawned Worker (i), real bundle under
Miniflare (ii), or both? (ii) retires the stub complaint but imports the
npm hazard; (i) stays zero-dep but proves a browser Worker, not the
platform class.

**TQ4 — Release-battery shape.** What exactly lands in `gates.json`: the
wasm-suite leg (A) only, or also the conformance harness? And on boxes
without a browser: fail-loud (gate refuses to pass) or is the browser leg
workflow-only with the battery carrying the Node tier? The pack leans
fail-loud-everywhere per §4.5, but the battery runs on human dev boxes —
the domain owner's call.

**TQ5 — The conformance workload.** A dedicated committed `.erl`+`.beam`
conformance handler exercising the acceptance shape's named surfaces from
bytecode (house fixture pattern, kit-handler precedent) — ratify, or reuse
existing fixtures? And the §4.4 tier split: does the domain owner ratify
the tier-1/tier-2 placement per claim (the brief would carry the full
per-claim table), with tier-3 as the G1 ledger?

**TQ6 — Sweep hardening.** The §3.11 validity gap: in scope as an R-number
(the C harness embeds a known-good workload anyway, and a manifest/validity
filter is one function at `collect_modules()`), or re-banked with
reasoning? The banked line says it "rides WPORT-9's conformance-gate
conversation" — this is that conversation.

**TQ7 — Module provenance.** Arc `:131` reserved the JS-visible
`ModuleOrigin::Fetched` claim's wall for WPORT-9. In (new JS-visible
surface + wall in the C harness) or explicitly re-banked (no consumer
requirement yet — the WPORT-10 discipline)? The pack notes: adding a JS
surface touches `crates/beamr-wasm` production code; everything else in
this rung is harness/CI/docs territory. A rung that otherwise ships zero
production-code motion is a smaller tear.

**TQ8 — The classing ledger's authority.** G1 requires the
sitting-territory reasoning RECORDED. Does the ledger live in the brief
itself (tier-3 table with per-claim reasoning, §3.8 census as source), or
as a standing section in the arc doc where future rungs inherit it without
re-derivation?
