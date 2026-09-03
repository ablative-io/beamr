# Memory-safety census at `43d8781`

This record closes the advisory's unmeasured introduction claim. It reads product code at `43d87819ed37515695a697b7416d47c6740502e2`; it does not change product code, enable lints, or add SAFETY comments.

## R2 — published population

Instrument: `instruments/registry_census.py`. Population: all 66 crates.io index rows for `beamr`, narrowed to every one of the 54 versions below `0.16.3`. For each served `.crate`, the instrument reads `.cargo_vcs_info.json` and tests the immutable pin with `git merge-base --is-ancestor` against C1 `2c064ed`, C2 `487aae5`, and fix `67f89c4`.

Measured: 35 affected published versions (33 live and two yanked), 18 pre-C1 clean, zero unresolved. Published `0.16.2` is not affected because pin `5206e7af16e8985c774a7b6bb7fee6aadff02095` contains the fix. See `registry-population.json` and `registry-population.md`.

## R3 — unsafe sites by advisory class

Instrument: `instruments/census_unsafe.py`, equivalent population command `git grep -n -w unsafe 43d87819 -- crates/beamr/src`. Population: 327 Rust files. Same-instrument `from_raw_parts` positive control: five lines.

Measured: 200 grep lines in 48 files: 197 code sites and three prose lines. Code kinds are 192 blocks, two unsafe functions, two unsafe implementations, and one unsafe extern. The four-preceding-line SAFETY window finds 166 documented and 31 undocumented code sites. The class tally measured by the instrument is C1 2, C2 1, C3 30, RF-006 29, AR-1 0, NONE 138 (including the three prose rows). Every NONE row states the actual low-level mechanism rather than merely saying it is unrelated.

Region population over the 197 code sites: io 57, term 38, jit 29, native 25, scheduler 14, gc 10, interpreter 8, supervision 5, mailbox 3, distribution 2, etf 2, process 2, constant_pool 1, ets 1. JIT files: runtime_binary_match 7, runtime_closure 6, runtime 6, compiler_tests 5, runtime_binary_build 2, runtime_map 2, runtime_message 1.

A12 is venue-typed: on macOS/Apple git 2.47.1 the ERE word-boundary form returns 0; on this 205-class venue (`git version 2.47.3`, GNU regex) the same command returns 200. The same-instrument `git grep -w from_raw_parts` control returns five on both venues. The erratum is a fact about Artemis' seat, not a proposition under test.

No `unsafe_code` or `undocumented_unsafe_blocks` lint is configured in the tree. This census records the 31 missing local arguments and writes no comments.

## R4 — JIT-reachable crossings

Run from the repository root: `python3 docs/design/beamr/briefs/evidence/memsafe-census/instruments/jit_sweep.py --out-dir docs/design/beamr/briefs/evidence/memsafe-census`. The wrapper invokes RF-006's real `r6_sweep.py` and `r6_stage3.py wide`, which measured 74 candidates: 57 in `crates/beamr/src` and 17 out-of-population `beamr-wasm` rows. Its 44-root call walk follows `crates/beamr/src` to depth four and stops at interpreter/scheduler boundaries; `jit-walk-edges.json` records the edges.

Arm P, R4's bounded population, contains six hand-ruled rows: FP-D 3, FP-F 1, SAFE-FIXED 2, REAL 0. Arm W contains the other 51 beamr rows: FP-D 14, FP-E 6, SAFE-A 2, SAFE-C 16, SAFE-OFFHEAP 1, SAFE-REREAD 1, SAFE-ROOTED 6, UNRULED-PRERESERVE 5, REAL 0. All three surviving historical REAL carriers are explicitly SAFE-ROOTED at their repaired bytes. Relative to the 69-row prior, 57 keys persist, 12 are gone, and 17 are new. The brief's four carried `UNRULED-PRERESERVE` rows are all outside Arm P, so their expected prior belongs to the whole-crate Arm W axis; a fifth new pre-reservation row is recorded in Arm W, and forcing any of them into the bounded set would be another regrade. Round 1's programmatic regrade is deleted: every current verdict comes from `jit-rulings.json`, and a non-zero REAL would remain a finding.

## R5 — reconciliation

Instrument: the cambium four-rule ast-grep pack archived read-only from actual local mirror pin `c09ddde2043e8e5104e1c5e2b6edddf8358adace`. Population: the archived repository tree. Pack findings reproduce exactly: 1,054 mod-rs declarations, 277 let-underscore results, 16 lint bypasses, zero std-Mutex-in-async; pack total 1,347. This ast-grep binary emitted seven separate unused-suppression diagnostics, not the brief's nine, so this run's grand total is 1,354 rather than 1,356.

The ast-grep pack does not read unsafe. Its 1,347 rule findings and R3's 200 unsafe lines are orthogonal populations reconciled by name; no subtraction or delta between them has meaning. The positive-control export containing `allow(unsafe_code)` produced one lint-bypass hit without touching this tree. The 277 let-underscore results include 79 in exact fleet-record path `gate-logs/13/census-instrument.rs`, leaving 198 product-code hits. See `reconciliation.md` and `reconciliation.json`.

## R6 — falsifiers

`tests/test_falsifiers.py` machine-checks adjacent-tag flips, first-introduction discrimination, line/file modification, tag algebra, registry pin split, the live AR-1 instrument, measured RF-006 cfg-test blindness, the real external carrier receipt, explicit rulings, findings, and row-derived tallies. `tests/test_instruments.py` checks populations and output partitions. Red-first: 8 failed and 15 passed against the base artefacts. Green: 23 passed after the measured run.

## Files

- `registry-population.json`, `registry-population.md` — R2
- `unsafe-sites.json` — R3
- `jit-crossings.json`, `jit-walk-edges.json`, `jit-candidates-main.json`, `jit-rulings.json` — R4
- `reconciliation.json`, `reconciliation.md` — R5
- `falsifiers.md`, `r6c-arm1-shape-hunt.txt`, `r6c-arm3-carrier.json`, `test-deletion-receipt.md`, `tests/` — R6
- `instruments/` — reproducible census programs
