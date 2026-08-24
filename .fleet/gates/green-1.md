# GATE-GREEN — fix round 1, beamr-r8-flight1

Taken at HEAD `1b69bbc468217aef05fae3c2ca91534fd6756948`, tree CLEAN.
The only delta between that pin and this file's own commit is this
receipt — a markdown file under `.fleet/`, inside the wall, touching
no leg. Prior rounds: `red-1.md`.

```
== 1. environment floors ==
  disk free: 386G
== 2. tree receipt ==
  HEAD: 1b69bbc468217aef05fae3c2ca91534fd6756948
  TREE: CLEAN
== 3. paths census ==
  census OK (36 files in wall, 24 under crates/)
== 3b. ratchet judgment lock (the named door's lock) ==
  locked: CEILING, CEILING_PIN, verdict(), parse_tally() byte-identical to 272ca9e034b0f78f50e30ec1068ea1eca967e6d6
== 4. suppression sweep ==
  clean
== 5. the canon (gates.json legs, run as ci.yml runs them) ==
  gates.json declares 9 legs
  -- leg 1/9: fmt
  LEG 1 (fmt) rc=0
  -- leg 2/9: clippy
  LEG 2 (clippy) rc=0
  -- leg 3/9: wasm32-check
  LEG 3 (wasm32-check) rc=0
  -- leg 4/9: wasm-tests
  LEG 4 (wasm-tests) rc=0
  -- leg 5/9: tests
  LEG 5 (tests) rc=0
  -- leg 6/9: blocking-call-in-native-bif
  LEG 6 (blocking-call-in-native-bif) rc=0
  -- leg 7/9: clippy-all-features
  LEG 7 (clippy-all-features) rc=0
  -- leg 8/9: tests-all-features
  LEG 8 (tests-all-features) rc=0
  -- leg 9/9: nostd-ratchet
  LEG 9 (nostd-ratchet) rc=0
== 6. verdict (scripts/ci-verdict.sh — the one copy of the truth) ==
self-test: all-green -> gamma: rc=0 pass (expected)
self-test: one-fail -> FAIL — measured red (expected)
self-test: cannot-measure -> CANNOT-MEASURE (expected)
self-test: uncontracted-2 -> alpha: rc=2 FAIL (expected)
self-test: malformed-rc -> MALFORMED rc (expected)
self-test: truncated-set -> LEG COUNT MISMATCH (expected)
self-test: empty-tests -> TEST-COUNT (expected)
declared legs: 9
recorded legs: 9
  wasm-tests: 2 result line(s), 86 passed
  tests: 85 result line(s), 2196 passed
  tests-all-features: 85 result line(s), 2206 passed
  blocking-call-in-native-bif: rc=0 pass
  clippy-all-features: rc=0 pass
  clippy: rc=0 pass
  fmt: rc=0 pass
  nostd-ratchet: rc=0 pass
  tests-all-features: rc=0 pass
  tests: rc=0 pass
  wasm-tests: rc=0 pass
  wasm32-check: rc=0 pass
GATE-GREEN: 386G free; tree CLEAN at 1b69bbc468217aef05fae3c2ca91534fd6756948; census in wall (24 crate files), suppressions 0; all 9 canon legs green via ci-verdict.sh against 272ca9e034b0f78f50e30ec1068ea1eca967e6d6
```
