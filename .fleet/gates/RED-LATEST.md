== 1. environment floors ==
  disk free: 395G
== 2. tree receipt ==
  HEAD: 1fd8b65468f60cf242cc172a05063bf260def068
  TREE: CLEAN
== 3. paths census ==
  census OK (32 files in wall, 24 under crates/)
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
  leg nostd-ratchet rc=3 — last 30 lines:
    mktemp: too few X's in template 'nostd-ratchet'
    ./scripts/gate-nostd-ratchet.sh: line 203: : No such file or directory
    ./scripts/gate-nostd-ratchet.sh: line 205: : No such file or directory
    no-std ratchet: cargo rc=1, rustc tally=<absent>, ceiling=1051
    REFUSE: cargo exited 1 but no "due to N previous errors" line was
      found. This gate cannot measure, so it does not get to report.
      A green here would be worth nothing -- same value whether the tree
      is sound or the instrument is dead. Fix the parse; do NOT relax it
      to make this pass.
  LEG 9 (nostd-ratchet) rc=3
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
  nostd-ratchet: rc=3 FAIL — measured red
  tests-all-features: rc=0 pass
  tests: rc=0 pass
  wasm-tests: rc=0 pass
  wasm32-check: rc=0 pass
GATE-RED: canon verdict rc=1 (named cause: canon_red — read the leg logs above)
