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
  tests: 66 result line(s), 2126 passed
  tests-all-features: 0 result line(s), 0 passed
TEST-COUNT: tests-all-features emitted no 'test result:' line — it ran no test binary, which is not a pass
