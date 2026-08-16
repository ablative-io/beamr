# #115 — red-first, before the fix was allowed to count

Raw output: `gate-logs/115/falsifiers.log`.

## The probe was RE-POINTED, not retired — and not just left alone either

`ar1_site4_defended_by_the_callers_prereserve_not_by_the_site` is unchanged
Rust. What moved is **what its arms mean**.

When #110 wrote it, the two halves it separated lived apart: the accumulation in
a file-local `entries_to_list` in `dictionary_bifs.rs`, the reserve up in
`bif_get_0`. **Arm B deleted the caller's half.** The interim fuses both halves
into `ProcessContext::dict_entries_to_list`, so *the caller no longer has a half
to delete* — **arm B is now the `ensure_heap_space` INSIDE that method.** Same
falsifier, one level down.

⚠️ The probe's doc comment carried the old geography and would have been quietly
false after the move. It is re-pointed in this lane, including the closing
caveat, which is **narrowed rather than lifted**: see below.

## The arms

| arm | mutation | required | observed |
|---|---|---|---|
| **A** | none — production bytes | GREEN | ✅ probe 1/1, module 8/8 |
| **B** | `ensure_heap_space` deleted from `dict_entries_to_list` | RED | ✅ rc 101 |
| **D** | `entries_heap_words`: `* 5` → `* 2` | RED | ✅ rc 101 |

⭐ **B and D die at the IDENTICAL assertion they died at in #110** —
`dictionary entry tuple`, `Tuple::new` returning `None` on a stale term, at
`dictionary_bifs.rs:246`. So the *failure signature* is unchanged by the move,
not merely the verdict. A red obtained by a different mechanism would not have
shown that the same defect is still the thing being prevented.

⭐ **ARM D KILLS A SECOND TEST FOR FREE, and it is a path #110 never probed.**
Under the halved reserve, `erase_0_reports_allocation_failure_without_clearing_dictionary`
also goes RED. That is an **independent witness that the reserve constant is
load-bearing on the ERASE path**, which matters because erase is the sharper
half: after a drain the entries are rooted by *nothing*, whereas on the get path
the dictionary still holds them. #110 had no erase-side control. This lane has
one, and it was not designed — it fell out of the arm-D sweep and is recorded
because it is real, not because it was wanted.

## What the arms do NOT show

They do not show that the accumulator shape is gone. **It is not gone — it
moved**, into a private `ProcessContext::entries_to_list`. See
`relocation_disclosure` on ledger row 4. What the arms show is that the reserve
still defends it and that no in-crate caller can now omit the reserve.

## The census is a WEAK instrument here and is not offered as if it were

`shape_hunt.py` at both arms:

```
BEFORE (origin/main e74d9f2 bytes) : 44 raw · 22 production · 22 cfg(test)
AFTER  (interim bytes)             : 44 raw · 22 production · 22 cfg(test)
```

Class-by-class identical (S3a 11 · S3b 16 · S3c 1 · S3d 6 · S3e 10), and **all 5
class controls PASS on both runs** — so the zeroes are interpretable rather than
uninterpretable. But the delta is zero because **the hunt is blind to a plain
push loop in both arms**, exactly as recorded for row 2. The census corroborates
nothing about this fix and is reported only so that its silence is on the record
as *predicted silence* rather than as evidence.

⚠️ The first run of `shape_hunt.py` this lane returned `0 raw · 0 production ·
0 cfg(test)` with `population walked: 0 files`, because it was invoked from the
evidence directory rather than the repo root. **It refused correctly** — all five
class controls reported `FAIL` and the run printed
`⛔ CONTROL FAILURE -- this run's zeroes are uninterpretable`. Recorded because a
clean-looking zero that the instrument itself disowned is the exact shape that
gets mistaken for a result.
