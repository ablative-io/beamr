# beamr 0.18.2 — release battery receipt (RE-RUN, gate of record)

Pin: commit `4ea651f131ffa22a41bcaa7d2e070f706159a7d6`, tree
`7f80a3955c7b22a5091043b8247782615d3bbed6`, branch `artemis/release-0.18.2`.
Runner: `runner.sh` beside this file. Six canon legs read from the committed
`gates.json` at run time.

**This supersedes `gate-logs/0182-release/` as the release gate of record.**
That run was clean for the tree it measured; the tree changed because two
CHANGELOG claims did not survive measurement. See that receipt's appended
SUPERSEDED notice for what moved and why.

## Verdict: 6/6 rc 0. COMPLETE marker DERIVED (legs_declared=6, legs_scored=6).

fmt 0 · clippy `-D warnings` 0 · wasm32-check 0 · wasm-tests 0 · tests 0 ·
blocking-call-in-native-bif 0.

Tree check (amended three-artifact form, counted with `wc`, never `grep -c`):
**0 pre, 0 post.** `tree.dirty.raw` is retained non-empty (it carries the known
untracked `.claude/skills/`), so an empty filtered result cannot be confused
with a broken instrument.

## Axes — PRE-REGISTERED, then measured

`AXES-PREDICTION.txt` beside this file was written **before `cargo` was
invoked**. It predicted **73 / 2094 / 0 / 0**.

Measured: **result-lines 73 / passed 2094 / failed 0 / ignored 0.** Exact match.

Prior (superseded `b42156c`) run: 73 / 2093 / 0 / 0. Delta **+1 passed**, no
new binary.

### The name-set diff — the check the count is structurally blind to

⭐ **A rename moves the NAME SET and leaves the COUNT alone.** The prediction
file said so in advance and named the renamed wall as an expected non-event, so
a green 2094 could not be allowed to stand as evidence on its own. Diffed
against the superseded run's `leg-5-tests.log`:

```
REMOVED (1)
  - scheduler::…::frame_resolution_tests::compiled_frame_reports_no_line_rather_than_the_modules_first_line
ADDED (2)
  + loader::load::tests::no_real_module_carries_a_line_marker_at_ip_zero
  + scheduler::…::frame_resolution_tests::compiled_frame_reports_no_line_from_its_placeholder_ip
```

Net +1, and **every one of the three movements is accounted for by name**: one
rename (out/in) plus one genuinely new gate. Nothing else in the suite moved.

**Denominator reconciliation between the two instruments, stated rather than
glossed.** `passed` = 2094 while the name census yields 2091 unique names — a
constant gap of **3 in both runs**. The three are doc-tests, whose result lines
do not match a `test <name> ... ok` shape:

```
crates/beamr/src/scheduler/mod.rs - scheduler::Scheduler::with_services (line 1115) - compile
crates/beamr/src/process/mod.rs - process::Process (line 145) - compile fail
crates/beamr/src/process/mod.rs - process::Process (line 136) - compile fail
```

The gap is identical on both sides, so it cancels in the delta. It is recorded
because an unexplained 3 between two instruments is exactly the residue that
should not be waved through.

## The forward-claim sweep — run two-armed, not as a bare zero

⚠️ **An empty grep proves nothing until the same grep is shown to hit.** Both
arms run against the **committed git objects**, not the working tree:

| arm | command | result |
| --- | --- | --- |
| **positive control** | `git show b3596d1:CHANGELOG.md \| grep -Ei '<pattern>'` | **1 hit** — line 8, `"lines are in preparation and this advisory will name them when they exist"` |
| **subject** | `git show 4ea651f:CHANGELOG.md \| grep -Ei '<pattern>'` | **0 hits** |

Pattern: `in preparation|will name|is planned|forthcoming|when they exist`.

The control fires on the known-bad predecessor and goes silent on the subject,
so the zero is a measurement rather than a broken instrument.

## What changed between the two runs

**One production-source change: none.** The delta is `CHANGELOG.md` plus three
test-only sites plus one added test.

