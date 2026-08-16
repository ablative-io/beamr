# BATTERY PREDICTION — #111 tranche 3, pin `2f0f3d9`

Written and committed **before the runner was launched.** If a number below is
wrong, the reconciliation says so and the miss is the finding — a prediction that
is quietly adjusted afterwards is not a prediction.

⭐ **AXES, per the rider — expected priors travel with NAMED AXES:**
`result-lines / passed / failed / ignored`.

## Prior, at `ae29d2c` (the pin of the previous battery's parent)

| leg | prior |
|---|---|
| 4 `wasm-tests` | 2 / 83 / 0 / 0 |
| 5 `tests` | 76 / 2144 / 0 / 0 |
| 8 `tests-all-features` | 76 / 2154 / 0 / 0 |

## Predicted, at `2f0f3d9`

| leg | predicted | basis |
|---|---|---|
| 4 `wasm-tests` | 2 / **86** / 0 / 0 | **+3**, derived at the bytes: `#[wasm_bindgen_test]` in `convert.rs` goes 6 → 9 (`git grep -c` against both refs). Named: `ar1_site13_array_to_term_two_armed`, `ar1_site17_object_to_term_two_armed`, `ar1_sites_13_17_nested_scopes_open_inside_one_another`. |
| 5 `tests` | 76 / 2144 / 0 / 0 | **UNCHANGED.** The only Rust file touched is `crates/beamr-wasm/src/convert.rs`, whose tests are behind `#[cfg(all(test, target_arch = "wasm32"))]` and are not compiled by a native `--workspace` run. |
| 8 `tests-all-features` | 76 / 2154 / 0 / 0 | **UNCHANGED**, same reason. `--all-features` does not turn on a different target architecture. |

All other legs: **rc 0**, no count axis.
`SCORED == DECLARED == 8`. Marker **COMPLETE**. Pin identical at open and close.

⚠️ The `+3` is derived from a **grep against both refs**, never from reading a
diff: this repo has a side-by-side external diff pager, so a `^+`-anchored grep
over `git diff` silently returns zero and would make the whole delta look like
nothing at all. (Same trap cost me a hunk-header read earlier this tranche;
`--no-ext-diff` is the fix when unified output is actually wanted.)

## Stop conditions, stated up front

1. `SCORED != DECLARED` — **but check `ps` first.** A partial artefact and an
   aborted one are identical on disk; the process table is what separates "in
   flight" from "aborted". I misread a 7-row TSV this way once already this lane.
2. Any leg rc != 0.
3. Any predicted axis missing its number — including a leg that comes in
   **higher** than predicted. An unexplained extra passing test is a finding, not
   a bonus.
4. `git status --untracked-files=no` non-empty at close: a tracked file moved
   during the run, so the bytes that ran are not the bytes that ship.
5. Pin differing between open and close.

## Declared in advance, so it is not read as a surprise

- The tree census will show a **non-zero untracked count** at both open and
  close: five leg logs from the previous battery (`battery-ea7211a.leg{1,2,3,6,7}.log`)
  are deliberately untracked, and this battery adds its own. The census that
  matters is condition 4 above, which counts **tracked** movement only.
- **Which leg logs get committed is a SELECTION, and it is declared rather than
  silent:** the `.tsv` (every leg's rc), the runner's own `BATTERY.log`, and the
  three test legs' logs. Legs 2 and 7 are ~230 KB clippy JSON dumps each and
  legs 1/3/6 are near-empty; every one of their verdicts is in the `.tsv`, which
  is the artefact the reconciliation reads.
- The marker is authoritative, **the exit code is not.**
