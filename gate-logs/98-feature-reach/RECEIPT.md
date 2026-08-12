# beamr #98 FEATURE-REACH — battery receipt

Base `65710ac`. Runner beside this file; **seven** canon legs read from the
committed `gates.json` at run time, never transcribed — the runner takes its leg
count from the file, so adding a leg changed what ran with no edit to it.

## Verdict: 7/7 rc 0. COMPLETE marker DERIVED (legs_declared=7, legs_scored=7).

Tree check: **0 pre, 0 post**. Interpreter logged: `/usr/bin/python3`, Python
3.9.6. Axes **73 / 2110 / 0 / 0** — unchanged from #97, and that was the
prediction: this lane adds no Rust test.

## The question this lane asked

#97 applied Vesper's deleted-checker test to the CHECKER. This lane applied it
to the **target**: *would this leg still report zero if part of the tree fell
out of its reach?*

## Crate reach — measured, hazard ABSENT

| crate | fmt names it | clippy names it |
| --- | --- | --- |
| beamr | ✅ | ✅ |
| beamr-cli | ✅ | ✅ |
| beamr-wasm | ✅ | ✅ |
| gleam-types | ✅ | ✅ |

Control green on the untouched tree in both legs; each arm cites
`clippy::len_zero` — the lint claimed — with **zero** `dead_code` mentions, so no
arm went red for a reason other than the one under test.

⭐ **A negative result, and it is worth having.** This question's previous state
was "reasoned about". The `native/`-shaped hard-coded-population hazard does not
exist for fmt or clippy: both resolve through cargo's `--all`/`--workspace`.

## Feature reach — the defect

`telemetry` is enabled by **nobody**: not beamr-cli (default), not beamr-wasm
(`default-features = false`, cooperative + json), not the clippy/tests legs'
`--features beamr/encode`, not dev-dependencies. It gates four files and **156**
cfg sites.

A deliberate **type** error planted in `telemetry/spans.rs`:

| leg | rc with the specimen planted |
| --- | --- |
| fmt | 0 |
| clippy | 0 |
| wasm32-check | 0 |
| wasm-tests | 0 |
| tests | 0 |
| blocking-call-in-native-bif | 0 |

**Six green.** Positive control `cargo check -p beamr --features telemetry` →
rc 101, and its attribution was audited rather than its colour accepted:
**exactly one error, E0308, at `spans.rs:391:17`** — the planted line. Telemetry
compiles fine today; it is simply never compiled by anything that gates.

⚠️ **The specimen is a TYPE error, not a syntax error, on purpose.** rustfmt
walks `mod` declarations syntactically and does not honour `#[cfg]`, so a syntax
error would have gone red at the fmt leg for a reason that is not the claim.

## The fix, and its own falsifier

New leg `clippy-all-features`, **appended** — `runner.sh` reads axes from
`leg-5-tests.log`, and appending keeps that index stable.

| arm | rc |
| --- | --- |
| untouched tree | **0** |
| same type error planted in telemetry/spans.rs | **101**, naming the file, `mismatched types` |

Direct artifact evidence that the leg reaches the feature — from this run's own
`leg-7-clippy-all-features.log`:

    "features":["cooperative","default","embedded","encode","fs","jit","json",
                "net","readiness","std","telemetry","test-support","threads"]

`ci.yml` needed no edit: it reads its legs out of `gates.json`.

## ⛔ What this does NOT claim

**Three named feature combinations out of 2^14 is not "every feature
combination is gated".** Every cfg has a complement — under `--all-features` all
`#[cfg(not(feature = "…"))]` code stops being compiled, and
`not(feature = "threads")` is how the cooperative runtime is selected. So the
new leg **does not replace its sibling** and must not be collapsed into it:

1. default ∪ cooperative ∪ json ∪ encode — sibling clippy/tests legs; the shape
   host consumers build.
2. cooperative + json, `default-features = false` — the wasm32-check leg, which
   is **the only leg that type-checks the not-`threads` paths**.
3. all-on — this leg, **the only leg that compiles `telemetry`**.

## ⛔ Known-red, DECLARED not dropped

`cargo test --workspace --all-features` fails: **1838 passed, 4 failed**, all
four telemetry-gated `scheduler::tests`. **Not** the known parallel-run OTel
flake — identical under `--test-threads=1`. Probed, `execute_slice` returns
`Exited(Error, nil)` where the tests expect `Requeue`. Stale test setup vs live
scheduler change is **UNRESOLVED**; both assertion sites are themselves
telemetry-gated, so the suite carries no non-gated twin to discriminate against.
The all-features **tests** leg lands when it is green, and not before.

## Errata — mine, and this lane's own subject

1. ⚠️ **The union arm confounded itself.** Specimen in all four crates, matrix
   off one run, reported `gleam-types` only — which reads exactly like "clippy
   reaches one crate of four". It isn't: gleam-types is a leaf dependency,
   `-D warnings` made its lint a compile **error**, and its three dependents
   failed to **build** rather than to **lint**. ⭐ **A union arm is valid only
   across INDEPENDENT units**; on a compiler-driven leg, planting in a
   dependency silently masks every dependent, and the mask reads as the coverage
   hole being hunted.

2. ⚠️ **My revert had no rc check.** `git checkout -- "$f"` lost the
   `.git/index.lock` race, three reverts failed, specimens accumulated across
   arms — and the script went on printing a tidy matrix. ⭐ **An unchecked
   cleanup step is a checker that cannot fail**, committed by the instrument
   built to sweep exactly that. Fixed by making the revert lock-free
   (`git show HEAD:path`) **and** re-censusing the probe to zero between arms
   with an ABORT. **The verification is the fix, not the lock-free write** — a
   lock-free revert can still fail.

## Cleared, and NOT reported as a hole

The wasm-tests leg prints `no tests to run!` for `tests/generated_bootstrap.rs`
and `tests/profile_seal.rs`. Correct by design: both hold plain `#[test]` fns
and both **do** run on the host tests leg — confirmed at the log lines before it
was raised, not after.

## Carried invariant

`refusal.count` re-measured at **55**, unchanged from #95 and #97 — the encode
guard still declines the same population. That number falls to zero only when
#95's `Type`-chunk fix lands.