1. **`CHANGELOG.md`** — F2's description corrected (it claimed an observable
   effect it does not have), and the advisory's only forward-looking clause
   removed rather than softened.
2. **`scheduler/execution/core.rs`** — fixture rebuilt from `line_table =
   vec![(0, 0)]` to `vec![(1, 0)]`, helper renamed, wall renamed, dependent
   position moved ip 0 → 2.
3. **`interpreter/opcodes/exceptions.rs`** — fixture rebuilt from
   `vec![(0, 0), (10, 1)]` to `vec![(1, 0), (10, 1)]`, position moved ip 0 → 2.
4. **`loader/load.rs`** — new constructibility gate (below).
5. **`gate-logs/89-jit-frame-identity/RECEIPT.md`** — appended correction
   re-declaring one pre-registered mutation arm as NON-DISCRIMINATING.

### The constructibility gate

`loader::load::tests::no_real_module_carries_a_line_marker_at_ip_zero` sweeps
every `.beam` in `test-workflows/sample` and asserts, per module, that the
first instruction is a `label` — the invariant, not merely its symptom. It
carries a **≥20-module non-empty-corpus assertion**, so a shrunken or unreadable
corpus fails loudly instead of passing vacuously.

⚠️ **It gates an assumption, not a contract.** `validate_module`'s four
`validate*` functions constrain nothing about ip 0, and the only references to
prologue ordering anywhere in the loader are this gate's own comments. **The
claim is therefore bounded — observed behaviour of the current compiler over
the measured set — and no `by construction` phrasing survives anywhere**, in
the changelog, the code comments or this receipt.

### The ip-0 fixture class, enumerated and classified

Swept as a class rather than patched as a row, because a per-row fix never
reaches rows that closed before the gate existed.

| site | was | disposition |
| --- | --- | --- |
| `scheduler/execution/core.rs` | `vec![(0, 0)]` | REWRITTEN |
| `interpreter/opcodes/exceptions.rs` | `vec![(0, 0), (10, 1)]` | REWRITTEN |

**Two members, both rewritten, ZERO retained as marked controls** — recorded as
a positive finding, because "no controls survive" is indistinguishable from
"nobody looked" unless it is written down. `module.rs`'s
`vec![(2, 0), (6, 1), (10, 99)]` is **not** a member: first entry at ip 2, a
shape the producer does emit.

## RELEASE_CHECKLIST coverage

The non-battery checks — publish dry-runs in checklist order, `cargo bench
--no-run`, `cargo doc`, `cargo metadata`, and the banned-macro audit stated as
a subset relation against `v0.18.1` — were run at `b42156c` and are in
`gate-logs/0182-release/release-checks/`. **They are not re-run here and remain
valid**: they exercise manifests, dependency metadata and non-test production
source, none of which changed between the two commits.

Tag `v0.18.2`: **NOT CREATED at the time of writing.** Publish: **NOT RUN.**

## What this release does NOT claim

* **F2 claims no impact.** It ships as a hardening: calling `line_at_ip` with a
  known placeholder is wrong regardless of consequence, and the guard makes the
  absent line structural rather than a property of table layout. Its observable
  effect in every measured module is nil.
* **F3 is disclosed OPEN and unfixed**, and its reachability is unproven — both
  obvious routes by which a recorded ip could reach the disagreement window
  land outside it.
* **AR-1 ships open and unnarrowed.** Ruled shippable by the build
  coordinators; this receipt records that state, it does not grant it.
* **Publish is not delivery.** No existing consumer resolves `0.18.2`: each
  `0.x` minor is a semver major, so `^0.16` and `^0.17` cannot reach it.
* **No production incident is explained by any of this.** F1 is confirmed
  rendering a real production frame off a shipped `0.17.0`, which explains a
  *diagnostic* — not the failure it was diagnosing. The originating incident,
  a `json:encode_integer/1` receiving a non-integer after 99 clean cycles,
  **remains unsolved.**
