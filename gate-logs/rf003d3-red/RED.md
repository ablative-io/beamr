# RF-003-D3 — RED FIRST (R1 Leg A), committed BEFORE the cap exists

Red-first is a commit-ordering claim, not a narrative one: this commit contains
the failing test and NOT the fix. The next commit converts it to green under the
SAME test name.

## The leg

| item | value |
| --- | --- |
| test name | `distribution::etf::tests::deframe_refuses_length_above_max_dist_frame_bytes` |
| file | `crates/beamr/src/distribution/etf.rs` |
| command | `cargo test -p beamr --lib distribution::etf::tests` |
| start (UTC) | `2026-08-03T01:21:19Z` (`red-start.utc`) |
| end (UTC) | `2026-08-03T01:22:07Z` (`red-end.utc`) |
| rc | `101` (`red-leg-a.rc`) |
| log | `red-leg-a.log` (REDIRECT, no tee, no pipeline) |

Harness line, quoted from `red-leg-a.log`:

```
test result: FAILED. 9 passed; 1 failed; 0 ignored; 0 measured; 1771 filtered out; finished in 0.22s
```

## Exact assertion text

```
thread 'distribution::etf::tests::deframe_refuses_length_above_max_dist_frame_bytes' (62791319) panicked at crates/beamr/src/distribution/etf.rs:1325:9:
assertion `left == right` failed
  left: Err(TruncatedBody { expected: 67108865, actual: 0 })
 right: Err(LengthTooLarge)
```

This is the defect stated in its own words. `try_reserve_exact` is a
fallible-ALLOC rail, not a ceiling: the helper accepted the peer's declared
length, reserved it successfully, and only then failed on the body that never
arrived. `TruncatedBody` is a *framing* complaint about a size the helper had
already agreed to.

## BOUNDED-RED, proven from the artifact rather than asserted

The header claims `MAX_DIST_FRAME_BYTES + 1` = **67108865 bytes (64 MiB + 1)**,
never `0xFFFFFFFF`. The reserved size is not an estimate here — it is printed by
the failure itself as `expected: 67108865`, because `TruncatedBody.expected`
carries the very length the reservation used. So the red run's peak frame
allocation is ~64 MiB, held briefly and dropped. No multi-gigabyte commit, no
`handle_alloc_error`, no runner abort.

**The `0xFFFFFFFF` extreme leg is GREEN-ONLY and is deliberately ABSENT from
this commit.** Running it against unfixed bytes is a STOP condition; it is added
in the implementation commit, once the cap can refuse it before any reservation.

## Import note (converts, does not persist)

At this commit the test imports the constant from its current home:
`use crate::distribution::connection::MAX_DIST_FRAME_BYTES;`. The implementation
commit MOVES the constant to `etf.rs` (D-c), after which `use super::*` supplies
it and that import line is removed. The test NAME and its assertion are
unchanged across the conversion — the green is the same leg, not a replacement.

## Boundary and disk

Threshold `47710208` KiB per `../rf003d3-battery/THRESHOLD.txt`.

| reading | KiB available | artifact |
| --- | --- | --- |
| before this phase (baseline commit) | 106343652 | `../rf003d3-baseline/boundary-baseline.df` |
| immediately after the red run | 100876964 | `boundary-red.df` |

Both clear the threshold by more than 53 GiB. `du -sk` of the worktree after the
red run: **5406920 KiB** (`du-red.txt`), up from 4161096 at baseline end — the
red run added the dev-dependency closure (criterion, proptest) that
`cargo test -p beamr --lib` pulls in.
