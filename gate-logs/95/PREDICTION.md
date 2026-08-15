# #95 battery prediction — COMMITTED BEFORE LAUNCH

Tree: fix commit 7e318c0 (= origin/main bfaa85c + the #95 Type-chunk carry).
Battery: gate-logs/103/battery-RUNNER.sh, all 8 legs, per-leg rc tsv is THE
VERDICT.

Named axes, both cargo-test legs:
- result-lines: **75** (unchanged — no new test binary; the 3 new tests live
  in the existing lib harness and the existing encode_round_trip binary)
- passed: **2123** on the default∪encode leg (was 2120; +3 = 1 lib test
  `typed_registers_round_trip_when_the_type_chunk_is_present` + 2 integration
  tests `type_chunk_is_carried_verbatim_and_typed_registers_encode`,
  `typed_registers_without_a_chunk_still_refuse`)
- passed: **2133** on the all-features leg (was 2130; same +3)
- failed: **0** · ignored: **0**
- all 8 legs rc **0**

Also pre-stated: the committed-fixture ratchet line inside the
encode_round_trip leg reads "105 fixtures, 0 refused" (measured pre-battery,
tally at gate-logs/95/ratchet-tally.log) — the #95 refusal denominator's
55 → 0, with zero fixtures flipping to Failed. Pre-battery check done: both
clippy FULL legs rc 0 AFTER the last edit (encode leg + all-features leg).
