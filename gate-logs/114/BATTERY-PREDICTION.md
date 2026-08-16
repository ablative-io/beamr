# BATTERY PREDICTION — AR-1 site 6

**Code pin `20b9245`. Written and committed BEFORE the runner was launched.**

⚠️ **Pin wrinkle declared up front**, per the ratified handling: committing a
prediction before the run moves HEAD, so the battery runs at the sha of *this
file's own commit*, one docs-only commit above the code. Proved with
`git diff --numstat`, not asserted.

⭐ **AXES, per the rider:** `result-lines / passed / failed / ignored`.

## Prior, at `049ffbf`

| leg | prior |
|---|---|
| 4 `wasm-tests` | 2 / 86 / 0 / 0 |
| 5 `tests` | 76 / 2148 / 0 / 0 |
| 8 `tests-all-features` | 76 / 2158 / 0 / 0 |

## Predicted

| leg | predicted | basis |
|---|---|---|
| 4 `wasm-tests` | 2 / 86 / 0 / 0 | **UNCHANGED.** `beamr-wasm` is untouched; this lane is entirely in `crates/beamr/src/native/otp_stubs/`. |
| 5 `tests` | 76 / **2150** / 0 / 0 | **+2**, derived at the bytes: `^#[test]` in `otp_stubs/tests.rs` goes **15 → 17** (`git grep -c` against both refs). Named: `ar1_site6_env_pairs_survive_a_collection_during_accumulation`, `ar1_site6_probe_population_really_collects`. |
| 8 `tests-all-features` | 76 / **2160** / 0 / 0 | **+2**, the same two. `otp_stubs/tests.rs` is `#[cfg(test)]` and not feature-gated, so both test legs see them. |

Result-lines stay **76** on both: two new `#[test]` functions land in an existing
test binary and add no new binary.

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

- ⛔ **The `--sign-off` leg is NOT part of this battery and is EXPECTED TO REFUSE
  (rc 2) with `1 crossing(s) still PENDING: [4]`.** That refusal is correct and is
  the ledger doing its job — site 4 is deliberately not dispositioned by this
  lane. Recorded here so a reader does not mistake a working refusal for a
  regression.
- The untracked census is **non-zero at open** and only the **tracked** census
  (condition 4) is evidence. Per #112/#113: a flat untracked count does not mean
  nothing was written, and a moved one does not mean anything did.
- **Committed leg logs are a SELECTION:** the `.tsv`, the runner's `BATTERY.log`,
  and the three test legs. Legs 2 and 7 are large clippy JSON dumps; their
  verdicts live in the `.tsv`.
- The runner writes its marker to **stdout only**; the redirect is what makes it
  survive.
- **The red-first evidence already ran and is NOT part of this battery** —
  recorded at `gate-logs/114/site6-red-at-parent.log` and
  `gate-logs/114/site6-cell-sweep.log`, at the pre-fix bytes.
