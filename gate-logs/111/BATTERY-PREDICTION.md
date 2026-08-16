# #111 AR-1 FIX LANE — battery prediction, PRE-REGISTERED

Written **before** the battery ran. Pin at time of writing: `ea7211a`
(15 commits ahead of `origin/main` `8d64fd3`). Tranches 1 and 2 complete;
phase 4 (the seal) NOT started and held on Cally's §8.2.

## The prior, and where it comes from

`gate-logs/110/battery2-run.txt`, the battery that landed the lane's base
`8d64fd3`:

| leg | result lines | passed |
|---|---|---|
| `wasm-tests` | 2 | 82 |
| `tests` | 76 | 2141 |
| `tests-all-features` | 76 | 2151 |

⭐ **AXES ARE NAMED, per the rider:** result-lines · passed · failed · ignored.
`failed` and `ignored` were 0 at the prior and are predicted 0 here.

## The delta, derived rather than guessed

Counted at the bytes with `git grep -c` against both refs, not from the diff —
this repo's diff renders side-by-side, so a `^+`-anchored grep silently returns
zero and would have made the delta look like nothing at all.

- `#[test]` in `crates/beamr/src`: **1813 → 1816**, and the whole `+3` is in
  `crates/beamr/src/native/context/accumulator.rs`, which did not exist at the
  base. Those are phase 1's own unit tests, landed in `1a70068`.
- `#[wasm_bindgen_test]` in `crates/beamr-wasm/src/convert.rs`: **5 → 6**, the
  one new nested-scope test from site 12.
- **Every other probe was INVERTED IN PLACE, not added.** Thirteen sites, zero
  new test functions between them — the arms changed, the count did not.

## PREDICTION

| leg | result lines | passed | failed | ignored |
|---|---|---|---|---|
| `wasm-tests` | 2 | **83** (+1) | 0 | 0 |
| `tests` | 76 | **2144** (+3) | 0 | 0 |
| `tests-all-features` | 76 | **2154** (+3) | 0 | 0 |

All 8 legs rc 0. Runner marker **COMPLETE**, `SCORED == DECLARED == 8`, pin
stable across the run.

## ⚠️ A DISCLOSED BLIND SPOT IN THE `tests` LEG, stated in advance

`tests` runs `--features beamr/encode`, which does **not** compile
`crates/beamr/src/term/json.rs` — that file is behind the non-default `json`
feature. **Sites 11 and 15 are therefore invisible to leg 5 and always were**;
only `tests-all-features` (leg 8) reaches them. This is the instrument gap I
found mid-lane by denominator, and it is repeated here so the leg-5 green is
read for what it covers and not for what it does not. The `+3` above is
unaffected: `accumulator.rs` is not feature-gated.

## What would make me stop rather than explain

Any leg rc != 0; a `passed` count that is not exactly the predicted number; any
`failed` or `ignored` above zero; `SCORED != DECLARED`; or a pin that moved.
A number that misses gets traced to the bytes before it gets a story, the way
the site-16 census miss did.
