# WPORT-3 PROBE-EARLY-FIRE — triage of the `requests == 1` deadline-wall flake

**Status:** `TORN PASS 2026-07-23 (Artemis Peach) — OVERCONSTRAINED WALL confirmed, service correct; hardened wall landed. Tear note: in early-fire-witness-output.json the worst_early_ms field holds the worst ELAPSED time (24.521), not the worst early delta — the early_deltas_ms array is the authoritative record; the emitter field name is corrected for future probe runs, committed evidence stands as-run.`
**Authored by:** the flake triage ruled first act of the port-arc allocation
(2026-07-23, Artemis Peach's rung confirmation; evidence bar per the same-day
addendum: an accounting explanation carries the same evidence weight as a
defect — demonstrated at the bytes, not adopted).
**Branch:** `triage/wport3-deadline-requests-flake` (from `main` `fb3efcf`).
**Evidence:** `evidence/2026-07-23-early-fire/`.

## The observed flake

`tests::await_exit_waits_for_armed_receive_timer` — a WPORT-3 wall — failed in
CI run 30004895967 (11:54Z, push of docs-only `fb3efcf`) at
`crates/beamr-wasm/src/lib.rs:2425`: `assert_eq!(counters.requests, 1)` got
**2**. The identical code was GREEN 55 minutes earlier (run 30001395255, push
of `5206e7a`). 64/65 tests passed in the red run. `requests` counts total host
`setTimeout` arms performed by the unified deadline service
(`crates/beamr-wasm/src/lib.rs:540`, `:1205` at `fb3efcf`).

Reproduced locally on stock `fb3efcf`: 2 failures in 101 filtered
single-test runs (~2%), identical assertion — see
`evidence/2026-07-23-early-fire/local-repro.txt`. Not CI-runner-specific.

## Verdict (proposed): OVERCONSTRAINED WALL — service correct, assertion claims platform promptness

The service's at-most-one-ACTIVE-callback contract **held**. The wall's final
`requests == 1` additionally asserts that the host never fires the one-shot
early relative to the wasm `Instant` clock — a timing-precision claim of
exactly the class the WPORT-3 platform bound (arc doc `:82`) forbids in the
late direction. The platform legally violates it, sub-millisecond, at a
per-arm rate that matches the observed once-per-battery flake.

## Mechanism at the bytes (all cites at `fb3efcf`)

1. **Stamp-at-sync:** receive deadlines are stamped
   `Instant::now() + delay` with sub-ms precision
   (`crates/beamr-wasm/src/lib.rs:1083-1095`). `Instant` under
   `wasm32-unknown-unknown` via `web_time` is `performance.now()`.
2. **The arm:** one host `setTimeout` at `millis_until_ceil(deadline)` —
   `div_ceil` of remaining micros, so the delay is rounded UP to whole ms
   (`:1182-1207`, `:1293-1299`). The design is LATE-biased; the core-fire
   comment says it "never rejects a timer because `now` is later than its
   requested instant" (`:1242-1244`). The late direction was designed for;
   the early direction was never contemplated.
3. **The platform's early fire:** Node schedules the timeout on libuv's
   cached ms-granularity loop clock — due = (loop timestamp at registration)
   + (integer ms delay). That cached timestamp lags `performance.now()` by
   however long the current loop iteration has already run, so the callback
   can be invoked while `performance.now()` still shows LESS than the
   requested delay elapsed. Witnessed directly on the exact clock pair:
   3/400 samples early, worst 0.479ms
   (`evidence/2026-07-23-early-fire/early-fire-witness-output.json`).
4. **Empty due set:** the admitted fire (token-matched, arm consumed,
   `executions += 1`, `:1221-1236`) collects due records by
   `record.deadline <= now` (`:1245-1251`). An early `now` leaves the one
   record NOT due: nothing is delivered, the record survives, and one
   arbiter turn is still requested (`:1264-1270`).
5. **The benign re-arm:** that turn's drain completion seam
   (`perform_drain`, `:758-774`) reconciles: `active_arm` is None (consumed),
   `unified_minimum()` is the SAME stamped deadline, so the `(None, Some)`
   leg arms exactly one new one-shot — `requests += 1`, `next_arms += 1`,
   `cancellations` untouched (`:1159-1163`, `:1204-1206`). The re-armed
   one-shot fires at/past the deadline and delivers, late-but-delivered.

**Why `requests == 2` cannot mean two concurrent callbacks:** every path to
`arm()` first consumes the previous arm — `reconcile` retires it with a host
`clearTimeout` (`:1144-1153`) and an admitted fire takes it under the token
check (`:1222-1228`); a stale callback performs no delivery and no counter
mutation (`:1229-1231`). `requests` counts arms over time, not concurrent
arms; the concurrent gauge is `queued_now`, which never exceeded 1.

