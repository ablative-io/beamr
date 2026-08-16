# #115 — RESULTS

Site 4 discharged by the non-breaking interim. **The last `PENDING` row in the
AR-1 disposition ledger is gone, and `--sign-off` passes for the first time in
the ledger's life.**

Ground: `gate-logs/114/SITE4-PARK.md`. Red-first: `gate-logs/115/FALSIFIERS.md`.
Prediction: `gate-logs/115/BATTERY-PREDICTION.md`, committed before the runner.

## ⭐ THE HEADLINE IS NOT THE FIX — it is what the fix did to the instrument

Full record: `gate-logs/115/SELF-TEST-PARASITISM.md`.

`--sign-off` passed. `--self-test` dropped **10/10 → 7/10 in the same working
tree.** Three of its arms had been written against site 4 *because* site 4 was
the last `PENDING` row. They did not rot, and nobody touched the checker — they
**ran out of defect to feed on.**

⛔ **A control that only works while a defect is outstanding is not a control.**
It reads as strongest exactly when there is least to prove, and it fails on the
same commit that earns the green — so the tempting reading is *"notice
`--sign-off` is green and ship on that."* **A `--sign-off` green beside a dead
self-test is ZERO evidence, not weak evidence**, so this was repaired in-lane
rather than filed. The arms now mint their own guinea-pig row, the anchor line is
**found** rather than counted to, and sign-off is fired in **both** directions.
**11/11**, where the eleventh is an arm the borrowed-row design was structurally
incapable of having.

## The fix

Reserve → copy/drain → build is **fused** into `dict_entries_to_list` /
`dict_erase_all_to_list` / `dict_keys_for_value_to_list`. The raw copy and drain
bodies are private; the three public raw methods are thin `#[deprecated]`
wrappers pointing at their replacements. All three call sites migrated; the
file-local `entries_to_list`, `entries_heap_words` and `list_heap_words` deleted.

⚠️ **The reserve carries the safety, not a rooting construct**, and the
accumulator was deliberately *not* also wrapped around the build — doing so would
tell a false story about what protects the site. Row 4 is therefore the one
`STRUCTURALLY-ELIMINATED` crossing whose replacement is not `with_accumulator`.

⛔ **The shape moved; it did not vanish.** Row 4 carries a
`relocation_disclosure` naming where the `tuples` loop went and the three
measured grounds on which it is not re-registered as a new crossing.

## Red first, at the moved bytes

`gate-logs/115/falsifiers.log`.

| arm | required | observed |
|---|---|---|
| A — production bytes | GREEN | ✅ probe 1/1, module 8/8 |
| B — reserve deleted from `dict_entries_to_list` | RED | ✅ rc 101 |
| D — `entries_heap_words` `* 5` → `* 2` | RED | ✅ rc 101 |

B and D die at the **identical assertion** as at `f993280`, and arm D
additionally kills `erase_0_reports_allocation_failure_without_clearing_dictionary`
— a free, independent witness on the **erase** path that #110 never probed.

## The battery

Pin `d6c82ae`. Prediction was **total stasis**, declared up front as
corroboration rather than evidence.

| leg | predicted | measured | |
|---|---|---|---|
| 4 `wasm-tests` | 2 / 86 / 0 / 0 | 2 / 86 / 0 / 0 | ✅ unchanged |
| 5 `tests` | 76 / 2150 / 0 / 0 | 76 / 2150 / 0 / 0 | ✅ unchanged |
| 8 `tests-all-features` | 76 / 2160 / 0 / 0 | 76 / 2160 / 0 / 0 | ✅ unchanged |

8/8 rc 0 · `SCORED == DECLARED == 8` · marker **COMPLETE** · pin identical at open
and close. The `+0` was derived at the bytes — zero `#[test]` /
`#[wasm_bindgen_test]` attribute lines added or removed anywhere in the diff.

Prediction pin `abe8d2d` vs battery pin `d6c82ae`: one commit, `--numstat`
**74/0, zero Rust** — the wrinkle declared up front, confirmed rather than
asserted.

### ⚠️ Tree-count disclosure — the raw count is NOT flat lane-to-lane, and that is legitimate

`tree pre: 26` against #114's `tree pre: 21`. **This is not drift in this lane.**
Established form commits only `leg4/5/8`, the `.tsv` and `BATTERY.log`; the other
five per-leg logs carry no axes and are left untracked. So the residue grows by
five every lane and `tree pre` is **monotonic across lanes by construction** —
the +5 here is #114's own leftovers, sampled after #114's `tree pre` was taken.

What the instrument actually asserts is **`pre == post` within a run**, and that
holds exactly (26 → 26), with **zero tracked modifications** throughout. But the
absolute number is not comparable between lanes and must not be read as if it
were. Flagged as standing hygiene, **not fixed mid-battery** — binning four
lanes' logs is not a call to make quietly inside an unrelated lane.

✅ Working notes went to scratchpad again rather than into the repo, holding the
#113 operator correction for a second consecutive lane.

## The ledger

- **`--self-test`: 11/11**, `✅ every check fired`.
- **`--sign-off`: PASSES.** 22 rows, 17 crossings + 5 control fixtures.
- Row 4 → `STRUCTURALLY-ELIMINATED`, `replacement_construct` **`dict_entries_to_list`**
  machine-verified present at `dictionary_bifs.rs:72`.
- The `PENDING` set is **empty**.

The raw trio is **not deleted** — that is semver-breaking and rides the 0.19.0
cut beside the #104 `spawn_process` scaffold deletion. The park's re-check
trigger is now **act-time**: any new internal caller goes red at compile under
`-D warnings`.
