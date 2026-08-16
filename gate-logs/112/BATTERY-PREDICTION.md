# BATTERY PREDICTION — #112 collection observable

**Code pin `f04773b`. Written and committed BEFORE the runner was launched.**

⚠️ **THE PIN WRINKLE, STATED UP FRONT THIS TIME** rather than reconciled
afterwards. Committing a prediction before the run necessarily moves HEAD, so
the battery runs one commit above the code — at the sha of *this file's own
commit*. The delta is docs-only and is proved with `git diff --numstat`, not
asserted. The lead ratified this handling on the previous lane: **a
pre-registration that can be edited to fit is not one, and recording beats
re-pointing.** So the discrepancy is declared in advance here, and the
reconciliation only has to confirm it.

⭐ **AXES, per the rider:** `result-lines / passed / failed / ignored`.

## Prior, at `42ea92d`

| leg | prior |
|---|---|
| 4 `wasm-tests` | 2 / 86 / 0 / 0 |
| 5 `tests` | 76 / 2144 / 0 / 0 |
| 8 `tests-all-features` | 76 / 2154 / 0 / 0 |

## Predicted

| leg | predicted | basis |
|---|---|---|
| 4 `wasm-tests` | 2 / 86 / 0 / 0 | **UNCHANGED.** `beamr-wasm` is untouched this lane. |
| 5 `tests` | 76 / **2148** / 0 / 0 | **+4**, derived at the bytes: `#[test]` in `crates/beamr/src/gc/tests.rs` goes **26 → 30** (`git grep -c` against both refs). Named: `a_fresh_process_has_collected_nothing`, `arm_a_collection_counter_moves_on_a_collection`, `arm_b_collection_counter_does_not_move_on_a_pure_resize`, `a_collection_can_happen_with_capacity_unchanged`. |
| 8 `tests-all-features` | 76 / **2158** / 0 / 0 | **+4**, the same four. `gc/tests.rs` is `#[cfg(test)]` and **not** feature-gated, so both test legs see them. |

Result-lines stay **76** on both: four new `#[test]` functions land in existing
test binaries and add no new binary.

All other legs **rc 0**, no count axis. `SCORED == DECLARED == 8`. Marker
**COMPLETE**. Pin identical at open and close.

## Stop conditions

1. `SCORED != DECLARED` — **check `ps` first**; a partial artefact and an aborted
   one are identical on disk.
2. Any leg rc != 0.
3. Any predicted axis off, **including higher than predicted**. An unexplained
   extra passing test is a finding, not a bonus.
4. `git status --untracked-files=no` non-empty at close.
5. Pin differing between open and close.

## Declared in advance

- The untracked census is **non-zero at open**, carrying leftover leg logs from
  the two previous batteries plus this run's `BATTERY.log`, created by the output
  redirect at launch. Only the **tracked** census (condition 4) is evidence.
- **Committed leg logs are a SELECTION:** the `.tsv` (every leg's rc), the
  runner's `BATTERY.log`, and the three test legs. Legs 2 and 7 are ~230 KB
  clippy JSON dumps; their verdicts live in the `.tsv`.
- The runner writes its marker to **stdout only**; the redirect is what makes it
  survive.
- **The falsifiers already ran and are NOT part of this battery** — they are
  recorded at `gate-logs/112/falsifiers.log` with both mutations reverted and the
  tree re-verified green. This battery grades the shipped bytes, not the
  mutations.
