# WPORT-3 PROBE-THROTTLE — platform timer-throttling prototype

**Status:** `PROBE-THROTTLE: NOT RUN — UNVERIFIED-ON-PLATFORM`
**Authored by:** WPORT-3 (tear Ruling 3, 2026-07-14: the prototype is authored,
NOT run, in that brief)
**Arc-board line:** the arc's deadline pillar (WASM-PORT-ARC WPORT-3) remains
**OPEN** until one real browser+Worker run attaches observations to this
artifact — a ~15-minute manual run against the built bundle, post-land, on
Tom's or Annabel's word. Later observations are appended here as evidence
attached to this artifact; they are **never** CI acceptance.

## Why this probe exists

Tear condition T2, quoted from the governing paper
(`docs/design/beamr/WASM-PORT-ARC.md:81-87`):

> Browsers throttle timers in backgrounded tabs — `setTimeout` clamping at
> minutes scale exists in the wild, and Worker contexts differ again — so a
> one-shot armed at the earliest deadline delivers LATE under throttling, by
> design of the platform.

The unified deadline service (WPORT-3 R1) arms exactly one `setTimeout` at the
earliest known deadline across receive-after and native `Deliver` timers. Node
acceptance proves deterministic late due-set completeness, id/token stale-fire
safety, cancellation outcomes, one arbiter request per admitted fire, and
next-deadline selection. Node does **not** prove background-tab clamp amounts,
Worker scheduling differences, or real-platform promptness. Those are this
probe's territory, and BEAM receive-after semantics tolerate late fire — the
probe makes that **stated** rather than believed.

The acceptance law is equally binding in the other direction, quoted from the
same lines: "No acceptance test in this brief may assert timing precision that
flakes under throttling, and no liveness claim rides deadline promptness."
This probe asserts *late-but-delivered and complete*, never promptness.

## Prototype specification — two manual runs

Both runs use the **real generated wasm bundle** (the `beamr-wasm` build
output loaded through its generated bootstrap), not a JS stub and not the Node
test runner. Workload for both contexts:

1. Construct a `WasmVm` and load a module (or use the host seams) so that the
   VM parks with **both** deadline classes pending:
   - one receive-after timer (`receive ... after` / the wrapper receive map),
     requested at **T+30s**;
   - one native `Deliver` timer (`erlang:send_after/3` from bytecode),
     requested at **T+45s**, targeting a parked receive process.
2. Record the requested deadlines and the unified service's armed deadline.
3. Background/throttle the context (below) for **at least 5 minutes** —
   comfortably past both deadlines and past known minute-scale clamps.
4. Foreground/unthrottle. Record when the one deadline callback actually ran,
   what it delivered, and what the service armed next.

### Run (a) — backgrounded tab

Open the bundle in a normal browser tab, arm the workload, then switch to a
different tab (and ideally minimise the window) for the throttle interval.
Expectation under the arc: the one-shot fires LATE (possibly minutes late),
and when it fires the **complete** due set delivers — both classes, exactly
once each — and the next known deadline (if any) is armed.

### Run (b) — Worker context

Run the same workload inside a dedicated `Worker` owned by a page, then
background/throttle the owning page for the same interval. Worker timer
throttling differs by engine; the expectation is identical: late-but-delivered
and complete, no lost timers, no duplicate delivery, no recurring callbacks.

## Observations to capture (per run)

| Field | Value |
| --- | --- |
| Date (UTC) | _unfilled — probe not run_ |
| Operator | _unfilled_ |
| Browser + version | _unfilled_ |
| OS + version | _unfilled_ |
| Bundle commit (`git rev-parse HEAD`) | _unfilled_ |
| Requested deadlines (receive, native) | _unfilled_ |
| Armed unified deadline at background time | _unfilled_ |
| Backgrounded interval | _unfilled_ |
| Observed callback time(s) vs requested | _unfilled_ |
| Delivered identities + counts (must be complete, exactly once) | _unfilled_ |
| Next arm after settle | _unfilled_ |
| Late-but-complete? (yes/no + notes) | _unfilled_ |

## What this probe is not

- **Not CI.** No workflow runs it; no acceptance test references it. The
  eighteen-name Node inventory in `.github/workflows/cooperative-wasm.yml` is
  the executed gate and contains no browser/Worker claim.
- **Not a promptness benchmark.** A clamp measurement is context, not a wall;
  no repository test may consume it as a bound.
- **Not optional bookkeeping.** Until observations are attached here, every
  browser/Worker deadline behaviour claim in this repository remains
  UNVERIFIED-ON-PLATFORM and the arc's deadline pillar stays OPEN.

