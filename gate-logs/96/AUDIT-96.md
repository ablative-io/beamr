# AUDIT-96 — which cfg-gated test code does the canon battery never execute?

Artemis Peach, 2026-08-16. Tree: `200f425` (origin/main at audit time). Host:
macOS (Darwin 25.3.0) — the box the canon battery runs on.

## Premise, corrected before measuring

The naive premise ("feature-gated tests are unreached") died at #97/#98: the
canon's all-features legs already compile telemetry/json/test-support/encode
inline tests. The live question is **structural**: gates.json's own note says
the canon compiles exactly THREE feature points (default∪encode ·
cooperative+json no-default (wasm32 check, type-check only) · all-on), and
all-features turns `threads` ON — so anything gated `not(feature = …)` can
never be compiled by a test-running leg. Is any TEST so gated?

## Instruments (two, independent)

**1. The compiler** (`cargo test … -- --list`, names diffed — not grep):

| point | command | rc | tests |
|---|---|---|---|
| A (canon default∪encode) | `cargo test --workspace --features beamr/encode -- --list` | 0 | **2120** |
| B (canon all-features) | `cargo test --workspace --all-features -- --list` | 0 | **2130** |
| C (not-threads host point) | `cargo test -p beamr --no-default-features --features cooperative,json -- --list` | 0 | **2069** |

- A = 2120 and B = 2130 match the battery's own per-leg denominators
  EXACTLY (row3-battery2 tsv) — the enumeration instrument and the battery
  corroborate each other.
- **The "may not compile on host" hypothesis is REFUTED**: point C compiles
  and runs `--list` cleanly on the host, rc 0.
- **C − (A ∪ B) = 0 test names.** Nothing is enumerable at the not-threads
  point that the canon does not already enumerate.
- B − A = 10 tests, all telemetry-gated (4 `scheduler::tests` spans/metrics +
  5 `telemetry::lifecycle::tests` + 1 `telemetry::spans::tests`) — all
  executed by the all-features legs. Named in full in the enumeration
  artifacts.
- Enumeration artifacts (scratchpad, sha256):
  `t96-list-canonA.txt 93fc74ca…dda55` · `t96-list-canonB.txt 1860c9ba…c902`
  · `t96-list-pointC.txt 1ccb0a7b…135d`.

**2. The syntax** (rg over `crates/`, guards the one hazard a name-diff cannot
see — a `not(threads)` test whose name collides with a threads-on twin):

- `not(feature = "…")` occurs at **29 sites, all inside crates/beamr, zero in
  any other crate** — and every one is PRODUCTION code (no-op method bodies,
  const strings, cooperative IO-sink branch, non-telemetry send/remove arms,
  non-jit dispatch stubs, non-embedded replay passthrough, a no_std `use`).
  **Zero gate a `#[test]` or a `mod tests`.** Each site was opened and read,
  not counted.
- Positively-gated test cfg census (threads 231 · telemetry 153 · net 91 ·
  readiness 64 · jit 58 · test-support 25 · cooperative 18 · …): every
  positive gate is satisfied at the all-features point, so all are compiled
  by the tests-all-features legs.

The two instruments RECONCILE: no negatively-gated tests exist (syntax), and
the not-threads point enumerates no test the canon misses (compiler).

## beamr-wasm

Covered by its own canon leg (wasm-tests, 80 passed at
`row3-battery2.leg4.log`), not by the host diff above. Point C's type-check
role for the wasm32 shape is unchanged.

## OS-gated tests (found while sweeping, recorded for the CI residue)

All `target_os` gates in test files are BLOCK-level (an assertion arm inside
a test that runs on both platforms) except **one whole test**:
`scheduler::inventory_tests::service_inventory_threads_are_all_live_in_the_os_probe`
(`inventory_tests.rs:334`) is `#[cfg(target_os = "macos")]`. It IS enumerated
and run on this box. **Forward-looking datum: when GitHub CI (ubuntu) is
enabled, its --lib denominator will be exactly one lower than the local
battery's** — whoever wires the CI denominator expectation must not copy
2120/2130 verbatim. Filed with the CI-enablement residue (standing finding:
Actions currently `disabled_manually`).

## Verdict

**The canon battery executes every cfg-gated test the workspace contains; no
new leg is needed.** The audit's original suspicion (a not-threads test
population invisible to all three compile points) is measured EMPTY at two
independent instruments. The only reachability boundary is the deliberate one:
beamr-wasm's wasm-gated tests run under the wasm-tests leg, and the wasm32
no-threads SHAPE is type-checked (not test-run) by the canon's second compile
point — which this audit confirms compiles on host too, adding zero tests.

No production bytes changed; no battery owed (docs/evidence-only landing).
