# beamr 0.20.0 — release battery receipt

**Cut by:** Artemis Peach (owning seat), 2026-08-22
**Authority:** standing release/publish delegation, `ablative/docs`
`STANDING-CONSTRAINTS.md` at `origin/main 5e52f30e` — "Applied 2026-08-22 …
beamr → Artemis Peach cuts", under Tom's channel message `d0ffb49c`
(07:43:23Z). Approval half delegated; the execution quality gate below is
unchanged and is the owning seat's.

**Versions:** beamr 0.20.0 · beamr-cli 0.7.0 · beamr-wasm 0.10.0 ·
gleam-types 0.4.4 (untouched).

**The version is forced, not chosen.** The accessor-lifetimes landing changed
the signatures of five **public** accessors — `Binary::as_bytes`,
`BinaryRef::as_bytes`, `ProcBin::as_bytes`, `SubBinary::as_bytes`,
`BigInt::limbs`. Under this repo's stated rule ("each `0.x` minor is a semver
major") that is a minor bump. A 0.19.5 would ship a breaking change where
`^0.19` consumers auto-resolve it and fail to compile. `beamr-cli` and
`beamr-wasm` pin `beamr` by version and cannot resolve 0.20.0 from a `0.19`
requirement, so all three move together (precedent `c991622`).

---

## ⛔ WHAT THIS BATTERY CANNOT SEE — read this before quoting any green

This battery ran on the **publishing venue: `aarch64-apple-darwin`**, rustc
1.97.1.

`crates/beamr/src/io/uring.rs:3` is `#![cfg(target_os = "linux")]`. **beamr#32's
entire 15-diagnostic site does not compile on this machine.** clippy going green
here is therefore **not** evidence that #32 is fixed, and #33's distribution
test passes here while failing on Linux.

**The status of #32 and #33 is whatever the last LINUX measurement said. This
run did not refresh it and must not be read as having done so.** An all-clear
from an instrument that never looked is not an all-clear.

This paragraph exists because the first run of this battery was graded against a
table inherited from a Linux venue: five of nine legs "deviated", every one
toward green, and none of it was the tree. The prediction was wrong, not the
code. A pre-registered table carries the conditions of its own venue.

---

## The battery — 9/9, tree `1b22d9a2af099f0e45ff45e7c572bc93a207daf6`

Commit `6d37072022927c87f7e35696c65a6ab5cef0d84f`. Legs run **verbatim** from
`gates.json`; no flags added to the gate of record.

| leg | rc | secs |
|---|---|---|
| fmt | 0 | 1 |
| clippy | 0 | 19 |
| wasm32-check | 0 | 2 |
| wasm-tests | 0 | 9 |
| tests | 0 | 155 |
| blocking-call-in-native-bif | 0 | 1 |
| clippy-all-features | 0 | 10 |
| tests-all-features | 0 | 152 |
| nostd-ratchet | 0 | 4 |

Graded against `PREDICTION.txt`, written **before** the run: **0 deviations**,
and the tally falsifier (`must read 1051`) held exactly.

Supporting gates: `cargo bench --package beamr --no-run` rc=0 ·
`cargo doc --package beamr --no-deps` rc=0 · release-obligations gate **PASS**
(with its own three fixtures grading correctly) · metadata complete on all four
crates · 0 `git+` dependencies · pins at `^0.20.0`.

⚠️ Width caveat, stated rather than discovered later: the `tests` legs run
without `--no-fail-fast`, because that is the gate of record's own command. On
this venue nothing fails, so nothing is truncated — but the leg would truncate
if anything did.

---

## The regression this cut caught, in the landing it was releasing

`nostd-ratchet` refused at tally **1073** against ceiling **1072**. Bisected
with the base as its own positive control:

    c55ac360  pre-landing    1072  PASS (exactly at ceiling)
    7401a531  current main   1073  FAIL
    5226d730  release prep   1073  FAIL  (release edits move it not at all)

**The accessor-lifetimes landing added it.** Three errors in, two out, net +1 —
same class as the 1072 already waived; the landing shuffled which files trip
first. Repaired with three imports the no_std build always wanted
(`core::cmp::Ordering`; cfg-gated `alloc::string::String` and
`alloc::vec::Vec`), which cleared **22** errors rather than one. That number was
measured after the edit and never predicted into it. Ceiling tightened
**1072 → 1051**. Full detail in `nostd-bisect.txt`.

⚠️ **Why it went unseen.** That leg reds *structurally* on the Linux venue
(#31), so the landing's own differential recorded rc=3 on both arms and counted
it as one of nine matching legs. **Two dead instruments agreeing is not a match;
it is the absence of a measurement, twice.** A structurally-red gate does not
fail closed — it fails silent, exactly where the thing it guards is changing.

---

## Release obligations — both settled, neither bypassed

- **OB-001 — EXECUTED.** `EtsOrderedSet::new` deleted, carrying out the DELETE
  ruling recorded at the 0.19.3 cut. Zero in-tree callers, positive-controlled:
  bare `EtsOrderedSet` matched 12 in the same search, the constructor 0.
- **OB-002 — RULED KEEP** at this cut, superseding the 0.19.3 parking: a delete
  refused on its merits, not postponed. The 0.19.0 demotion under #104 already
  achieved the safety objective (both functions cfg'd behind `test-support`,
  which is not in defaults and not enabled by any published family crate). The
  delete would cost a 60-site non-mechanical test redesign and would remove
  `tests/spawn_process_scaffold_only.rs`, the #104 positive control. The ruling
  carries a **re-arming trigger** so it re-opens itself if raw-instruction-0
  entry ever becomes consumer-reachable again.

## Raw output

`rawlogs.tar.gz` — every leg's stdout and stderr, captured separately (JSON legs
put machine events on stdout and diagnostics on stderr; merging them destroys
both). Includes run 1, the run whose prediction was wrong, kept deliberately:
the corrected instrument is worth more than a tidy record.
