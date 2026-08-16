# #115 battery — committed BEFORE the runner

Pin at time of writing: `abe8d2d`. Legs read from `gates.json` at run time; 8
declared.

## ⚠️ DECLARED UP FRONT: THIS IS A WEAK BATTERY, AND IT IS NOT WHAT CARRIES THE LANE

The prediction below is **"nothing moves on any axis."** A battery whose
prediction is total stasis is **corroboration, not evidence** — it can only
falsify by catching collateral damage, and it has no power to confirm the fix.
Stated here rather than discovered in the reading.

What actually carries #115:

1. **The three probe arms** — A GREEN, B RED, D RED, both reds at the identical
   assertion as at `f993280`, plus arm D's free erase-path kill.
   `gate-logs/115/falsifiers.log`.
2. **The two self-test falsifiers** — S1 refuses to run, S2 turns an arm red.
   `gate-logs/115/selftest-falsifiers.log`.
3. **`ledger_check.py --self-test` 11/11**, which is what makes `--sign-off`'s
   green mean anything at all.
4. **The two-arm census**, whose zero delta is predicted silence and is offered
   as such.

## The axes

Priors are #114's measured close at `cf00e03`.

| leg | axis (result-lines / passed / failed / ignored) | prior | predicted |
|---|---|---|---|
| 4 `wasm-tests` | 2 / 86 / 0 / 0 | 2 / 86 / 0 / 0 | **unchanged** |
| 5 `tests` | 76 / 2150 / 0 / 0 | 76 / 2150 / 0 / 0 | **unchanged** |
| 8 `tests-all-features` | 76 / 2160 / 0 / 0 | 76 / 2160 / 0 / 0 | **unchanged** |

**Derived at the bytes, not off a diff.** `git diff -U0 -- crates/` contains
**zero** added or removed `#[test]` / `#[wasm_bindgen_test]` attribute lines.
Whole-tree counts at the committed bytes: 2097 `#[test]`, 86
`#[wasm_bindgen_test]`.

## ⭐ THE PREDICTED FLIP, named in advance so it is an observation and not a surprise

`ledger_check.py` is **not a leg in this battery** and is run separately. Both
of its movements are predicted here:

- **`--sign-off` PASSES.** This is the first pass in the ledger's life. Every
  prior run refused, most recently on `[4]` alone and before that on `[4, 6]`.
  Row 4 moving to `STRUCTURALLY-ELIMINATED` with a machine-verified
  `replacement_construct` at `dictionary_bifs.rs:72` empties the `PENDING` set,
  and `PENDING` is the only sign-blocking disposition still populated.
- **`--self-test` 11/11**, up from 10/10 — the count goes UP because the repair
  adds `sign-off/not-over-eager`, the negative half the borrowed-row design
  could not have had. It read **7/10 before the repair**, and that reading is
  recorded in `SELF-TEST-PARASITISM.md` rather than smoothed over.

## Pin wrinkle, declared up front

The prediction is committed on top of `abe8d2d`, so the battery pin will be the
commit that carries this file. That commit is **docs-only** — one new file under
`gate-logs/115/`, zero Rust — so the tree the battery measures is Rust-identical
to `abe8d2d`. To be confirmed by `--numstat` in `RESULTS.md`, not asserted.

## Contingency, pre-registered

If any axis moves, the movement is **recorded and re-aimed, never scored as a
pass**. A moved axis on a zero-test-delta commit means something was collaterally
disturbed, and finding out what would become the lane — the battery would have
done the one job a stasis prediction is good for.

## Operator note

Working notes go to **scratchpad**, not into the repo, for the duration of the
run. The `tree pre` / `tree post` raw counts must move only for reasons the
runner can see. This is the #113 disclosure corrected at the operator end, held
for the second lane running.
