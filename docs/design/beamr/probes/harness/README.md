> **Committed copy (2026-07-18, Annabel's word):** this harness lives on main so any
> seat can run the probe sittings — the original single-copy lived in Artemis's
> artifacts dir. Path variables at the top of `build-bundle.sh` (REPO, WBG) are
> machine-specific: adapt them to your checkout and your wasm-bindgen-0.2.123
> install. For a **WPORT-3-only** sitting no panic source is needed — build the
> bundle CLEAN from main (skip the panic-source.diff step entirely):
> `cargo build --release --target wasm32-unknown-unknown -p beamr-wasm --locked`
> then `wasm-bindgen <target>/wasm32-unknown-unknown/release/beamr_wasm.wasm
> --target web --no-typescript --out-dir web/pkg` (CLI must print 0.2.123),
> then `erlc` nothing — the workload `.beam`s are committed — and stage
> `workloads/*.beam` + `crates/beamr-wasm/fixtures/fetch_*.beam` into
> `web/artifacts/`. Observations land in `observations/` beside the runners.

# Probe-sitting harness — WPORT-6 / WPORT-7 / WPORT-3

A zero-dependency (Node stdlib only) browser + Worker harness that sits the
three OPEN beamr wasm probes against the **real generated `beamr-wasm` bundle**:

- `WPORT-6-PROBE-FETCH` — real-browser artifact-loader run
- `WPORT-7-PROBE-FAILURE` — failure/panic surfacing + the §2 timer-strand rider
- `WPORT-3-PROBE-THROTTLE` — platform timer-throttling (backgrounded tab + Worker)

Observations are **evidence attached to those probe artifacts — never CI
acceptance** (the driver asserts no timing wall). This directory is the
harness author's build + smoke; the **official evidence runs happen at the
operator's hands** (Tom / Annabel), and overwrite the `"smoke": true`
observations here.

> Built and smoke-tested by the harness author. WPORT-6 and WPORT-7 smoke
> GREEN end-to-end headless. WPORT-3 is **headed** (real backgrounding on the
> operator's display) and was NOT run here by design.

---

## File map

```
driver.mjs               Zero-dep CDP driver (server, Chrome launch, page/Worker attach, backgrounding)
runner-common.mjs        bootProbe()/withTimeout() over driver.mjs (shared by the runners)
build-bundle.sh          Rebuild the wasm bundle + compile workloads + stage artifacts
run-wport6.mjs           WPORT-6 runner (headless)
run-wport7.mjs           WPORT-7 runner (headless; main-thread legs + Worker 1b; --strand-worker mode)
run-wport3.mjs           WPORT-3 runner (HEADED; operator-only) + --worker mode
run-strand-node.mjs      WPORT-7 §2 strand — NODE baseline (no Chrome)
panic-source.diff        The UNCOMMITTED scratch panic BIF edit (base a399b54) — see disclosure
panic-source.base-commit.txt

web/
  probe-common.js        Shared page JS: timer spies, bundle load, obs channel, onerror recorder, runStrandSamples()
  wport6.html            WPORT-6 page (fetch loader, expected observations 1-5)
  wport7.html            WPORT-7 page (legs 1a/1c/1d/1e + §2 strand, main thread)
  wport7-worker.html     WPORT-7 leg 1b owner page (Worker panic surfacing)
  wport7-strand-worker.html  WPORT-7 §2 strand owner page (Worker environment)
  worker.js              Worker body for 1b + WPORT-3 run (b) + §2 strand
  wport3.html            WPORT-3 page (both deadline classes; ?mode=worker for run (b))
  pkg/                   Generated bundle (beamr_wasm.js + beamr_wasm_bg.wasm) — wasm-bindgen 0.2.123
  artifacts/             Served over HTTP: manifest.json, manifest_bad.json, all .beam files

workloads/               Fresh-authored .erl + compiled .beam (OTP 29), kept side by side
  panic_probe.erl        boom/0 (sync panic), wait_boom/0 (queued-turn panic)
  throttle_probe.erl     wait30/0 (receive-after 30s), deliver45/0 (native send_after 45s, self-armed)
  strand_probe.erl       run/0 (mid-turn send_after(0,self(),probe))
  io_probe.erl           interleave/0 (interleaved out/err via gleam_stdlib print family)

observations/            JSON written by the runners (this dir holds the smoke run)
```

## Build

Prereqs on PATH: OTP 29 `erlc`, Rust `wasm32-unknown-unknown`, and the pinned
`wasm-bindgen` 0.2.123 at `artemis-artifacts/tools/wbg-0.2.123/bin` (the global
`~/.cargo/bin/wasm-bindgen` is never touched). Then:

```
./build-bundle.sh
```

which, verbatim:

1. Builds in the scratch worktree with the shared target dir:
   `cd .worktrees/probesitting && CARGO_TARGET_DIR=<repo>/target cargo build \
    --release --target wasm32-unknown-unknown -p beamr-wasm --locked`
   (the worktree holds `panic-source.diff` applied **uncommitted** at base
   `a399b54`).
2. `wasm-bindgen --version` prints **0.2.123**, then generates the web bundle:
   `wasm-bindgen target/wasm32-unknown-unknown/release/beamr_wasm.wasm \
    --target web --no-typescript --out-dir web/pkg`.
3. `erlc` compiles the four workloads (OTP 29).
4. Stages the workload `.beam` copies and the five WPORT-6 fetch fixtures into
   `web/artifacts/`.

## Run each leg

All runners serve `web/` over HTTP from `127.0.0.1` and drive Chrome
(`/Applications/Google Chrome.app`) over CDP. Observations land in
`observations/<probe>-<leg>.json` (`"smoke": true` for author runs; pass
`--official` for the operator's overwrite).

```
node run-wport6.mjs                  # headless: report, run(=42), network, bad-url rejection, provenance gap
node run-wport7.mjs                  # headless: §2 strand + 1a/1c/1d/1e (main) then 1b (Worker)
node run-wport7.mjs --strand-worker  # headless: §2 strand INSIDE a dedicated Worker
node run-strand-node.mjs             # NODE baseline: §2 strand on the bundle under plain Node (no Chrome)
node run-wport3.mjs                  # HEADED, run (a): arm both classes, background 6.5min, foreground, settle
node run-wport3.mjs --worker         # HEADED, run (b): same workload inside a dedicated Worker
```

WPORT-3 knobs (env): `WPORT3_BACKGROUND_MS` (default 390000 = 6.5 min),
`WPORT3_GRACE_MS` (default 120000 post-foreground). Do **not** run WPORT-3
headless — real platform throttling requires a real window on the operator's
display.

### Smoke results (harness author, headless)

| Probe | Leg | Result | Note |
|---|---|---|---|
| WPORT-6 | report | GREEN | `ok:true`, loaded order `c,b,a`, empty unresolved/deferred/denied/cycles/missing |
| WPORT-6 | run | GREEN | `fetch_chain_a:run/0` settles `exited` with result **42**, zero manual pumps |
| WPORT-6 | network | GREEN | CDP log shows the manifest + 3 artifacts (deps before dependants); each fetched once |
| WPORT-6 | bad-url rejection | GREEN | `artifact_fetch_failed: 404`; `data` names artifact `fetch_chain_a`, the bad URL, stage `fetch`, loaded `[c,b]` |
| WPORT-6 | module provenance | GAP | `ModuleOrigin::Fetched` not reachable from JS — recorded honestly (see deviations) |
| WPORT-7 | §2 strand (browser-main) | GREEN | 20 samples, all `exited got_probe`; p50 ≈ 5 ms, max ≈ 5 ms, `armsDuringWait`≡1 → one arm→one fire |
| WPORT-7 | §2 strand (Worker) | GREEN | 20 samples, all `got_probe`; p50 ≈ 5.8 ms, max ≈ 6.2 ms, `armsDuringWait`≡1 |
| WPORT-7 | §2 strand (Node baseline) | GREEN | 20 samples, all `got_probe`; p50 ≈ 1.4 ms, max ≈ 4.5 ms, `armsDuringWait`≡1 |
| WPORT-7 | 1a caught | GREEN | caught `RuntimeError`; panic callback fires BEFORE the trap with message + location; `console.error` line captured |
| WPORT-7 | 1a uncaught | GREEN | `window.onerror` observes the trap; callback fired first |
| WPORT-7 | 1c queued-turn | GREEN | `send_message` returns; trap surfaces at `window.onerror`; callback first; VM then bricked |
| WPORT-7 | 1d latched-brick | GREEN* | every call re-traps; fresh VM works — *but refines the spec (see below) |
| WPORT-7 | 1e console order | GREEN | browser devtools console shows ONE interleaved out/err timeline; ordered-sink opt-in delivers in order |
| WPORT-7 | 1b Worker | GREEN | worker `self.onerror` AND owner-page `worker.onerror` fire; panic callback fires first |
| WPORT-3 | (a)/(b) | NOT RUN | headed — operator's run; harness verified arming + wiring only |

`GREEN*` = works and is honest, but the on-platform behavior **refines** the
probe's written inference — read the 1d deviation.

---

## Panic-source disclosure (WPORT-7 §1)

WPORT-7 §1 needs a real SYNC-entry panic. The shipped surface has none (by
design), so the harness adds a **minimal, UNCOMMITTED** scratch edit in the
probesitting worktree:

- **Diff:** `panic-source.diff` (35 lines, `crates/beamr-wasm/src/lib.rs` only)
- **Base commit:** `a399b544295f5bae4a09a2c80569f37570138281` (`panic-source.base-commit.txt`)
- **NEVER committed.** It adds `WasmVm::install_probe_panic_bif(&self)`, which
  registers a native BIF `probe:panic_now/0` through the ordinary
  `bif_registry.register(...)` seam — mirroring the cfg(test)
  `panicking_test_bif` in `failure_tests.rs` — panicking with the identical
  message `panic!("wport7 intentional panic wall probe")`. The workload calls
  `probe:panic_now/0` directly (sync leg) or from a queued receive turn (async
  leg). `install_probe_panic_bif()` must be called before loading the workload
  so the import resolves at load time.

To reproduce the bundle the pages load, the diff must be applied in the
worktree before `./build-bundle.sh` (it is already applied there).

## Spy-observation rationale (why setTimeout/clearTimeout spies)

`UnifiedDeadlineSnapshot` freight (`next_arms`, `rearms_earlier`, `executions`,
`cancellations`) is **`cfg(test)`-only** — it is not compiled into the shipped
bundle (per `WPORT-7-PROBE-FAILURE.md:116-119`). So on the real bundle the
**only** observation channel for host timer activity is to wrap
`globalThis.setTimeout` / `clearTimeout` **before** constructing any `WasmVm`
(`HostPrimitives::probe` captures those globals at construction, so the VM
captures the wrappers). Every page installs the spies first and records arm
delays, clear counts, and handles; the summaries ride in each observation. The
unified deadline service arms exactly **one** `setTimeout` at the earliest
deadline — the smoke confirms a single `30000ms` arm for the WPORT-3 dual-class
workload, and one-arm→one-fire for the strand.

---

## Deviations and gaps (no silent scope-drops)

1. **WPORT-6 observation 5 — module_info provenance is a GAP.**
   `ModuleOrigin::Fetched` is set by `load_module_with_origin` in core, but no
   `#[wasm_bindgen]` surface exposes `module_info` / `ModuleOrigin`; the batch
   report and `load_module` JSON carry no origin field. Provenance is verified
   in Rust/Node tests, not observable from the browser. Recorded as
   `wport6-module-provenance-gap.json` with `reachableFromJs: false`.

2. **WPORT-7 leg 1d — the platform REFINES the spec.** The probe text infers
   that on the bricked VM `terminal_error()` "still answers `null`" and
   `await_exit` "hangs". On this wasm-bindgen toolchain neither is observable:
   the aborted `&mut self` borrow from the panicking `run_step` leaves the JS
   object's wasm-bindgen borrow guard stuck, so **every** method on the bricked
   object (`run_step`, `send_message`, `terminal_error`, `await_exit`) re-traps
   **synchronously** with `"recursive use of an object detected which would
   lead to unsafe aliasing in rust"`. `terminal_error()` never reaches its
   `null` return; `await_exit()` never constructs a hanging promise. The
   arbiter-internal reasoning (`last_error = None`, stuck `Draining`) is real
   but **unobservable from JS** because the wrapper guard intercepts first. A
   fresh `WasmVm` works. This is a faithful on-platform finding, not a harness
   bug — route it to the board alongside the existing recovery-contract text.
   (Confirmed identically in the Node baseline; the operator's browser run
   records whatever the real engine does.)

3. **WPORT-3 — a pid cannot cross the JS `spawn(module, fn, args_json)`
   boundary.** JSON/JS numbers marshal to integer terms and `Tag::Pid` is
   unconvertible (`convert.rs`), so `erlang:send_after(45000, TargetPid, ...)`
   with a JS-passed pid is impossible. The native Deliver class therefore
   **self-arms** (`deliver45/0` = `send_after(45000, self(), deliver)` then
   parks) — faithful to "a native Deliver timer targeting a parked receive
   process", within one process. `park/0` + `arm/1` are retained in
   `throttle_probe.erl` for documentation but are unused by the page.

4. **Atoms cannot cross `send_message`.** JS strings marshal to **binaries**;
   there is no atom encoding on that boundary (`convert.rs`). So the strand's
   outer receive matches **any** message (`_Go`) rather than the atom `go`;
   the inner `probe` and the throttle `deliver` are real atoms delivered by
   `send_after` and match normally. Externally-triggered clauses in the
   workloads are written to match any message for this reason.

5. **`spawn` auto-queues a microtask drain.** `spawn`/`send_message` call
   `schedule_external_edge`, which queues an arbiter turn via `queueMicrotask`.
   For 1a-uncaught this means the boom process is driven by that automatic
   microtask turn (its uncaught trap routes to `window.onerror`), in addition
   to the page's explicit `setTimeout(run_step)`. The observation still holds:
   `window.onerror` observes the trap, callback first.

6. **beamr wasm `send_message` to a dead/unknown pid throws `Badarg`**, unlike
   BEAM's silent drop. The strand phase-variation was originally driven by
   ordinary sends to a filler process; that process exits when its `send_after(0)`
   delivers, and the next send hit `Badarg`. Phase is now varied by cycling
   host macrotask hops before the measured arm — no beam filler — which is the
   meaningful variation for the wheel-cursor hypothesis anyway.

7. **Smoke ≠ official.** All observations here carry `"smoke": true`; the
   operator overwrites with `--official`. No boarded line is closed by this
   smoke.

### §2 strand confirm-or-kill MATRIX (all three environments)

The ruling requires all three environments; the harness now measures the
IDENTICAL 20-sample strand (shared `runStrandSamples()` in `probe-common.js`)
in each. Smoke numbers:

| Environment | file | p50 | max | armsDuringWait | got_probe |
|---|---|---|---|---|---|
| browser main-thread | `wport7-strand.json` | ≈5.1 ms | ≈5.2 ms | ≡1 (all 20) | 20/20 |
| dedicated Worker | `wport7-strand-worker.json` | ≈5.8 ms | ≈6.2 ms | ≡1 (all 20) | 20/20 |
| Node baseline | `wport7-strand-node.json` | ≈1.4 ms | ≈4.5 ms | ≡1 (all 20) | 20/20 |

Every sample in every environment: one `setTimeout` arm → one fire → one
delivery, latency macrotask-scale (1–6 ms), **nowhere near a ~1s wheel
revolution and zero re-arm churn** — the **KILL** criteria across the full
matrix. This SMOKE meets kill on all three; the boarded wasm-exposure line
closes on the operator's `--official` re-run of the same three modes (the
native-side finding stands alone, per the ruling). No `crates/beamr/src/timer.rs`
observation or edit is implied — cert-resign territory, untouched.

## Laws honored

Zero new dependencies (Node stdlib only, no `npm install`); nothing committed
to any repo; edits confined to the probesitting worktree and this harness dir;
`CARGO_TARGET_DIR` always the main repo `target`; the global `wasm-bindgen`
untouched (pinned 0.2.123 used from the tools dir); `.worktrees/embbuild` never
touched.