---

## 2026-07-18 official sitting — live desktop (second environment)

Operator: Waffles the Terrible, at Tom's live desktop on his word (headed run
watched by Tom in person — display context: active, unlocked, physical
display). Harness: the committed zero-dep CDP driver at `harness/` (main @
`b68b596`); bundle built CLEAN from main at the operator's hands
(wasm-bindgen 0.2.123, no panic source — WPORT-3 needs none). Raw
observation JSONs committed beside this record under
`evidence/2026-07-18-live-desktop/` (same-commit standard).

### Run (a) — backgrounded tab, main-thread VM

| Field | Value |
| --- | --- |
| Date (UTC) | 2026-07-18T04:56:54Z (settled) |
| Operator | Waffles the Terrible (live desktop, Tom observing) |
| Browser + version | Google Chrome 150.0.7871.128 |
| OS + version | macOS 26.3.1 (arm64) |
| Bundle commit (`git rev-parse HEAD`) | `b68b596` |
| Requested deadlines (receive, native) | 30 000 ms receive-after; 45 000 ms native deliver (rides behind, re-arms after the 30 s fires) |
| Armed unified deadline at background time | 30 000 ms (one one-shot; backgrounded immediately on arm: real second-tab activation + `Browser.setWindowBounds` minimize) |
| Backgrounded interval | 390 s (past the ~300 s intensive-throttle threshold) |
| Observed callback time(s) vs requested | receive-after fired at 30 104 ms (+104 ms); native-deliver fired at 45 553 ms (+553 ms) |
| Delivered identities + counts | both classes delivered exactly once (`bothDeliveredExactlyOnce: true`); fire timeline carries exactly 2 fires; zero further wakeups across the remaining ~344 s of minimized intensive throttle |
| Next arm after settle | none — no known deadline remained; zero recurring callbacks observed post-settle |
| Late-but-complete? | YES (`lateButComplete: true`) — receive-after correctly `timed_out`, native deliver correctly `got_it`; no lost timers, no duplicates |

### Run (b) — same workload in a dedicated Worker

| Field | Value |
| --- | --- |
| Date (UTC) | 2026-07-18T05:03:26Z (settled) |
| Operator | Waffles the Terrible (live desktop) |
| Browser + version | Google Chrome 150.0.7871.128 |
| OS + version | macOS 26.3.1 (arm64) |
| Bundle commit | `b68b596` |
| Requested deadlines (receive, native) | 30 000 ms receive-after; 45 000 ms native deliver |
| Armed unified deadline at background time | 30 000 ms; owning page backgrounded + minimized on arm |
| Backgrounded interval | 390 s |
| Observed callback time(s) vs requested | receive-after fired at 30 008 ms (+8 ms); native-deliver fired at 45 003 ms (+3 ms) — dedicated-Worker timers essentially unthrottled by page backgrounding on this engine |
| Delivered identities + counts | both classes delivered exactly once (`bothDeliveredExactlyOnce: true`); 2 fires total; zero further wakeups |
| Next arm after settle | none; zero recurring callbacks post-settle |
| Late-but-complete? | YES (effectively on-time) — same completion identities as run (a) |

**Verdict against the probe's stated expectation:** late-but-delivered and
complete in both contexts — no lost timers, no duplicate delivery, no
recurring callbacks under intensive throttle. BEAM receive-after semantics
tolerated the late fire exactly as stated. The main-thread +104 ms/+553 ms
deltas show mild background throttling (fires landed before the intensive
threshold); the decisive intensive-throttle evidence is the ZERO additional
wakeups across the remaining minimized interval, both contexts. A parallel
same-day sitting ran on a remote Tailscale display at Annabel's machine
(Artemis's record, appended separately with its own display-context
disclosure) — a deliberate two-environment close.

---

## 2026-07-18 official sitting — remote Tailscale display (second environment)

