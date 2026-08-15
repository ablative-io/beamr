# AR-1 ROW 1 — the ordering-sensitive detector: BUILT, controlled, baselined

Artemis Peach. No clock in this header — the commit carrying it records the
instant (gate amendment 3's rule). Ground: beamr main `6566acc` + this
lane's working set. Gate row 1 (AR-1-LANDING-GATE.md): ⛔ SIGN-BLOCKING —
"the detector is ordering-sensitive, not presence-sensitive."

## What was built

1. **`ordering_detector.py`** (this directory) — grades the 17 ledger
   carriers, RE-DERIVED at the current bytes (the ledger contributes only
   file/function/carrier; every line number is measured fresh). The rule:
   FLAG when the first collecting call on the carrier's live range precedes
   the first rooting call naming the carrier — `with_rooted` / `rooted_push`
   / the carrier handed to `alloc_{list,tuple,map,cons}`. The terminal-alloc
   arm is what makes "roots it, but too late" an ordering fact rather than a
   presence fact. Verdicts: COLLECT-FIRST/FLAG · NO-ROOTING/FLAG ·
   ROOTED-FIRST/CLEAN · NO-COLLECT/CLEAN; UNGRADEABLE is loud (rc 4);
   `--sign-off` refuses while any carrier flags (rc 5). Declared limits in
   the docstring (per-line reading errs toward FLAG, the safe direction).
2. **`crates/beamr/src/ar1_ordering_control.rs`** — the row's break-it arm
   and control, COMMITTED so they fire every run, not once: BAD-ORDER
   (rooting present but after the collect — MUST FLAG; a presence-sensitive
   instrument clears it) and GOOD-ORDER (rooting scope opens first — MUST
   CLEAR). Markers are binding names (`ord_bad_tail`/`ord_good_tail`),
   uniqueness-asserted (0 or ≥2 = UNUSABLE, never PASS). Local types only;
   `#[cfg(test)]`; designed to add ZERO `shape_hunt.py` hits so that
   instrument's ruled ledger reconciliation is untouched — **verified by
   re-run**: 39 raw · 27 production · 12 cfg(test), the gate's pinned counts
   exactly, all 5 class controls PASS (`row1_shape_hunt_census.txt`).
3. **`ordering_detector_mutations.py`** — four deaths shown ONE-TO-ONE
   (`row1_mutations_run.txt`): A presence-degradation → caught by ORD-BAD
   (THE row-1 death); B flags-everything → caught by ORD-GOOD alone (ORD-BAD
   stays green under B, so the credit is unambiguous); C marker rename →
   caught by the uniqueness assert; D ledger row deleted → caught by the
   denominator check (rc 4). M0 green before and after; fixture and ledger
   restores sha256-verified.

## Baseline verdict at these bytes (`row1_baseline_run.txt`)

**17 FLAG · 0 CLEAN · 0 UNGRADEABLE; both controls PASS; rc 0.** Every
carrier in the population is measurably mis-ordered TODAY — including the
eleven that carry a (too-late) rooting call, which is the exact reading a
presence instrument cannot produce. Sites 15/16/17 read NO-ROOTING because
their terminal is the local helper `alloc_sorted_map` (rooting on the far
side of a function boundary): the per-function verdict is factually right
and fail-safe, and it means the eventual fix must root the carrier in ITS
OWN frame — the instrument's conservatism points at the correct fix shape.

## FINDING — ledger site 17's function field was stale AT THE LEDGER'S OWN COMMIT

The detector's first run REFUSED site 17 ("carrier let-bind found in 0 fn
candidates") rather than approximating. Measured: at the ledger's own
commit (`1f7e675`), bind line 199 already sat inside `fn object_to_term` —
the extraction commit (`252a250`, "feat(wasm): add direct JS term
conversion") is an ANCESTOR of the ledger. The `value_to_term` name was
inherited from the sweep-era ground without re-verification against the
bytes the ledger was committed on — the inheritance class again ("a field
attached to a finding is inherited by its remedy"). Corrected in
`dispositions.json` with the original value preserved in a
`function_erratum` field; defect, carrier, bind line, and the row-4
red-at-parent demonstration are unaffected (the probe drives the JsValue
path, which reaches this arm through `value_to_term`'s dispatch).
`ledger_check.py` re-run green over the correction. **For Cally's grading:
this is an edit to a pre-registered artifact — address correction only,
population unchanged, flagged here rather than made quietly.**

## Battery prediction — REGISTERED HERE, PRE-BATTERY, COMMITTED BEFORE LAUNCH

Delta: one new `#[cfg(test)]` module (`ar1_ordering_control`, one `#[test]`)
+ one `mod` line in lib.rs + docs/evidence files. NAMED AXES
(result-lines / passed / failed / ignored; result-lines = per-binary
`test result:` lines; the new test joins the existing beamr lib binary, so
result-lines are UNCHANGED):

| leg                | predicted axes    |
|--------------------|-------------------|
| tests              | 75 / 2117 / 0 / 0 |
| tests-all-features | 75 / 2127 / 0 / 0 |

All 8 legs rc 0; new test present BY NAME
(`both_ordering_fixtures_are_live`) once in each test leg's log; pin stable;
census 0/0 (battery runs on the committed tree; battery outputs go to the
scratchpad and enter the repo in a follow-up evidence commit). Pre-battery
check per the #78 law: BOTH clippy legs' full commands re-run after the
last edit, rc 0 required before launch.

## Row status consequence

Row 1's instrument now EXISTS with both its break-it arm and its control
committed and firing per run. The row is BUILT, not signed off — sign-off
belongs to fix time, when `--sign-off` must pass on 17 dispositioned
carriers with these same controls green. Rows still open at my seat before
any fix design: row 3 (the four UNRULED-PRERESERVE arithmetic audits) and
the remedy-shape proposal under the blast-radius census.
