# Pre-registered falsifier transcripts

## R6(a) — introductions and discriminating line test

Instrument/population: `git log --all --reverse -S`, `git grep` at tags, `git tag --contains`, and touched-file/line history on the ancestry of `43d8781`.

- C1 is absent at `v0.2.0`/`v0.3.15`, present at `v0.4.0`, and absent at `v0.16.3`; first ancestry commit `2c064ed`, with fix `39a6dcc` modifying `process/heap.rs`.
- C2 is absent at `v0.2.0`/`v0.3.15`, present at `v0.4.0`, and absent at `v0.16.3`; first ancestry commit `487aae5`, with fix `e9565d6` modifying the introduced `ets/set.rs` line. `4310e4a` does not touch that file.
- Tag algebra: 33 tags contain both introductions without the fix, 18 predate both, and 17 contain the fix, 12 of them beamr-versioned.

## R6(b) — registry pin split

The committed registry instrument measured all 54 served versions below `0.16.3`. Untagged `0.15.3`, `0.15.4`, `0.16.0`, and `0.16.1` contain both introductions and not the fix; `0.16.2` contains the fix. Zero pins were unresolved.

## R6(c) — controls run at the bytes

1. **Own-instrument arm.** Ran `python3 docs/design/beamr/briefs/evidence/accumulator-rooting/shape_hunt.py` from the repository root. It walked 362 files, printed `PASS  S3a` through `PASS  S3e`, printed `ALL 5 CLASS CONTROLS PASS`, and exited 0. Full stdout and rc are in `r6c-arm1-shape-hunt.txt`.
2. **Blindness finding.** The real RF-006 run produced 74 candidates and zero rows for `crates/beamr/src/ar1_shape_control.rs`, while the live shape hunt found all five controls. This is measured blindness between two instruments, not a source/docstring inference.
3. **Production-visible carrier arm.** Exported `HEAD` to the external local path recorded in `r6c-arm3-carrier.json`, added `crates/beamr/src/probe_carrier.rs`, and ran `r6_sweep.py` plus `r6_stage3.py wide`. The run produced 75 candidates with the carrier and 74 after removing it. The extra row was `probe_carrier_stage`, variable `carrier`, `alloc_tuple`, bind/call/use lines 5/6/7. Both runs exited 0, the exported source is embedded in the receipt, and the self-created export was deleted. No synthetic source entered this tree.

## Red/green receipt

Red-first against base artefacts: `8 failed, 15 passed in 14.40s`. Failing tests were `test_ar1_shape_hunt_receipt_matches_live_tail`, `test_rf006_blind_to_cfg_test_controls`, `test_r6c_arm3_carrier_receipt`, `test_no_regrade_in_r4_artefacts`, `test_r4_tally_is_derived_from_rows`, `test_r4_every_bounded_row_is_ruled`, `test_r4_real_rows_are_findings`, and `test_r4_raw_candidates_are_auditable`.

Green after the measured run: `23 passed in 6.96s`.
