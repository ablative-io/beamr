# RUN RECORD — #78 D2 amended in place + always-on-native pin

Lane: #78 "jit feature contradicts design record D2 — RULING FIRST, not a
cfg patch." Ruled by Waffles 2026-08-14 (~13:52Z): shape (A) — amend D2 in
place; (B) refused (cause b kills it: target-cfg cannot restore the letter
without decoupling jit from threads — a design change serving a sentence);
(C) refused (superseding orphans D2's history). Two sharpenings applied:
cost stated honestly; intent pinned by a NAMED test, not prose.

## Ground (assembled 2026-08-14, all measured at the bytes)

- The record: docs/design/beamr/design.json, decisions[1] (id D2,
  "Cranelift as always-on dependency") — "no cargo feature flag. The JIT
  compiler ships in every build"; rejected option: "Feature-gated
  dependency (conditional compilation complexity...)".
- The tree: crates/beamr/Cargo.toml:79 `jit = ["std", "threads",
  dep:cranelift-*]`, cranelift optional; 58 `feature = "jit"` cfg sites
  across 7 files; jit sits in DEFAULT features.
- TWO causes, both post-dating D2 (feature born in the wasm scaffold
  commit 1233685, 2026-06-10):
  (a) cranelift does not target wasm32-unknown-unknown — both wasm gate
      legs build beamr-wasm, which consumes beamr
      `default-features = false, features = ["cooperative", "json"]`;
  (b) `jit` REQUIRES `threads`, so the single-threaded cooperative
      profile excludes it structurally — the feature is the COMPOSITION
      MECHANISM for a threads-coupled backend, not merely a target
      exclusion.
- D2's intent survives at the bytes: default builds always carry JIT.
  Only the letter ("no feature flag") was overtaken.
- Namespace note for future readers: docs/design/beamr/DESIGN.md:203 has
  an UNRELATED decision also labeled "D2" (atom table). This lane's D2 is
  design.json's decisions[1].

## The change (commit 70ce683)

- docs/design/beamr/design.json: one `"amendment"` key added to D2
  (textual single-line insert; an earlier json.dump round-trip was
  DISCARDED because it reformatted the whole file — the diff is 2
  insertions / 1 deletion, nothing else). The amendment keeps the
  original chosen/rationale/rejected untouched, states the price
  honestly (rightly rejected THEN, bought at 58-sites cost LATER, dated
  to 1233685), and cites the pin by name.
- crates/beamr/tests/jit_wireup.rs: new test
  `jit_stays_in_default_features_d2_always_on_native` — asserts
  `cfg!(feature = "jit")` in the default-features suite. Existing binary
  on purpose: no new result-line, no process-global state involved.

## Falsifier (committed-first; restore sha-equal b643ec3f…)

Mutation: `jit` removed from the default features list (exact-match,
asserted matched once). Result: the tests-leg build goes RED at COMPILE —
beamr (lib) fails with 7 errors (E0432/E0433, `pub mod jit` configured
out) before any test runs. Log: falsifier-default-features.log.

Recorded honestly: the pin's own assert was NOT directly observed red —
it cannot be, because today a compile wall stands in front of it. The
pin guards the world where that wall is removed: if someone cfg-gates the
remaining consumers so a no-jit tree compiles cleanly (the natural
"cleanup" path), every gate would go green with default builds silently
JIT-less — except this assert. Defense in depth: compile wall today,
named pin when the wall erodes. Green direction verified: the pin runs
and passes under shipped bytes in both test legs (by NAME).

## Battery 1 (canon 8-leg, at 70ce683) — RED, kept in evidence

Runner: gate-logs/103/battery-RUNNER.sh reused byte-identical.
Prediction pre-registered in PREDICTION.md BEFORE any leg output:
result-lines UNCHANGED 75 both legs (the counter counts per-binary
`test result:` lines; the pin joins an existing binary), passed +1 both
legs (2115→2116, 2125→2126), confirmed by test NAME in each leg's log.

RESULT: legs 2 and 7 (clippy, clippy-all-features) rc 101 —
`clippy::assertions_on_constants` at the pin (cfg! resolves at compile
time, so the assert is constant — which is the pin's whole point, but
undeclared to the linter). Test legs matched the prediction EXACTLY
(75/2116/0/0 and 75/2126/0/0, pin by NAME in both). ⚠️ The runner's
COMPLETE marker printed anyway: it derives from scored==declared + pin
stability only, NOT per-leg rc — the per-leg tsv is the verdict, the
marker is not. This is a live in-house datum for the #84 class
(markers/verdict-collapse) and is recorded here for that lane.

Erratum (mine, cost one battery): the pre-battery check ran the full
clippy leg command BEFORE the pin test existed and only a narrowed
`--example` check after — the leg's ACTUAL command must re-run after the
LAST edit. Logs kept: battery-RED-70ce683.*.

## The fix (commit 69a7df2) + Waffles' pin-site sharpening

His sharpening on the compile-wall finding: it must live AT THE PIN SITE
— gate logs do not travel to the desk of the future engineer who takes
the wall down. Executed as the pin's doc comment: records that no-jit
fails at COMPILE today (7 errors), the assert has never been observed to
fire, and whoever cfg-gates the remaining consumers must red-prove this
pin as part of that cleanup. The clippy allow is scoped and states its
reason in place: the constancy IS what is being pinned. Both clippy leg
commands verified green at full form before commit; pin still green by
name.

## Battery 2 (canon 8-leg, at 69a7df2)

Prediction re-registered (PREDICTION.md) before any leg output: identical
axes (75/2116/0/0, 75/2126/0/0), all 8 legs rc 0, pin stable at 69a7df2.

DISCLOSURE (sequencing erratum, mine): the falsifier log was copied into
gate-logs/78/ BEFORE the battery launched, so the runner's tree census
reads 1/1 (pre/post) instead of 0/0 — the single untracked evidence file
gate-logs/78/falsifier-default-features.log, identical at both ends,
readable by no leg. The pin (HEAD sha) is the battery's binding to the
bytes and is stable; the census residue is disclosed with its identity,
not absorbed.

RESULT: GREEN — all 8 legs rc 0 AT THE PER-LEG TSV (the verdict; the
marker agrees but is not it). Measured axes vs the re-registration, exact
to the digit:

| leg                | predicted         | measured          |
|--------------------|-------------------|-------------------|
| tests              | 75 / 2116 / 0 / 0 | 75 / 2116 / 0 / 0 |
| tests-all-features | 75 / 2126 / 0 / 0 | 75 / 2126 / 0 / 0 |

Pin by NAME (`jit_stays_in_default_features_d2_always_on_native ... ok`)
once in each test leg's own log. Pin post
69a7df232933c9133a3258259f28891a249c61dc, marker COMPLETE 8/8 pin
stable. Census post 1 = the same disclosed falsifier log, sole untracked
path both ends.

DISCLOSURE (capture artifact): the background task's stdout capture
clipped the runner's header lines (pin-pre, tree-pre, OPEN) — BATTERY.log
starts at leg 1's rc. The verdict does not rest on the clipped lines: the
per-leg tsv, all eight leg logs, pin-post, and the COMPLETE marker
survived intact; tree-pre is attested by the dispatcher's own census at
launch (identical single untracked path) and pin stability is the
runner's own pre/post comparison, printed in the surviving marker line.

Battery logs: BATTERY.log, legs.tsv, leg5/leg8 (axes witnesses).
Battery-1 red logs kept as battery-RED-70ce683.{tsv,leg2,leg5,leg7,leg8}
— legs 2/7 are the red witnesses (rc 101, assertions_on_constants),
legs 5/8 the exact-axes witnesses from the same run.
