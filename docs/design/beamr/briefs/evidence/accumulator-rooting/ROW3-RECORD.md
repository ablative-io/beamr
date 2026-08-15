# AR-1 ROW 3 — the four UNRULED-PRERESERVE audits: 2 discharged, 2 promoted REAL and fixed

Artemis Peach. No clock in this header — the commit carrying it records the
instant (gate amendment 3's rule). Ground: beamr main `b77c21a` + this
lane's working set. Gate row 3 (AR-1-LANDING-GATE.md): the four
UNRULED-PRERESERVE rows must each be "discharged or promoted" — an exact
audit of the reserve arithmetic against every word the collection sequence
allocates, or promotion to a REAL defect with the full row-4 treatment.

**Population note (binding): these four rows are NOT rows of the 17-site
carrier ledger.** They are pre-reserve arithmetic questions on sites whose
carriers are protected BY the reserve — a separate population.
`dispositions.json` is untouched by this row.

## The safety mechanism being audited (verified at the bytes)

Non-prereserved `alloc_tuple` (context/alloc.rs) and `alloc_list_with_tail`
self-`ensure_heap_space` AND root their own inputs via `with_rooted`. An
exact-or-over OUTER reserve therefore makes every inner ensure a no-op ⇒ no
mid-sequence collection ⇒ the functions' unrooted accumulators never go
stale. The whole guarantee rests on the reserve arithmetic being ≥ the true
word count — which is exactly what these audits check. Word costs: 2-tuple
= 3 (header + 2 elements); cons cell = 2.

## Audit verdicts

| row | site | reserve arithmetic | verdict |
|-----|------|--------------------|---------|
| R-a | ets `bif_info_1` (reserve at the call site) | `info_proplist_heap_words` = 5/item | **AUDITED-EXACT → DISCHARGE.** `info_value` returns only immediates and takes `&ProcessContext` — structurally cannot allocate. Per item: tuple 3 + cons 2 = 5. |
| R-d | system_info `bif_memory_0` | `info_proplist_heap_words` = 5/item | **AUDITED-EXACT → DISCHARGE.** `memory_values` yields immediates only; same 5/item shape. |
| R-b | `bif_process_info_1` (per-item `3 + value_heap_words + 2`) | `value_heap_words` | **PROMOTED TO REAL → FIXED.** Monitors arm counted **4** words per matching monitor; `alloc_monitor_list` consumes **5** (`{process, Pid}` 2-tuple 3 + its cons cell 2). Reserve short 1 word per matching monitor, directly in front of the unrooted `tuples` accumulator. |
| R-c | `bif_process_info_2` (same helper) | `value_heap_words` | **PROMOTED TO REAL → FIXED.** Same defect through the same helper, in front of the unrooted `terms` accumulator. |

Every other `value_heap_words` arm audited exact: CurrentFunction 4 (the
`{M, F, A}` 3-tuple: header + 3 elements; its proplist cons is covered by
the per-item `+ 2`), immediates 0, Links n×2 (pids are immediates, so each
link costs only its cons cell). The Monitors arm was the sole miscount.

## The instrument — three exactness wall tests (committed with the fix)

Pattern: the whole-call `process.heap().young_used()` delta must EQUAL the
reserve recomputed from the SAME production helper. Mutating the helper
moves the promise side, not the measured side, so the wall catches both
under-counted arithmetic and added allocations — and exact equality (not ≤)
also pins every pre-reserve step as heap-allocation-free.

- `process_info_reserve_covers_every_allocation_exactly` — mock facility
  populates all 9 SUPPORTED_ITEMS; Monitors = 2 matching (watcher 7) + 1
  non-matching (watcher 99); Links ×2; CurrentFunction Some.
- `memory_zero_reserve_covers_every_allocation_exactly`
- `info_one_reserve_covers_every_allocation_exactly`

## Red at parent (`row3_red_at_parent.log`)

At the parent bytes (fix not yet applied), the process_info wall FAILED with

    consumed 63 words, reserved 61

— short by exactly 2 = 1 word × the mock's 2 matching monitors, the audit's
predicted magnitude to the word. The memory and ets walls ran GREEN at the
same bytes, corroborating the two discharges. rc 101 from the leg; the
other two walls' `... ok` lines are in the log.

## The fix

`value_heap_words` Monitors arm `* 4` → `* 5`, with the accounting comment
(2-tuple header + 2 elements, plus its cons cell). One-line arithmetic
change; no allocation-path bytes touched. All three walls green after
(`row3_green_after_fix.log`, lib leg `3 passed`).

## Falsifiers — subtract one word from EACH arithmetic (`row3_falsifiers_run.txt`)

A green never observed red is a regression test, not a wall. Three arms
(`row3_falsifiers.py`), each mutating one production arithmetic, running
only the three walls, requiring the victim FAILED while the other two stay
green, restoring byte-identical (sha256-checked before and after):

- F-ets: ets `info_proplist_heap_words` `* 5` → `* 5 - 1` — DIED, others ok
- F-sys: system_info `info_proplist_heap_words` `* 5` → `* 5 - 1` — DIED, others ok
- F-pi: `value_heap_words` Monitors `* 5` → `* 4` (the parent defect
  restored verbatim) — DIED with the parent's exact signature
  (consumed 63 / reserved 61), others ok

Baseline green before the arms; unmutated tree green after; all restores
sha-verified. Each wall is provably sensitive to a single-word shortfall in
its own arithmetic.

## Battery prediction — REGISTERED HERE, PRE-BATTERY, COMMITTED BEFORE LAUNCH

Delta: three new `#[test]`s in the beamr lib binary (one per BIF tests
module) + the one-line Monitors fix + docs/evidence files. NAMED AXES
(result-lines / passed / failed / ignored; result-lines = per-binary
`test result:` lines; the new tests join the existing lib binary, so
result-lines are UNCHANGED). Baseline at `b77c21a`: 2117/2127.

| leg                | predicted axes    |
|--------------------|-------------------|
| tests              | 75 / 2120 / 0 / 0 |
| tests-all-features | 75 / 2130 / 0 / 0 |

All 8 legs rc 0; the three new tests present BY NAME once in each test
leg's log; census 0/0 (battery runs on the committed tree; battery outputs
enter the repo in a follow-up evidence commit). Pre-battery check per the
#78 law: BOTH clippy legs' full commands re-run after the last edit, rc 0
required before launch.

## Row status consequence

Row 3 is CLOSED at my seat pending battery + landing word: R-a/R-d
discharged on exact audit, R-b/R-c promoted to REAL, witnessed red at
parent, fixed, and walled with per-arithmetic falsifiers. The row-4-style
red-at-parent demonstration for this defect is the wall test itself
(committed, fired red at parent bytes, log retained). Still open at my
seat before any carrier-fix design: the remedy-shape proposal under the
205-site blast-radius census vs the shape-cannot-be-written criterion —
likely needs a ruling.