**Alternative in-code path ruled out:** a retire-and-re-arm from a changed
minimum would increment `cancellations`; in this single-timer scenario the
minimum is stamped once and never changes (schedules are drained exactly once,
`:1079-1095`), so the early-fire re-arm is the only in-code route to
`requests == 2` consistent with the flaked run's green mid-test snapshot.

## Evidence pack (the five tear requirements)

### 1. Deterministic signature match

`tests::early_host_fire_re_arms_the_remainder_and_still_delivers`
(committed on this branch) drives the shared callback logic through the
deterministic fire seam (`fire_unified_deadline_at`, `:1792-1817`) at
`deadline − 1ms`: empty due delivery, single re-arm of the SAME stamped
instant, `requests == 2`, `next_arms == 2`, `cancellations == 0`,
`queued_now` never above 1, the process still waiting (0ms marker wins the
race), then a second fire at `deadline + 1ms` delivers exactly once with
`timed_out`. Final counter set: requests 2, executions 2, next_arms 2,
cancellations 0, queued_now 0, armed None — the flaked run's observed prefix
(`requests == 2` after a green mid-test armed snapshot) extended to the full
consistent set. GREEN on stock service code, deterministically, every run.

### 2. Platform witness

`early-fire-witness.mjs` (committed beside the outputs): 400 samples of
`setTimeout(25)` measured against `performance.now()` on Node v26.5.0/darwin —
3 early fires, deltas 0.479/0.262/0.267 ms. This is the same Node major CI
runs. Mechanism: libuv's cached loop clock (ms granularity, updated per loop
iteration) vs `performance.now()` (sub-ms), plus integer-ms due comparison.

### 3. Cascade bound

`cascade-witness.mjs` re-arms per `millis_until_ceil` semantics on every
early fire and measures the chain: across 400 samples, **max chain length 1**
(histogram 399×0, 1×1). Mechanism of the bound: any positive sub-ms remainder
re-arms a one-shot with a **1ms floor** (`div_ceil`), which overshoots the
remainder unless a fresh, independent staleness event at least as large as
the (now sub-half-ms) remainder recurs at that hop; a zero remainder arms 0ms
and delivers. Each hop's remainder is strictly smaller than the previous
skew, so P(chain ≥ k) decays geometrically. Stated exactly: **at most one
extra arm in practice (never exceeded in 400 samples); k extra arms require k
independent early-skew events against a shrinking sub-ms remainder — bounded
in expectation, no recurrence mechanism exists.** NO-POLLING compliance: each
re-arm is a one-shot delivering a KNOWN deadline — the binding law's
permitted form — not a check for change.

### 4. Hardened wall, fail-first proven

The amended `await_exit_waits_for_armed_receive_timer` (committed on this
branch, landing only on tear ratification) KEEPS every load-bearing claim:
mid-test active-arm cardinality (`queued_now == 1`, `armed_deadline` Some),
exactly-once delivery (`timed_out`, single settle), final quiescence
(`queued_now == 0`, `armed_deadline` None), `cancellations == 0`, and the
accounting identities `executions == requests` and `next_arms == requests`
(every arm was admitted and fired; no stale callback ever executes, no
cancels). It DROPS only the promptness claim (`requests == 1` as an absolute
count). Fail-first: with a seam-injected duplicate ACTIVE arm (a second
`arm()` without consuming the first — the real defect class), the hardened
wall goes RED — injection diff and red run output at
`evidence/2026-07-23-early-fire/hardened-wall-red-injection.diff` and
`hardened-wall-red-run.txt`. A duplicate active arm is also caught at the
final identities: the stale token's callback is refused admission, so
`executions < requests`.

### 5. Platform bound (T2's sibling), recorded

**EARLY-UNDER-CACHED-CLOCK:** browsers and Node may fire a one-shot timeout
EARLY relative to `performance.now()` by sub-millisecond amounts (cached
event-loop clocks, integer-ms due comparison). The unified deadline service
tolerates this by construction: an early fire delivers nothing, keeps the
records, and the completion seam re-arms the remainder as one one-shot
(self-quenching, see §3). Consequences for the arc: **no acceptance test may
assert absolute host arm counts (`requests == N`) across a real timer fire**
— assert the cardinality gauge, the accounting identities, and delivery
instead. This bound travels with WPORT-3's T2/PROBE-THROTTLE law (late under
throttling) as its early-direction sibling; a cross-reference is appended to
`WPORT-3-PROBE-THROTTLE.md`. WPORT-9's conformance walls inherit this review.
