GATE SCOPE: beamr's own CI verdict harness (gates.json), the real mechanism
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
  tests: 66 result line(s), 2127 passed
  tests-all-features: 66 result line(s), 2137 passed
  blocking-call-in-native-bif: rc=0 pass
  clippy-all-features: rc=101 FAIL — measured red
  clippy: rc=101 FAIL — measured red
  fmt: rc=0 pass
  nostd-ratchet: rc=3 FAIL — measured red
  tests-all-features: rc=101 FAIL — measured red
  tests: rc=101 FAIL — measured red
  wasm-tests: rc=0 pass
  wasm32-check: rc=0 pass