Operator: Artemis Peach, at Annabel's machine on her acknowledged window
(Waffles's rider). **Display-context disclosure:** Annabel was remote via
Tailscale for the whole sitting and could not see the Chrome windows; the
console's physical display may have been locked or asleep. The run is
CDP-automated end to end (headed Chrome launched by the committed driver,
backgrounding = real second-tab activation + `Browser.setWindowBounds`
minimize); no human watched the windows — the acknowledged window was the
consent, not observation. This context is the deliberate complement to the
live-desktop sitting above: one active unlocked display, one remote
possibly-asleep display, same harness, same workload.

**Bundle provenance + discarded first run:** bundle built CLEAN from main @
`b68b596` at the operator's hands (`cargo build --release --locked` +
side-root wasm-bindgen printing `0.2.123`; greps for the WPORT-7 panic
source over both `beamr_wasm.js` and `beamr_wasm_bg.wasm` returned zero
matches). A FIRST run-pair earlier the same hour was completed but
DISCARDED before recording: it had been served the WPORT-7 sitting bundle
(a399b54 + the disclosed panic-source diff — `install_probe_panic_bif`
present in the JS glue), violating this harness README's clean-build
runbook for WPORT-3. Its observables were behaviorally consistent with the
clean rerun below (exactly-once delivery, correct re-arm, near-on-time
Worker fires); the raw JSONs are retained uncommitted at the operator's
artifacts directory (`observations/discarded-run1-wport7bundle/`), not
citable as evidence. Everything below is the clean-bundle rerun. Raw JSONs
committed beside this record under `evidence/2026-07-18-remote-display/`
(same-commit standard).

### Run (a) — backgrounded tab, main-thread VM

| Field | Value |
| --- | --- |
| Date (UTC) | 2026-07-18T05:07:02Z (settled) |
| Operator | Artemis Peach (CDP-automated; remote display per disclosure above) |
| Browser + version | Google Chrome 150.0.7871.125 |
| OS + version | macOS 26.5.2 (arm64) |
| Bundle commit (`git rev-parse HEAD`) | `b68b596` |
| Requested deadlines (receive, native) | 30 000 ms receive-after; 45 000 ms native deliver |
| Armed unified deadline at background time | 30 000 ms — exactly one deadline-scale one-shot (`unifiedArmMatchesEarliest: true`; spy at arm: 1 deadline-scale arm, 0 clears); backgrounded immediately on arm |
| Backgrounded interval | 390 s (past the ~300 s intensive-throttle threshold) |
| Observed callback time(s) vs requested | receive-after fired at 30 954 ms (+954 ms); native-deliver fired at 45 962 ms (+962 ms) — ~1 s background timer-alignment throttling, both fires while still backgrounded |
| Delivered identities + counts | both classes delivered exactly once (`bothDeliveredExactlyOnce: true`); pid 1 `timed_out`, pid 2 `got_it`; fire timeline carries exactly 2 fires |
| Next arm after settle | after the first fire, ONE re-arm at 14 014 ms matching the scheduler's reported `next_native_deadline_ms` 14 013.6; after the second fire no known deadline remained and no further arm occurred — final spy 3 arms / 0 clears total, zero additional wakeups across the remaining ~344 s minimized |
| Late-but-complete? | YES (`lateButComplete: true`) — no lost timers, no duplicates, no recurring callbacks |

### Run (b) — same workload in a dedicated Worker

| Field | Value |
| --- | --- |
| Date (UTC) | 2026-07-18T05:13:34Z (settled) |
| Operator | Artemis Peach (CDP-automated; remote display per disclosure above) |
| Browser + version | Google Chrome 150.0.7871.125 |
| OS + version | macOS 26.5.2 (arm64) |
| Bundle commit | `b68b596` |
| Requested deadlines (receive, native) | 30 000 ms receive-after; 45 000 ms native deliver |
| Armed unified deadline at background time | 30 000 ms, armed inside the dedicated Worker (`armedDelaysRaw: [30000]`); owning page backgrounded + minimized on arm. Note: the timer spy lives worker-side, so `sinceArmMs` is null in the fire records — elapsed-since-start is the timing carrier |
| Backgrounded interval | 390 s |
| Observed callback time(s) vs requested | receive-after fired at 30 005 ms elapsed (+5 ms); native-deliver at 45 003 ms (+3 ms) — dedicated-Worker timers essentially unthrottled by page backgrounding on this engine, replicating the live-desktop finding |
| Delivered identities + counts | both classes delivered exactly once (`bothDeliveredExactlyOnce: true`); 2 fires total; re-arm confirmed via `next_native_deadline_ms` 14 995.5 after the first fire |
| Next arm after settle | none; zero recurring callbacks post-settle |
| Late-but-complete? | YES (effectively on-time) — same completion identities as run (a) |

**Verdict:** identical shape to the live-desktop sitting — late-but-delivered
and complete in both contexts, exactly-once delivery, correct next-arm
selection, zero recurring callbacks — on a second physical machine, a
different display context (remote/unattended vs live/watched), and a
different Chrome patch level (150.0.7871.125 vs .128). The two-environment
close is real: both official sittings independently satisfy the probe's
stated expectation, and no observation in either contradicts the other.
