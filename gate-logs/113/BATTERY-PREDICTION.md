# BATTERY PREDICTION — #113 AR-1 control retrofit

**Code pin `f47c514`. Written and committed BEFORE the runner was launched.**

⚠️ **THE PIN WRINKLE, declared up front** per the lead's ratification.
Committing a prediction before the run moves HEAD, so the battery runs one
commit above the code — at the sha of *this file's own commit*. The delta is
docs-only and is proved with `git diff --numstat`, not asserted.

⭐ **AXES, per the rider:** `result-lines / passed / failed / ignored`.

## Prior, at `96cc54a`

| leg | prior |
|---|---|
| 4 `wasm-tests` | 2 / 86 / 0 / 0 |
| 5 `tests` | 76 / 2148 / 0 / 0 |
| 8 `tests-all-features` | 76 / 2158 / 0 / 0 |

## Predicted — **all three UNCHANGED**

| leg | predicted | basis |
|---|---|---|
| 4 `wasm-tests` | 2 / 86 / 0 / 0 | `#[wasm_bindgen_test]` in `convert.rs` is **9 → 9** (`git grep -c` against both refs). No test added or removed; three existing tests were re-pointed at a different observable. |
| 5 `tests` | 76 / 2148 / 0 / 0 | **`crates/beamr` is untouched.** The only tracked file in the commit's code delta is `crates/beamr-wasm/src/convert.rs`. |
| 8 `tests-all-features` | 76 / 2158 / 0 / 0 | same. |

⭐ **An all-unchanged prediction is the weakest kind of prediction and I am
stating that plainly.** It cannot be confirmed by movement — only by absence of
movement, which a broken runner also produces. The load-bearing evidence for this
lane is therefore **not** the battery: it is the falsifier set in
`gate-logs/113/falsifiers.log`, where all three controls were shown RED under
M1b and site 13 RED under M2. The battery's job here is only to show that
re-pointing the controls broke nothing else.

That is also why the stop conditions below keep **rc per leg** and
`SCORED == DECLARED`: on an all-unchanged prediction those are the axes that can
actually fail.

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
  earlier batteries plus this run's `BATTERY.log`. Only the **tracked** census
  (condition 4) is evidence.
- ⚠️ And per the #112 finding: a flat untracked count does **not** mean nothing
  was written — git collapses an untracked DIRECTORY to one line.
- **Committed leg logs are a SELECTION:** the `.tsv` (every leg's rc), the
  runner's `BATTERY.log`, and the three test legs. Legs 2 and 7 are large clippy
  JSON dumps; their verdicts live in the `.tsv`.
- The runner writes its marker to **stdout only**; the redirect is what makes it
  survive.
- **The falsifiers already ran and are NOT part of this battery** — recorded at
  `gate-logs/113/falsifiers.log`, all mutations reverted, tree re-verified green
  at the bytes before the code commit.
