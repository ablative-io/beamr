# CRASH-2 BATTERY PREDICTION — committed BEFORE the run

Artemis Peach. Pin: `1965650` + the working-tree crash-2 fix. Runner:
`gate-logs/103/battery-RUNNER.sh` (legs read from `gates.json` at run time, so
the legs that run are the legs that ship). Written and saved before the runner
is launched.

## Prior, with NAMED AXES

From the #103 battery at `07e8a60` (`gate-logs/103/battery-07e8a60-*`),
re-extracted at my hands from those logs rather than quoted from memory:

| leg | result-lines | passed | failed | ignored |
|---|---:|---:|---:|---:|
| 5 `tests` (`--features beamr/encode`) | 73 | 2112 | 0 | 0 |
| 8 `tests-all-features` | 73 | 2122 | 0 | 0 |

`1965650` is three docs-only commits ahead of `07e8a60` (CHANGELOG, #103 gate
evidence, #13 census docs), so the prior carries forward unchanged.

## Delta this lane introduces

Production: `Process::nested_handler_floor` + its two accessors and the
nesting-aware pop (process/mod.rs); the floor install/restore around the nested
run (jit/runtime.rs); `dispatch_captured_exception` factored out of
`raise_exception` (interpreter/opcodes/exceptions.rs); the
`JIT_STATUS_EXCEPTION` re-offer (interpreter/opcodes/core.rs). **No test was
deleted, renamed, or moved.**

Tests: ONE new file, `crates/beamr/tests/jit_nested_run_handler_leak.rs`, with
FIVE `#[test]` functions. It is not cfg-gated (matching its siblings
`jit_wireup.rs` / `jit_send_delivery_gate.rs`), and `jit` is a default feature,
so it builds and runs in BOTH test legs.

## Prediction — exact, per named axis

| leg | result-lines | passed | failed | ignored |
|---|---:|---:|---:|---:|
| 5 `tests` | 73 -> **74** (+1, one new test binary) | 2112 -> **2117** (+5) | **0** | **0** |
| 8 `tests-all-features` | 73 -> **74** (+1) | 2122 -> **2127** (+5) | **0** | **0** |

All eight legs rc 0; runner prints COMPLETE 8/8 with the pin stable. Both clippy
FULL legs (2 and 7) run AFTER the last edit of any kind — every source and
document change for this lane, including this file, is on disk before the runner
starts.

A miss on any axis is reported as a miss, loudly, and reconciled before anything
is claimed green. `+5 exact` is the falsifiable part: any other number means the
new file's arms did not all run, or something else moved.

## Already measured, before the battery (both directions)

- Fixture RED at the unfixed bytes: 7 vs 1 at K=6; 3 at K=2 vs 7 at K=6; 3 per
  trampoline at nesting depth two. Control green throughout.
- Fixture with **half 1 only** (floor installed, `JIT_STATUS_EXCEPTION` left as
  an unconditional exit): all four frame-count arms PASS and
  `caught_exception_value_reaches_the_handler_across_a_compiled_trampoline`
  goes RED with the process killed (`got Error`). The liveness assertion is the
  only arm that catches a half-fix; this is why part 2 is not optional.
- Fixture GREEN 5/5 with the whole fix.

---

# ERRATUM (written after battery 1, before battery 2) — MY BASELINE WAS WRONG

Battery 1 measured **76 / 2128 / 0 / 0** (leg 5) and **76 / 2138 / 0 / 0**
(leg 8). I predicted 74/2117 and 74/2127. **The prediction MISSED on the
absolute axes.** Reported here rather than restated silently.

**What was wrong:** the prior. I took it from `gate-logs/103/` because 103 is
the highest-numbered gate-log directory, and asserted `1965650` was "three
docs-only commits ahead of `07e8a60`". Measured: `git rev-list --count
07e8a60..HEAD` = **35**, and that span includes six test-adding lanes (#104,
#106, #87, #91, AR-1 rows 1 and 3, #95). **Directory number is not recency** —
#95's battery (`c9a63c6`) landed AFTER #103's (`07e8a60`), and
`git merge-base --is-ancestor 07e8a60 c9a63c6` confirms the ordering.

**The correct prior**, re-extracted at my hands from `gate-logs/95/leg5.log` and
`leg8.log` (not from the commit message that quotes them):

| leg | result-lines | passed | failed | ignored |
|---|---:|---:|---:|---:|
| 5 `tests` | 75 | 2123 | 0 | 0 |
| 8 `tests-all-features` | 75 | 2133 | 0 | 0 |

`c9a63c6..HEAD` = 1 commit, docs-only, so the prior carries forward unchanged.

**Against the correct prior the delta prediction held EXACTLY:**
75 -> 76 result-lines (+1, the one new test binary), 2123 -> 2128 and
2133 -> 2138 passed (**+5, the five new arms**), failed 0, ignored 0. The
falsifiable content of the prediction — +1 binary, +5 tests, nothing else moves
— was right; the absolute numbers I hung it on were not.

**Banked instrument rule:** pick the prior battery by COMMIT ANCESTRY, not by
the largest gate-log directory number. Lane numbers are assigned by topic, not
by time, and a lane that starts later can land earlier.

## Battery 1 verdict and what forced battery 2

Battery 1: **7/8 rc 0, leg 1 `fmt` rc 1** — the tsv is the verdict, so battery 1
is RED and is kept as evidence, not discarded. The failure was formatting-only,
entirely inside the new test file (rustfmt collapsing five call sites), no
production byte involved. `cargo fmt --all` applied; `cargo fmt --all --check`
now rc 0.

That formatting change is an edit, so the whole battery re-runs — both clippy
FULL legs must run after the LAST edit, and the bytes that ran must be the bytes
that ship.

## Battery 2 prediction — committed before that run

Identical to battery 1's measurement, because the only delta is whitespace in a
test file: leg 5 **76 / 2128 / 0 / 0**, leg 8 **76 / 2138 / 0 / 0**, all eight
legs rc 0, runner COMPLETE 8/8, pin stable at `1965650`.
