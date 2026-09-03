# Cross-instrument reconciliation

Instrument: ast-grep scan with cambium `c09ddde2043e8e5104e1c5e2b6edddf8358adace` over the archived product tree. Population: four pack rules over the repository; the 200-line unsafe census is orthogonal because this pack does not read `unsafe`.

| population | count | meaning |
| --- | ---: | --- |
| unsafe grep lines | 200 | R3 only; no subtraction against ast-grep |
| ast-grep four-rule pack | 1347 | 1,054 mod.rs + 277 let-underscore + 16 lint-bypass + 0 std-Mutex-in-async |
| ast-grep unused-suppression | 7 | scanner diagnostic, not a pack rule |
| ast-grep grand total | 1354 | brief said 1,356; this binary yields 1,354 because 9 became 7 |

The populations reconcile by name only; no numeric delta between them is meaningful.

## Subject touchpoints

- The 16 lint bypasses comprise 14 `clippy::` allows and two bare `dead_code` findings at `interpreter/pattern.rs:8` and `term/compare/mod.rs:100`.
- Product `allow(unsafe_code)` count is zero. The external scratch positive control produced 1 `no-lint-bypass-attributes` hit.
- `let _`: 277 raw, 79 in the exact fleet-record path `gate-logs/13/census-instrument.rs`, 198 product-code hits after that named exclusion.
- Gap: no memory-safety rule exists in the pack. Proposed future rule: count unsafe blocks and whether a SAFETY comment appears in the preceding four lines; proposal only.
