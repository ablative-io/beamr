# beamr 0.18.2 — release battery receipt

Pin: commit `b42156c61b023ec0f37b0cf71b3d30bfe0309dc3`, tree
`cb1881eccb420f09290ee07bdb75cfa2a1d80da6`, branch `artemis/release-0.18.2`.
Runner: `runner.sh` beside this file. Six canon legs read from the committed
`gates.json` at run time.

This is the release gate of record for 0.18.2 under `RELEASE_CHECKLIST.md`.

## Verdict: 6/6 rc 0. COMPLETE marker DERIVED (legs_declared=6, legs_scored=6).

fmt 0 · clippy `-D warnings` 0 · wasm32-check 0 · wasm-tests 0 · tests 0 ·
blocking-call-in-native-bif 0.

Extras run beside the canon legs, all rc 0: `walls-jit-send`,
`walls-integration`, `extra-cooperative-json`, `interpreter`.

## Named axes

Base `2551841` (origin/main at 0.18.1): result-lines 72 / passed 2080 /
failed 0 / ignored 0.
This run: result-lines **73** / passed **2093** / failed 0 / ignored 0.

Reconciliation, exact and by name:

* **result-lines 72 → 73**: `+1` for the new `jit_send_delivery` integration
  gate binary added by #88. No other binary appeared.
* **passed 2080 → 2093**: `2080 + 8 + 5`.
  * `+8` from #88 (`jit_send_message` delivery): the named walls and arms
    listed in `gate-logs/88-jit-send-delivery/RECEIPT.md`.
  * `+5` from #89 (compiled-frame identity): the five named walls listed in
    `gate-logs/89-jit-frame-identity/RECEIPT.md`.

Tree check (amended three-artifact form: `tree.dirty.raw` / `.filtered` /
`.rc`, counted with `wc`, never `grep -c`): **0 pre, 0 post**.

Teardown: `du` target 19,083,012 KiB at completion. Not a cold tree — this
battery ran on the merge of two already-built lanes, so it is not a data point
for the cold/pure-move price class.

## RELEASE_CHECKLIST coverage

Evidence for the non-battery items is in `release-checks/` beside this file.

| Checklist item | Result | Artifact |
| --- | --- | --- |
| Crate versions match CHANGELOG top entry | beamr `0.18.2` = CHANGELOG top heading | `crates/beamr/Cargo.toml` |
| Path deps carry crates.io version fallbacks | `beamr -> gleam-types { path, version = "0.4.3" }`; `beamr-cli -> beamr { version = "0.18.0", path }` (caret admits 0.18.2) | manifests |
| License / description / repository / readme declared | present on `beamr` and `gleam-types` | manifests |
| `cargo metadata --no-deps` free of publish blockers | rc 0, `git+https://` dep count **0** | `release-checks/metadata.json` |
| `cargo publish --dry-run -p gleam-types` | **rc 0** | `release-checks/pub-gleam-types.{log,rc}` |
| `cargo publish --dry-run -p beamr` (after gleam-types) | **rc 0** | `release-checks/pub-beamr.{log,rc}` |
| `gates.json` battery at the release commit | **6/6 rc 0** | this directory |
| `cargo bench --package beamr --no-run` | rc 0 | `release-checks/bench.{log,rc}` |
| `cargo doc --package beamr --no-deps` | rc 0 (6 pre-existing doc warnings, not errors) | `release-checks/doc.{log,rc}` |
| Banned-macro audit | **no new hits** — see below | `release-checks/banned-raw.txt` |
| Open memory-safety findings ruling | **AR-1 remains OPEN** — see below | `docs/REVIEW-23-07.md` |
| Tag `v0.18.2` | **NOT CREATED** — gated on project lead | — |
| Publish | **NOT RUN** — gated on project lead | — |

### Banned-macro audit — measured as a diff, not as a count

