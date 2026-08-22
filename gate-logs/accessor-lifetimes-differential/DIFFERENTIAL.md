# ACCESSOR-LIFETIMES DIFFERENTIAL — leg for leg

base = c55ac360 (origin/main)   land = e1cb3060 (landing head)

| leg | base rc | land rc | base secs | land secs | verdict |
|---|---|---|---|---|---|
| fmt | 0 | 0 | 1 | 1 | SAME rc |
| clippy | 101 | 101 | 21 | 20 | SAME rc |
| wasm32-check | 0 | 0 | 13 | 13 | SAME rc |
| wasm-tests | 0 | 0 | 18 | 18 | SAME rc |
| tests | 101 | 101 | 143 | 144 | SAME rc |
| blocking-call-in-native-bif | 0 | 0 | 0 | 0 | SAME rc |
| clippy-all-features | 101 | 101 | 6 | 7 | SAME rc |
| tests-all-features | 101 | 101 | 130 | 144 | SAME rc |
| nostd-ratchet | 3 | 3 | 0 | 0 | SAME rc |

## leg: fmt

- base: rc=0, 0 error/REFUSE lines
- land: rc=0, 0 error/REFUSE lines

## leg: clippy

- base: 15 diagnostics extracted
- land: 15 diagnostics extracted

**multiset (level,lint,file,message): base=15 land=15**

✅ **MULTISETS EQUAL.** Every diagnostic present in one arm is present
in the other with the same multiplicity.

1 of them MOVED (line drift only, expected from the ripple):
  - `clippy::unnecessary_cast` crates/beamr/src/native/file_meta_bifs.rs : base lines [251, 252, 253, 254, 255, 255, 263, 264] -> land lines [253, 254, 255, 256, 257, 257, 265, 266]

## leg: wasm32-check

- base: rc=0, 0 error/REFUSE lines
- land: rc=0, 0 error/REFUSE lines

## leg: wasm-tests

- base: 86 passed / 0 failed / 0 ignored across 2 suites
- land: 86 passed / 0 failed / 0 ignored across 2 suites

✅ **FAILING-TEST MULTISETS EQUAL** (0 failing test names, identical both arms).

## leg: tests

- base: 2126 passed / 1 failed / 0 ignored across 66 suites
- land: 2127 passed / 1 failed / 0 ignored across 66 suites

✅ **FAILING-TEST MULTISETS EQUAL** (1 failing test names, identical both arms).
  - distribution_runtimes_exist_only_when_configured_and_are_joined_at_shutdown

## leg: blocking-call-in-native-bif

- base: rc=0, 0 error/REFUSE lines
- land: rc=0, 0 error/REFUSE lines

## leg: clippy-all-features

- base: 15 diagnostics extracted
- land: 15 diagnostics extracted

**multiset (level,lint,file,message): base=15 land=15**

✅ **MULTISETS EQUAL.** Every diagnostic present in one arm is present
in the other with the same multiplicity.

1 of them MOVED (line drift only, expected from the ripple):
  - `clippy::unnecessary_cast` crates/beamr/src/native/file_meta_bifs.rs : base lines [251, 252, 253, 254, 255, 255, 263, 264] -> land lines [253, 254, 255, 256, 257, 257, 265, 266]

## leg: tests-all-features

- base: 2136 passed / 1 failed / 0 ignored across 66 suites
- land: 2137 passed / 1 failed / 0 ignored across 66 suites

✅ **FAILING-TEST MULTISETS EQUAL** (1 failing test names, identical both arms).
  - distribution_runtimes_exist_only_when_configured_and_are_joined_at_shutdown

## leg: nostd-ratchet

- base: rc=3, 1 error/REFUSE lines
      REFUSE: cargo exited 1 but no "due to N previous errors" line was
- land: rc=3, 1 error/REFUSE lines
      REFUSE: cargo exited 1 but no "due to N previous errors" line was

