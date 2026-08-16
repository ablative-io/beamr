# The self-test was feeding on the last unfixed row

⭐ **This is the finding of #115, and it was not on the plan.** The lane set out
to discharge AR-1 site 4. Discharging it broke the instrument that certifies the
ledger — not by rotting, and not by any edit to the checker, but by **running out
of defect to feed on**.

## What happened, in order

`ledger_check.py --sign-off` passed for the first time in the ledger's life.
`ledger_check.py --self-test` dropped from **10/10 to 7/10** in the same working
tree, and the three arms that died were:

```
  FAIL  structural/no-evidence           DID NOT REFUSE -- []
  FAIL  structural/true-must-PASS        ['site 4: STRUCTURALLY-ELIMINATED without ...']
  FAIL  sign-off/silence                 DID NOT REFUSE
```

All three had been written against **site 4 specifically**, because site 4 was
the last `PENDING` row in the ledger. The checker's own comment said so:

> The STRUCTURALLY-ELIMINATED arm has no live rows, so it is fired here in
> BOTH directions

`sign-off/silence` read its refusal off the *live* ledger and needed a real
`PENDING` row to exist. `structural/no-evidence` flipped site 4's disposition and
relied on it having no `replacement_construct`. `structural/true-must-PASS`
hard-coded `dictionary_bifs.rs:128` and derived its construct from
`line.strip()[:20]` — and when the file reflowed, line 128 became **blank**, so
the construct silently became the empty string and the arm failed for a reason
that had nothing to do with the checker.

## ⛔ The law

**A control that only works while a defect is outstanding is not a control.**

This is a new member of the asleep-instrument family and it arrives from a
direction the existing ones do not cover. The banked cases are about instruments
that *rot* — a renamed symbol, a moved file, a regex that stops matching. This
one never rotted. It was **consumed by its own lane succeeding**, and the failure
is worse-shaped than rot in two ways:

1. **It reads as strongest exactly when there is least to prove**, and falls
   silent at the moment its verdict starts to matter — the moment somebody signs
   off on a fully-dispositioned ledger.
2. **It fails on the same commit that earns the green**, so the natural reading
   of `7/10 self-test + passing sign-off` is *"the fix broke something"*, when in
   fact the checker was perfectly healthy the whole time. The temptation is to
   go looking for a defect in the work, or — much worse — to notice that
   `--sign-off` is green and ship on that alone.

⛔ **A `--sign-off` green with a dead self-test is ZERO evidence, not weak
evidence.** It has the same value whether the ledger is sound or the checker is
inert. That is why this was fixed inside this lane rather than filed as a
follow-up: without it, the headline of #115 would have rested on an unverified
instrument.

## The repair

The three arms now **mint their own guinea-pig row** instead of borrowing a live
one. `add_synthetic()` appends a crossing row and bumps the declared population
so the row is legal, and each arm overrides only the fields it is testing. The
arms are now independent of the ledger's contents — today, and after the next
lane empties it further.

Two further hardenings fell out of the repair:

- **The anchor line is FOUND, never counted to.** `ANCHOR_TOKEN` is searched for
  in `ANCHOR_FILE`; if it is absent the self-test **refuses to run** with a
  message that says *do not weaken the arms to make this pass*. A hard-coded line
  number is exactly what turned a passing arm into a failing one with nobody
  touching the checker.
- **`sign-off` is now fired in BOTH directions.** The borrowed-row version could
  only ever test the refusal. A new `sign-off/not-over-eager` arm asserts that a
  fully-dispositioned ledger is **accepted**, so a checker that refused
  everything can no longer satisfy the suite. The count is now **11/11**, and the
  eleventh is the one the old design was structurally incapable of having.

## Falsifiers on the repair

`gate-logs/115/selftest-falsifiers.log`. The repair is itself an instrument, so
it carries its own positive controls.

| arm | mutation | required | observed |
|---|---|---|---|
| baseline | none | 11/11, rc 0 | ✅ 11/11, rc 0 |
| **S1** | `ANCHOR_TOKEN` set to a string absent from the file | **refuse to run**, not degrade | ✅ `SELF-TEST CANNOT RUN`, rc 1 |
| **S2** | §5's `construct not in lines[...]` gutted to `if False` | `structural/refuted-at-bytes` **RED** | ✅ `FAIL ... DID NOT REFUSE`, rc 2 |

S1 is the important one: it proves the new anchor cannot repeat the old arm's
failure mode of quietly resolving to an empty string.