Five hits at this tree: `interpreter/mod.rs:274` and `capability/audit.rs:55`
(`eprintln!`), and three `unimplemented!(` in `native/otp_stubs/tests.rs`
(test-only). All five are present **byte-for-byte at `v0.18.1`** — the shipped
predecessor — so this release introduces none. The v0.18.1 census over the same
pattern returns thirteen hits across a wider path set (it includes `beamr-cli`
and integration tests); the five above are a strict subset of it. A bare count
would not have shown that, which is why the check is stated as a subset
relation against a named predecessor rather than as a number.

### Open memory-safety finding

**AR-1 ships OPEN and unnarrowed in 0.18.2.** It was disclosed OPEN and SIZED
(17 sites) in the 0.18.1 advisory and is not touched by this release; the
0.18.2 CHANGELOG entry says so explicitly. The 0.18.1 precedent for shipping it
under a recorded project-lead ruling stands. **No new ruling is claimed here** —
this receipt records the state, it does not grant the clearance.

## What ships in 0.18.2

Two JIT defects, both silent-wrong-answer class:

* **#88** — `jit_send_message` implemented only the self-send arm. Every send
  from compiled code to any other destination was dropped while the helper
  returned the success value, so the sender could not observe the loss. Present
  byte-identically in **40 of 58 published versions** (0.4.0 – 0.18.1). The
  self-send arm additionally skipped the logical-clock tick and the
  replay-driver check, diverging replay determinism.
* **#89** — compiled stack frames took their module from the pinned
  `Arc<Module>` rather than from the frame's own recorded mfa, splicing one
  module's name onto another module's function; and the crash-report renderer
  resolved a line from the hardcoded placeholder `ip = 0`, fabricating the
  module's first line-table entry as the frame's line.

The whole-range advisory for #88, including the packaged-bytes census it rests
on, is at the top of `CHANGELOG.md`; the census itself is reproducible from
`docs/design/beamr/briefs/evidence/jit-send-drop/`.

## What this release does NOT claim

* **F3 is disclosed OPEN and unfixed.** `function_table` records `FuncInfo`
  ips while `line_table` records `Line` ips, and the real prologue order is
  `Label → Line → FuncInfo`; there is therefore exactly one ip per function at
  which the two tables disagree. See `gate-logs/89-jit-frame-identity/RECEIPT.md`.
* **No production incident is attributed to any of these defects.** One
  proposed attribution of #89's F2 to an external crash report was withdrawn
  when its pre-registered prediction was measured and failed.
* Backports are **not** covered here. `v0.17.0` can take #88's fix as a clean
  port; `v0.16.3` cannot — `JitRuntimeContext` at that base has no `services`
  pointer, so the mechanism the fix reaches through does not exist and 0.16.4
  would be a redesign, not a port.

---

## ⛔ SUPERSEDED 2026-08-12 — this is NOT the release gate of record

**Do not cite this battery for 0.18.2.** It ran at `b42156c`, a commit whose
`CHANGELOG.md` contained two claims that did not survive measurement:

1. **F2 was described as fabricating "the module's first line-table entry".**
   It does not — `line_at_ip(0)` is `None` in every module measured, so F2 had
   no observable effect. See `gate-logs/89-jit-frame-identity/RECEIPT.md`'s
   appended correction.
2. **The advisory carried a forward-looking clause** ("backports … are in
   preparation"). A claim whose truth value is scheduled to change must not be
   minted into an immutable artifact, and `CHANGELOG.md` ships inside the
   package.

The run itself was clean and its artifacts are kept as-is — the legs, the tree
check and the axes all stand for the tree they measured. **It is superseded on
the tree, not on the measurement.**

**Release gate of record: `gate-logs/0182-release-rerun/`, at `4ea651f`.**

This directory's `release-checks/` (publish dry-runs, bench, doc, metadata,
banned-macro audit) is **not** re-run there and remains valid: those checks
exercise manifests, dependency metadata and non-test production source, none of
which changed between `b42156c` and `4ea651f`. The delta is `CHANGELOG.md`
plus three test-only sites plus one added test.
