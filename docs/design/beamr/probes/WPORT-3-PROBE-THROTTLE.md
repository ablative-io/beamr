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
