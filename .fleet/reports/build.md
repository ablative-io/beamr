# FLIGHT REPORT — leg `build`, BEAMR-R8-DEOPT (fleet run `beamr-r8-flight1`)

- **Brief**: `.fleet/BRIEF.md`, RATIFIED AS AMENDED by Artemis Peach, 2026-08-24.
- **Plan**: `.fleet/plan.md`; findings **F1–F6 binding**, not re-litigated below.
- **Leg orders**: `.fleet/legs/build.md`.
- **Base**: `272ca9e034b0f78f50e30ec1068ea1eca967e6d6`. Branch `fleet/beamr-r8-base`.
- **Control token**: `br8-ctl-5350d25c-waffles-20260824`.

**First-party fixture note.** beamr is our own product and this box is our own
fleet venue. Every estate, key, corpus document and repository named in this
report is a **first-party local test fixture**: the `ablative-io/aion` clone used
for corpus sourcing and citation grounding is our own PUBLIC repo, cloned
read-only into `/tmp`, and nothing from it is committed — only fixtures DERIVED
from it, inside the wall.

---

## 0. What landed, in one paragraph

`Badmatch`, `Badrecord`, `CaseEnd` and `IfEnd` are now `Coverage::Supported`,
lowered as DEOPT terminals with the `func_info` arm's body **verbatim**, accepted
by the pre-pass with `validate_read_operand` on the three that carry an operand,
and joined to `control_flow_successors`' no-in-slice-successor arm so the
deopt-after-side-effect guard sees them correctly. The 75-variant consistency
walk is green with the derived Supported count moved 60 → 64, still exhaustive,
still wildcard-free, not weakened. Six new test files carry R3, R3b, R4, R5 and
the findings. `is_no_fail_label` was NOT edited — F1's declaration is made at the
bytes in §2. Six further findings (F7–F12) were measured and are recorded in §10
rather than papered over.

**The gate is NOT green.** 8 of the 9 canon legs pass — including `tests` (2196
passed) and `tests-all-features` (2206 passed), where every fixture in this
report runs — and the 9th, `nostd-ratchet`, returns **CANNOT-MEASURE (rc=3)** for
a cause outside this leg's wall: a BSD `mktemp -t PREFIX` idiom in
`scripts/gate-nostd-ratchet.sh` that GNU coreutils refuses. Receipt and reasoning
in §1b; the finding, its proof, and its one-line fix in §10 F11. The ratchet's
underlying measurement is a **PASS** — 1051 errors, exactly at the ceiling —
measured by running the script's own logic with the template repaired in `/tmp`
so nothing in the tree is touched. Getting the other eight legs green
additionally required fixing two **pre-existing, venue-specific reds measured at
the pinned base** (clippy; and a thread name Linux truncates), declared as their
own diff category in §9.

---

## 1. Gate receipts

### 1a. At base — the expected RED, cause named

```
$ sh .fleet/gate-entry.sh
== 1. environment floors ==
  disk free: 407G
== 2. tree receipt ==
  HEAD: 0d071d93b53ebd5f646f8f066b609197fe01db8b
  TREE: CLEAN
== 3. paths census ==
GATE-RED: nothing under crates/ — deliverable absent (named cause: deliverable_absent)
```

Exactly the red `.fleet/legs/build.md` §0 predicted: at the rig tip the diff
against base is the `.fleet/` rig files only, so the census sees zero files under
`crates/`. **The rig arms are live.**

**Environment facts recorded.** `cargo`, `jq`, `ast-grep` all present.
`wasm-bindgen-test-runner` is **present** on this box — the gate's absence note
did not fire, so no userspace install was needed and no leg was skipped.
Toolchain is the pinned `1.97.1` (`rust-toolchain.toml`), matching
`rustc 1.97.1 (8bab26f4f 2026-07-14)` / `clippy 0.1.97`.

Baseline re-measured at the rig tip before any edit:

```
$ cargo test -p beamr --lib --features beamr/encode coverage_
test jit::compiler::compiler_tests::coverage_table_has_one_entry_per_instruction_variant ... ok
test jit::compiler::compiler_tests::coverage_walk_agrees_with_prepass_and_dispatch_for_all_75_variants ... ok
test result: ok. 4 passed; 0 failed
```

Supported count = 60, as the plan measured.

### 1b. Final — **NOT GATE-GREEN. 8 of 9 canon legs green; the 9th cannot measure on this venue.**

Stated plainly rather than dressed up: **this leg did not reach GATE-GREEN, and
it cannot from inside its own wall.** Receipt at `aed8fde5`:

```
== 1. environment floors ==
  disk free: 400G
== 2. tree receipt ==
  HEAD: aed8fde53b4818fd71113ac1ed10dda323022bf1
  TREE: CLEAN
== 3. paths census ==
  census OK (30 files in wall, 24 under crates/)
== 4. suppression sweep ==
  clean
== 5. the canon (gates.json legs, run as ci.yml runs them) ==
  gates.json declares 9 legs
  LEG 1 (fmt) rc=0
  LEG 2 (clippy) rc=0
  LEG 3 (wasm32-check) rc=0
  LEG 4 (wasm-tests) rc=0
  LEG 5 (tests) rc=0
  LEG 6 (blocking-call-in-native-bif) rc=0
  LEG 7 (clippy-all-features) rc=0
  LEG 8 (tests-all-features) rc=0
  LEG 9 (nostd-ratchet) rc=3
== 6. verdict (scripts/ci-verdict.sh — the one copy of the truth) ==
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
```

**The rig arms are all green**: tree CLEAN, census in the wall (30 files, 24 under
`crates/`), suppression sweep **clean** (R6, mechanically). **Eight canon legs are
green**, including `tests` (2196 passed) and `tests-all-features` (2206 passed),
which is where every fixture in this report runs.

**Leg 9, `nostd-ratchet`, returns rc=3 — CANNOT MEASURE, not FAIL-on-the-merits.**
Its cause is named in full in **finding F11 (§10)**: a BSD `mktemp -t PREFIX`
idiom in `scripts/gate-nostd-ratchet.sh` that GNU coreutils refuses, so the
script's log file is never created and its own REFUSE arm fires — correctly.
`scripts/` is untouched by this leg (`git diff 272ca9e...HEAD -- scripts/
gates.json` is empty) and the failure is deterministic on GNU coreutils, so it is
structural at the pinned base; the base run never reached it only because it
stopped earlier at `deliverable_absent`. The script's own ceiling comment already
records this venue behaviour as issue **#31**.

**The ratchet itself is a PASS, measured** — running the script's own logic with
only the template repaired, from a copy in `/tmp` so nothing in the tree is
touched: `rustc tally=1051, ceiling=1051 → PASS: exactly at the ceiling. Debt
held.` This brief neither added nor removed no-std debt.

**Why the gate was not made green.** The one-line repair lives outside this
leg's wall, which `gate-entry.sh` §3 enforces mechanically: editing `scripts/`
reds the gate with `wall_breach`. The only two reachable outcomes on this venue
are `canon_red` (leave it) or `wall_breach` (fix it). The orders forbid editing
the gate and forbid changing anything outside the wall, and the leg orders' own
rule for exactly this case is *"Any other red at base is a rig finding — report
it, do not route around it, do not edit the gate."* That is what was done. **The
call on whether to widen the wall for a one-character-class instrument repair is
the seat's, not the builder's.**

---

## 2. R1 — Classification, all four tables

### 2a. `coverage` (`crates/beamr/src/jit/coverage.rs`) — the four move to `Supported`

The `RejectedIncremental { reason: "wave 2: error-raising terminals" }` arm is
**deleted entirely**, not left empty. The four join the `Supported` arm in enum
order, immediately after `FuncInfo`, with a comment in the register of the
`func_info` comment above it.

```diff
@@ pub fn coverage(instruction: &Instruction) -> Coverage {
-        | Instruction::FuncInfo { .. } => Coverage::Supported,
+        | Instruction::FuncInfo { .. }
+        // -- Supported: BEAMR-R8-DEOPT — the four error-raising terminals, under
+        // -- the func_info treatment exactly. Each is reached ONLY via a fail
+        // -- edge (a select_val fall-through trap or an assertion fail edge),
+        // -- lowered as a DEOPT terminal, and the restarted interpreter raises
+        // -- the error: {badmatch,V} / {case_clause,V} / the bare atom if_clause.
+        // -- No state crosses the deopt — the restart recomputes every operand
+        // -- from the function's bytecode entry. A cold error trap no longer
+        // -- rejects its whole containing function from JIT/AOT coverage.
+        | Instruction::Badmatch { .. }
+        | Instruction::Badrecord { .. }
+        | Instruction::CaseEnd { .. }
+        | Instruction::IfEnd => Coverage::Supported,
@@
-        Instruction::Badmatch { .. }
-        | Instruction::Badrecord { .. }
-        | Instruction::CaseEnd { .. }
-        | Instruction::IfEnd => Coverage::RejectedIncremental {
-            reason: "wave 2: error-raising terminals",
-        },
```

`SelectTupleArity`, the exception machinery (`Catch`/`CatchEnd`/`TryCaseEnd`/
`Raise`/`RawRaise`/`BuildStacktrace`) and `UpdateRecord` are **untouched**. That
is the Wall, and §6 asserts it mechanically.

### 2b. `is_observable_side_effect` — classified `false`, ZERO DIFF, declared

The four stay on the `false` side. **This table takes zero diff, and that is the
correct outcome, not an omission** — the `false` arm is one flat `|`-chain with
no per-group comments inside it, so there is no stale grouping to escape.

The classification, stated so R1's consistency claim is discharged explicitly
rather than by silence: **an error-raising terminal performs no observable effect
that a deopt-restart would duplicate.** The terminal itself commits nothing; the
raise happens exactly once, in the interpreter, AFTER the restart. A restart that
re-reaches the terminal re-raises the same error rather than doubling an effect.
Contrast `Send` (message duplication) and `RemoveMessage` (message loss), which
are the effects this table exists to name.

### 2c. `is_runtime_deopt_capable` — the four become `true`

```diff
         // LEG 1c A2: func_info is lowered as an unconditional DEOPT terminal.
-        | Instruction::FuncInfo { .. } => true,
+        | Instruction::FuncInfo { .. }
+        // BEAMR-R8-DEOPT: the four error-raising terminals are lowered as
+        // unconditional DEOPT terminals on the same seam (dispatch_core.rs), so
+        // they are runtime-deopt-capable and the deopt-after-side-effect guard
+        // must see them.
+        | Instruction::Badmatch { .. }
+        | Instruction::Badrecord { .. }
+        | Instruction::CaseEnd { .. }
+        | Instruction::IfEnd => true,
```

and they leave the `false` chain, out from under the now-stale
`-- non-Supported variants: never lowered (pre-pass rejects first) --` comment:

```diff
         | Instruction::BuildStacktrace
-        | Instruction::Badmatch { .. }
-        | Instruction::Badrecord { .. }
-        | Instruction::CaseEnd { .. }
-        | Instruction::IfEnd
         | Instruction::UpdateRecord { .. }
```

The doc comment was corrected in both halves as the leg orders require: the
`true` set now names the four with their deopt edge, and the sentence that ended
"and every non-`Supported` variant (never lowered …)" now reads "the REMAINING
non-`Supported` variants … Since BEAMR-R8-DEOPT that remainder no longer includes
`Badmatch`/`Badrecord`/`CaseEnd`/`IfEnd`: they are `Supported` and their `true`
classification above DOES gate a live decision."

### 2d. `is_no_fail_label` — F1's DECLARATION, no edit

**No edit was made, and this is R1's fourth table discharged by declaration, not
skipped.** Checked at the bytes:

> `crates/beamr/src/jit/coverage.rs` defines
> `pub(crate) fn is_no_fail_label(operand: &crate::loader::decode::Operand) -> bool`
> as a one-line `matches!(operand, Operand::Label(0))`. It takes an **`&Operand`**,
> has no match over `Instruction`, and therefore has **no exhaustiveness property
> at all** — adding an `Instruction` variant cannot break it. Its four call sites
> (`ir_control.rs:227`, `:409`, `:537`; `dispatch_core.rs:195`) all pass a `Bif`'s
> `parsed.fail`; it answers "is this Bif's fail label the `{f,0}` sentinel", never
> "what class is this instruction".
>
> The semantic R1 gestures at is nonetheless TRUE and checkable at the bytes:
> `Badmatch`/`Badrecord`/`CaseEnd` each carry exactly one `value: Operand` and
> `IfEnd` carries nothing (`loader/decode/instruction.rs:243-252`). **None of the
> four carries a fail label of its own, so none of them ever reaches
> `is_no_fail_label`.**
>
> The module header at `coverage.rs:3-9` OVER-CLAIMS by grouping this `&Operand`
> predicate with the three exhaustive `Instruction` tables and asserting all four
> are "EXHAUSTIVE WITH NO WILDCARD ARM". The brief inherited that over-claim.
> R1's "all four must classify the four terminals" is **not literally
> dischargeable**. Recorded, not worked around.

**Recommendation, not acted on (the call is Artemis's).** The header at
`coverage.rs:3-9` should say "three classification tables of record … plus the
`{f,0}` fail-label sentinel predicate", so the tree's own standards claim matches
the tree. It was NOT edited on this leg's initiative, per the leg orders.

### 2e. The 75-variant consistency walk — green, 60 → 64, derived

```diff
     // LEG 1c A2 adds FuncInfo (the function_clause DEOPT terminal) → 60.
+    // BEAMR-R8-DEOPT adds the four error-raising terminals — Badmatch, Badrecord,
+    // CaseEnd, IfEnd — each lowered as a DEOPT terminal under the func_info
+    // treatment (reached via a fail edge only; the restarted interpreter raises)
+    // → 64. The count stays derived from this walk; it is not a second literal.
     assert_eq!(
-        supported, 60,
+        supported, 64,
         "Supported count derived from the coverage table"
     );
```

```
$ cargo test -p beamr --lib --features beamr/encode coverage_
test jit::compiler::compiler_tests::coverage_table_has_one_entry_per_instruction_variant ... ok
test jit::compiler::compiler_tests::coverage_walk_agrees_with_prepass_and_dispatch_for_all_75_variants ... ok
test result: ok. 4 passed; 0 failed; 0 ignored
```

**Exhaustive-with-no-wildcard is unchanged**: no wildcard arm was added to any
table, the representative table at `compiler_tests.rs:3538` was not touched, and
the walk itself was not weakened — only the derived count moved, because the
table moved. The count remains DERIVED from the walk, not a second literal.

---

## 3. R2 — Lowering, and the pre-pass arms

### 3a. Lowering — the template, verbatim, no state marshalled

`crates/beamr/src/jit/compiler/dispatch_core.rs`, placed **immediately beside**
the `func_info` arm so the precedent is legible in the diff:

```rust
Instruction::Badmatch { .. }
| Instruction::Badrecord { .. }
| Instruction::CaseEnd { .. }
| Instruction::IfEnd => {
    return_status_raw(builder, JIT_STATUS_DEOPT, JIT_DEOPT_SENTINEL);
    Ok(Some(true))
}
```

That is the `func_info` body character for character. **No operand is
marshalled** — F5, and the arm's own comment says why: `JIT_STATUS_DEOPT` makes
`call_native` return `Ok(None)`, the caller re-enters the callee at its bytecode
entry, and the interpreter recomputes every operand itself. R2's "carrying enough
state" is satisfied by the RESTART contract, not by transfer. **No divergence
from the template to report.**

### 3b. Pre-pass acceptance + operand validation — accepted, not accepted blind

`crates/beamr/src/jit/ir_control.rs`, placed with the `FuncInfo` arm, block-start
insertion following it exactly (`block_starts.insert(index + 1);`):

```rust
Instruction::Badmatch { value }
| Instruction::Badrecord { value }
| Instruction::CaseEnd { value } => {
    validate_read_operand(value)?;
    block_starts.insert(index + 1);
}
Instruction::IfEnd => {
    block_starts.insert(index + 1);
}
```

`validate_read_operand` is the existing validator used by `WaitTimeout`'s
`timeout`, `RecvMarkerBind` and `RecvMarkerClear`/`RecvMarkerUse` — **mirrored,
not reinvented**. `IfEnd` is fieldless (`instruction.rs:252`): block-start only.

### 3c. Successors — the F6 corollary, not missed

The four joined the "leaves the function: no in-slice successor" arm beside
`FuncInfo` and the recv-markers, and the arm's comment names them:

```diff
-        | Instruction::RecvMarkerUse { .. } => {}
+        | Instruction::RecvMarkerUse { .. }
+        | Instruction::Badmatch { .. }
+        | Instruction::Badrecord { .. }
+        | Instruction::CaseEnd { .. }
+        | Instruction::IfEnd => {}
```

Without this the union dataflow in `reject_deopt_after_side_effect` would
propagate taint through a fall-through edge that does not exist and the guard
would mis-fire — a silent correctness bug the 75-walk cannot catch. §6's F6
census exercises the guard on all four and shows it firing for the right reason.

### 3d. The `debug_assert_ne!` agrees

`ir_control.rs`'s `other =>` arm carries
`debug_assert_ne!(coverage(other), Coverage::Supported, …)`. The walk runs in a
**debug build** (the gate's `tests` leg), and
`coverage_walk_agrees_with_prepass_and_dispatch_for_all_75_variants` drives every
one of the 75 variants through `compile`. It is green, so no `Supported` variant
reaches `other =>`.

---

## 4. R3 — Error IDENTITY, all four terminals

**Acceptance is IDENTITY.** Every fixture compares **class + reason term +
stacktrace**, byte for byte, rendered through the repo's own
`beamr::term::format::format_term`. None of the assertions below would pass
against a different reason term.

**Taken-ness is asserted, not inferred**, three mechanical ways in every fixture:

1. the JIT arm asserts the SPECIMEN is in the `JitCache` at the moment of the
   error drive. beamr's pre-pass admission is **whole-function** — the walk errs
   on the first non-`Supported` variant with no compile-around — so a cache entry
   for a function whose only cold edge is the terminal **is** the proof the
   terminal was lowered rather than rejecting its container;
2. the never-JIT'd arm asserts the cache is EMPTY for that same key;
3. the happy input returns its value through that same cached native body **in
   the same run** (the R3b null arm), so the body is demonstrably entered.

Both arms run **in one test binary**. The never-JIT'd arm uses
`set_jit_enabled(false)` applied immediately after construction and before the
first spawn — the operator switch's own contract, which additionally guarantees
already-compiled code is never entered; a large `jit_threshold` cannot promise
that second half.

Files: `crates/beamr/tests/jit_error_terminal_identity.rs` (CaseEnd, IfEnd,
Badrecord) and `crates/beamr/tests/jit_badmatch_restart_fidelity.rs` (Badmatch).

### 4a. CaseEnd — `{case_clause, V}`

```
---- BEAMR-R8-DEOPT R3 transcript: CaseEnd — reason {case_clause, V} ----
  specimen cached native? native-arm=true never-JIT'd-arm=false
  R3b null arm  (happy path, SAME RUN as the deopt arm)
    native-with-deopt : exit=Normal value=3 host_error=None
    never-JIT'd       : exit=Normal value=3 host_error=None
  R3 deopt arm  (error edge TAKEN)
    native-with-deopt : exit=Normal value={caught, error, {case_clause, no_such_verdict},
        [{awl_terminals, verdict_code, 1, [{line, 73}]},
         {awl_terminals, probe_verdict, 1, [{line, 130}]},
         {awl_terminals, probe_verdict, 1, [{line, 124}]}]} host_error=None
    never-JIT'd       : exit=Normal value={caught, error, {case_clause, no_such_verdict},
        [{awl_terminals, verdict_code, 1, [{line, 73}]},
         {awl_terminals, probe_verdict, 1, [{line, 130}]},
         {awl_terminals, probe_verdict, 1, [{line, 124}]}]} host_error=None
```

Class `error`, reason `{case_clause, no_such_verdict}`, and a three-frame
stacktrace **including line numbers** — identical across the boundary.

### 4b. IfEnd — the BARE ATOM `if_clause`

```
---- BEAMR-R8-DEOPT R3 transcript: IfEnd — reason is the BARE ATOM if_clause ----
  specimen cached native? native-arm=true never-JIT'd-arm=false
  R3b null arm  (happy path, SAME RUN as the deopt arm)
    native-with-deopt : exit=Normal value=present host_error=None
    never-JIT'd       : exit=Normal value=present host_error=None
  R3 deopt arm  (error edge TAKEN)
    native-with-deopt : exit=Normal value={caught, error, if_clause,
        [{awl_terminals, report_presence, 1, [{line, 85}]},
         {awl_terminals, probe_presence, 1, [{line, 149}]},
         {awl_terminals, probe_presence, 1, [{line, 138}]}]} host_error=None
    never-JIT'd       : exit=Normal value={caught, error, if_clause,
        [{awl_terminals, report_presence, 1, [{line, 85}]},
         {awl_terminals, probe_presence, 1, [{line, 149}]},
         {awl_terminals, probe_presence, 1, [{line, 138}]}]} host_error=None
```

The fixture asserts the reason **is** `if_clause` and separately asserts it is
**not** tuple-wrapped (`!caught.contains("{if_clause")`) — the prior regression
carried in `interpreter/opcodes/exceptions.rs`'s own comment, where a
`{if_clause, []}` reason failed to match `catch error:if_clause` in loaded
bytecode.

### 4c. Badmatch — the DISCRIMINATING fixture, and why it is about RESTART FIDELITY

**The naive reading of R3's "trap on the SUBJECT operand" is a trap.** Checked at
the bytes in our own public repo `ablative-io/aion`, both `Badmatch` construction
sites emit the identical operand:

```rust
// crates/aion-awl/src/mir/select/emit/burst.rs:293-295
self.push(Instruction::Badmatch { value: Operand::X(0) });
// crates/aion-awl/src/mir/select/emit/burst.rs:393-395
self.push(Instruction::Badmatch { value: Operand::X(0) });
```

**No lowering can report the walked tail by mis-reading the operand — the operand
is `X(0)` either way, and recording "the operand was X(0)" proves nothing.**

The distinction lives one instruction earlier. `assert_list`
(`burst.rs:344-396`) walks the list **through X0 itself** —
`Move { source: X(2), destination: X(0) }` on every bind (`burst.rs:370-373`) —
so at the fail label X0 holds the WALKED TAIL. That is precisely why the BC-2b-5
carried fix re-points X0 from the subject's `Y` home before trapping:

```rust
// burst.rs:385-395
self.push(Instruction::Label { label: fail });
// Trap on the SUBJECT list, not the walked tail X0 happens to hold —
// `let assert [...] = subject` reports the whole subject (BC-2b-5
// carried fix; the walked-tail operand misattributed the mismatch).
self.push(Instruction::Move { source: Operand::Y(self.home(list)?),
                              destination: Operand::X(0) });
self.push(Instruction::Badmatch { value: Operand::X(0) });
```

So what the fixture guards is **restart fidelity**: that the deopt restarts the
function from its BYTECODE ENTRY, so the restarted interpreter re-executes that
`Move` itself — rather than resuming mid-function or marshalling native register
state across the deopt, either of which would leave the walked tail in X0 and
reproduce the exact BC-2b-5 misattribution.

The specimen is AWL's `assert_list` shape reproduced instruction for
instruction — home the subject, load it into X0, unroll the binds each
**clobbering X0 with the walked tail**, terminate on a shape test, and on the
fail edge re-point X0 at the subject before `Badmatch { value: X(0) }`.

```
---- BEAMR-R8-DEOPT R3 Badmatch RESTART-FIDELITY transcript ----
  subject (must be reported)        : {fmt_target, {clippy_workspace_target, {test_target, embed_target}}}
  walked tail at failure (must not) : {test_target, embed_target}
  specimen cached native? native-arm=true never-JIT'd-arm=false
  R3b null arm (happy walk, SAME RUN)
    native-with-deopt : exit=Normal value=fmt_target exception=None host_error=None
    never-JIT'd       : exit=Normal value=fmt_target exception=None host_error=None
  R3 deopt arm (fail edge TAKEN)
    native-with-deopt : exit=Error exception=Some("error: {badmatch,
        {fmt_target, {clippy_workspace_target, {test_target, embed_target}}}}") host_error=None
    never-JIT'd       : exit=Error exception=Some("error: {badmatch,
        {fmt_target, {clippy_workspace_target, {test_target, embed_target}}}}") host_error=None
```

**Subject and walked tail are distinguishable terms** — the tail at the point of
failure is a proper sub-term, and the fixture asserts `subject != tail` before it
asserts anything else, so the fixture cannot be vacuous. It then asserts BOTH
`contains("{badmatch, <subject>}")` AND `!contains("{badmatch, <tail>}")`, with
both terms built from the atoms the fixture itself constructs, never typed into
the test as strings.

**Two transpositions from AWL's literal shape, both declared:**

1. **The walk.** AWL walks with `TypeTestOp::IsNonemptyList` and terminates on
   `IsNil`. **This JIT tier lowers neither** (`jit/ir_control_validation.rs`
   admits only IsInteger / IsAtom / IsPid / IsBinary / IsList / IsTuple), so the
   literal list burst is still rejected after R8 — see finding in §6 and §10.
   The specimen transposes the list walk to a 2-tuple chain, preserving the
   clobber-then-restore shape that is the whole point.
2. **The home.** AWL homes the subject in a `Y` slot; the specimen homes it in
   `X3`. Reason: **F7** (§10) — a deopt after an `Allocate` leaves the native
   frame on the stack and the restart pushes a second one, so the stacktrace
   would carry a duplicated frame and this fixture's stacktrace comparison would
   measure the deopt seam instead of the subject. F7 is measured separately, with
   a control proving it is the `func_info` template's own behaviour and not this
   brief's.

*Rendering note for the judge:* `CapturedFrame` names are resolved inside the
scheduler using the scheduler's own atom table, which for a hand-built module
does not know that module's atoms — so frame NAMES print as unrelated common
atoms in this one transcript. The comparison is unaffected (both arms resolve
through the same table) and the reason term, which is rendered with the module's
own table, is correct. The erlc-loaded fixtures in §4a/§4b/§4d have no such
artefact and show fully-resolved frames.

### 4d. Badrecord — **IDENTITY OF A NON-RAISE**

```
---- BEAMR-R8-DEOPT R3 transcript: Badrecord — IDENTITY OF A NON-RAISE (no interpreter arm exists) ----
  specimen cached native? native-arm=true never-JIT'd-arm=false
  R3b null arm  (happy path, SAME RUN as the deopt arm)
    native-with-deopt : exit=Normal value=true host_error=None
    never-JIT'd       : exit=Normal value=true host_error=None
  R3 deopt arm  (error edge TAKEN)
    native-with-deopt : exit=Error value=not_a_gate_result host_error=Some("unsupported opcode badrecord")
    never-JIT'd       : exit=Error value=not_a_gate_result host_error=Some("unsupported opcode badrecord")
```

**Stated in the words the leg orders require: this is IDENTITY OF A NON-RAISE.**
Per F2, `Instruction::Badrecord` has **no interpreter execution arm** —
`interpreter/opcodes/mod.rs` dispatches `Badmatch`, `CaseEnd` and `IfEnd` only;
`interpreter/opcodes/exceptions.rs` defines no `badrecord`; the variant falls to
the `other => Err(ExecError::UnsupportedOpcode { … })` catch-all. So the brief's
one-sentence contract — "deopt to the resident interpreter, which raises the
error" — **is false for this one of the four**.

Identity nonetheless holds, and holds for a real reason: before this change the
function was rejected from JIT, ran interpreted, and yielded `UnsupportedOpcode`;
after it, the function is JIT'd, deopts at the terminal, restarts interpreted,
and yields the **same** `UnsupportedOpcode`. Both arms agree byte for byte,
including the happy path. **This is NOT evidence that the terminal raises
correctly, and it is not presented as such.**

`Badrecord` was NOT implemented in the interpreter — out of scope, outside the
Walls. **Recommendation (not acted on):** a follow-up brief should add
`exceptions::badrecord` raising `error:{badrecord, V}` and dispatch it from
`interpreter/opcodes/mod.rs`, at which point this fixture becomes a real identity
fixture without any change to the lowering. Nothing in R8 depends on it: the
census says AWL's 27 emitted variants put only `Badmatch` and `CaseEnd` outside
coverage, so no corpus function depends on `Badrecord`.

---

## 5. R3b — the null arm, INSIDE the same run

Every transcript in §4 carries its null arm **in the same run as its deopt arm**,
measured in this order: heat on the happy path → assert the specimen is cached
native → drive the happy path once more (the null arm) → drive the error input
(the deopt arm). The happy results are byte-identical JIT'd vs interpreted in all
four cases: `3`, `present`, `true`, `fmt_target`. **The cure does not perturb the
path that never traps.**

### Specimen provenance — DERIVED from the census corpus, not synthetic minimums

Sources cloned read-only from our own PUBLIC first-party repo `ablative-io/aion`
(`git clone --filter=blob:none --no-checkout --depth 1`, checked out narrowly into
`/tmp/aion-src`). **Nothing from that repo is committed** — only fixtures derived
from it, inside the wall.

| Specimen | Source document + line | What was cut |
|---|---|---|
| `verdict_code/1` (CaseEnd) | `workflows/investigate/investigate.awl:148` — `type Verdict = Reproduced \| NotReproduced \| AlreadyFixed \| NeedsInfo \| WrongRepo` | prose docs; routed payloads kept as the small ints AWL routes on; **the `otherwise` arm** |
| `report_presence/1` (IfEnd) | `workflows/investigate/investigate.awl:273` — `outcome present: when probe_result.report is present, route file_it` | the `read_report` action plumbing; **the `otherwise` arm** |
| `gate_passed/1` (Badrecord) | `workflows/gates/gates.awl:65-88` — `type GateResult { gate, exit_code, passed, failing_targets, failure_detail, output_tail }` | the `failure_detail`/`output_tail` prose; the record shape kept whole |
| `first_failing_target/1` (Badmatch, erlc) | `workflows/gates/gates.awl:73` — `failing_targets: [String]` | String payloads become atoms |
| `assert_targets/1` (Badmatch, AWL burst shape) | `crates/aion-awl/src/mir/select/emit/burst.rs:344-396` applied to `gates.awl:73` | the list walk transposed to a tuple chain, and the Y home to X3 — both declared in §4c |
| `awl_replay:run/1` (R4) | `workflows/gates/gates.awl:135-158` — the seven-leg band sequence and `step manifest`'s routing select | actions, workspace plumbing, manifest derivation; **the `otherwise` arm** |

**The load-bearing cut, stated plainly.** In AWL the fall-through trap is emitted
**structurally and is cold by construction**: `emit/control.rs:133-134` pushes the
fail label and then `CaseEnd` for EVERY select, total arms or not, and
`emit/burst.rs` does the same for every `let assert`. That is exactly why these
traps poison 51 of 9,864 corpus functions while never firing in production. **To
WITNESS the taken edge — which is R3's acceptance — the specimen must drop the
total-coverage `otherwise` arm so the structurally-cold edge is actually TAKEN.**
That cut is disclosed here rather than buried, because it is the one place a
reader could mistake a specimen for a rewrite.

Fixture sources are committed with their `.beam` files, matching the existing
`crates/beamr/tests/fixtures/` convention (`.erl` + `.beam`, no `.S`).

---

## 6. R5 — pre-pass acceptance, PER FUNCTION WITH ITS REASON

`crates/beamr/tests/jit_error_terminal_prepass_acceptance.rs`. **Never a bare
count** — every refusal names itself, so a CORRECT rejection is separable from an
incomplete fix.

### 6a. The four terminals — ACCEPTED

```
---- BEAMR-R8-DEOPT R5: pre-pass acceptance, per specimen ----
  cold-edge specimen Badmatch   : ACCEPTED reason: -
  cold-edge specimen Badrecord  : ACCEPTED reason: -
  cold-edge specimen CaseEnd    : ACCEPTED reason: -
  cold-edge specimen IfEnd      : ACCEPTED reason: -
```

### 6b. Corpus-derived specimens — per function

```
---- BEAMR-R8-DEOPT R5: corpus-derived specimen census ----
  verdict_code           [CaseEnd    ] : ACCEPTED reason: -
  report_presence        [IfEnd      ] : ACCEPTED reason: -
  gate_passed            [Badrecord  ] : ACCEPTED reason: -
  first_failing_target   [Badmatch via is_nonempty_list/is_nil] : REJECTED
      reason: unsupported JIT opcode: TypeTest(IsNonemptyList)
```

The one rejection is asserted to be **about the type test and NOT about the
terminal** — the test fails if the reason mentions `Badmatch`.

### 6c. F6 — a trap after an observable side effect is STILL refused, and says why

```
---- BEAMR-R8-DEOPT R5 / F6: traps after an observable side effect ----
  Send then Badmatch   : REJECTED reason: unsupported JIT opcode: runtime-deopt-capable
      Badmatch { value: X(0) } is reachable after an observable side effect
      (a deopt would replay the effect on restart)
  Send then Badrecord  : REJECTED reason: … Badrecord …  (same guard)
  Send then CaseEnd    : REJECTED reason: … CaseEnd …    (same guard)
  Send then IfEnd      : REJECTED reason: … IfEnd …      (same guard)
```

**These rejections are CORRECT, not an incomplete fix.** A deopt restarts the
callee from its bytecode entry, so the `Send` would be replayed. This is the same
`reject_deopt_after_side_effect` guard that protects `func_info` and the
recv-markers. At the estate-side instrument such a function measures as "not
0/9,864" and would read as an incomplete fix when it is not — **which is why this
census reports the reason and never a bare count.**

### 6d. The Wall, asserted mechanically

```
---- BEAMR-R8-DEOPT R5: the scope wall, asserted ----
  wave-2 Catch            : REJECTED reason: unsupported JIT opcode: Catch { … }
  wave-2 CatchEnd         : REJECTED reason: unsupported JIT opcode: CatchEnd { … }
  wave-2 TryCaseEnd       : REJECTED reason: unsupported JIT opcode: TryCaseEnd { … }
  wave-2 Raise            : REJECTED reason: unsupported JIT opcode: Raise { … }
  wave-2 RawRaise         : REJECTED reason: unsupported JIT opcode: RawRaise
  wave-2 BuildStacktrace  : REJECTED reason: unsupported JIT opcode: BuildStacktrace
```

The exception machinery stays wave-2 and stays refused. Scope creep would fail
this test.

### 6e. The AWL `let assert` LIST burst — still refused, NOT for its terminal

```
---- BEAMR-R8-DEOPT R5: the AWL let-assert LIST burst ----
  awl assert_list burst : REJECTED reason: unsupported JIT opcode: TypeTest(IsNonemptyList)
```

The specimen is `burst.rs:349-395` in shape, BC-2b-5 subject `Move` included. The
test asserts the refusal names the TYPE TEST and asserts it does **not** name
`Badmatch`. **This bears directly on the estate-side R5 arm — see §10 F-AWL.**

### 6f. Happy paths run native

R5's "their happy paths run native" is witnessed by the R3/R3b fixtures, each of
which asserts a live `JitCache` entry for the specimen **before** driving, and
then runs the happy path through that cached body in the same run (§4, §5).

---

## 7. R4 — REPLAY PARITY ACROSS THE DEOPT BOUNDARY

`crates/beamr/tests/jit_deopt_replay_parity.rs`. **Distinct from R3, and R3 is
not substituted for it**: R3 compares one function's error; R4 compares a whole
workflow-shaped run's OUTPUT — every leg result, the derived routing token, and
the routed-or-trapped outcome — byte for byte, on a run that TAKES the error
edge. All three modes are driven in each arm; the happy-path-only comparison is
present but is explicitly not the evidence.

### The harness shape, stated for the judge

beamr has **no existing whole-run JIT-vs-interpreter parity harness to extend**.
`tests/differential.rs` is a PER-FUNCTION differential runner; the
`jit_*_replay_probe` files compare an exit reason and a side-effect count, not a
run's output term. beamr's deterministic-replay scheduler could not host it
either — **finding F9, §10**. So the harness is new:

- one `Scheduler` per arm over the same `awl_replay` module;
- **arm A**: JIT live, heated on the HAPPY modes until `route/1` is cached
  native, so the trapping mode's edge is taken IN NATIVE CODE;
- **arm B**: `set_jit_enabled(false)` applied after construction and **before the
  first spawn** — nothing is ever compiled and nothing already compiled is ever
  entered;
- all three modes driven in each arm, the trapping one last;
- each mode's `{run, Legs, Token, Outcome}` rendered through
  `beamr::term::format::format_term` and compared byte for byte, alongside the
  exit reason and any host error.

### The transcript

```
---- BEAMR-R8-DEOPT R4 replay-parity transcript ----
  routing step cached native? JIT'd-with-deopt arm=true never-JIT'd arm=false
  mode clean
    JIT'd-with-deopt : exit=Normal out={run, [{gate_result, fmt, 0, true}, …
                       {gate_result, gates_worker, 0, true}], green, {routed, clean}}
    never-JIT'd      : exit=Normal out={run, [{gate_result, fmt, 0, true}, …
                       {gate_result, gates_worker, 0, true}], green, {routed, clean}}
  mode failing
    JIT'd-with-deopt : exit=Normal out={run, […, {gate_result, tests, 101, false}, …],
                       red, {routed, failing}}
    never-JIT'd      : exit=Normal out={run, […, {gate_result, tests, 101, false}, …],
                       red, {routed, failing}}
  mode unmeasured <- ERROR EDGE TAKEN
    JIT'd-with-deopt : exit=Normal out={run, [{gate_result, fmt, 0, true}, …], no_verdict,
                       {trapped, error, {case_clause, no_verdict},
                        [{awl_replay, route, 1, []},
                         {awl_replay, run, 1, [{line, 44}]},
                         {awl_replay, run, 1, [{line, 32}]}]}}
    never-JIT'd      : exit=Normal out={run, [{gate_result, fmt, 0, true}, …], no_verdict,
                       {trapped, error, {case_clause, no_verdict},
                        [{awl_replay, route, 1, []},
                         {awl_replay, run, 1, [{line, 44}]},
                         {awl_replay, run, 1, [{line, 32}]}]}}
```

**Byte-identical on all three modes, including the trapping one**, with the
`{case_clause, no_verdict}` reason and the full stacktrace carried through the
whole run's output.

The test additionally asserts the trapping mode really trapped (it must contain
`trapped` and `case_clause`) and that the happy modes routed and did not trap —
so this cannot silently degrade into "a second happy-path parity run wearing R4's
name."

*One fixture correction worth recording, because the assertion caught it.* The
routing step was first written as a two-clause `route(green) -> …; route(red) -> …`.
That traps through `func_info` — the LEG 1c A2 landing pad, already Supported
before this brief — and the transcript showed `function_clause`, not
`case_clause`. The assertion failed and named it. The fixture now uses one clause
with an inner `case`, so R4 crosses **this brief's** terminal.

---

## 8. R5b — fmt-forced rejoins, DECLARED

Method: mechanism changes made first, then `cargo fmt --all`, with the tree
snapshotted before and after so rustfmt's own edits are separable.

### Category 1 — rustfmt-forced rejoins in the MECHANISM diff: **ZERO**

`cargo fmt --all` changed **no line** in any of the four mechanism files
(`jit/coverage.rs`, `jit/ir_control.rs`, `jit/compiler/dispatch_core.rs`,
`jit/compiler/compiler_tests.rs`). Measured by diffing a pre-fmt snapshot of
`crates/` against the post-fmt tree: the only files rustfmt touched were the new
test files, which are new in their entirety and therefore have no rejoin
category at all. The leg orders anticipated rejoins from collapsing the
four-variant `RejectedIncremental` arm; in the event the hand-written edits were
already rustfmt-clean.

### Category 2 — ARM-TERMINATOR RELOCATIONS (mechanism-adjacent, 3 lines)

Not rustfmt's doing, but the same review purpose, so declared here: three lines
changed **only because a match arm's `=>` terminator moved to the new last
alternative**, with no semantic change to the line itself.

| File | Line (post) | The relocation |
|---|---|---|
| `crates/beamr/src/jit/coverage.rs` | 109 | `\| Instruction::FuncInfo { .. } => Coverage::Supported,` → `\| Instruction::FuncInfo { .. }` |
| `crates/beamr/src/jit/coverage.rs` | 333 | `\| Instruction::FuncInfo { .. } => true,` → `\| Instruction::FuncInfo { .. }` |
| `crates/beamr/src/jit/ir_control.rs` | 523 | `\| Instruction::RecvMarkerUse { .. } => {}` → `\| Instruction::RecvMarkerUse { .. }` |

Every other line in the mechanism diff is a real mechanism change or a comment
that states one.

### Category 3 — rustfmt-forced rejoins in the VENUE-REMEDIATION diff: **ONE**

The house rule covers the whole diff, so the one genuine rejoin is declared here
rather than left inside §9's remediation. In
`crates/beamr/src/native/file_meta_bifs.rs`, dropping `as u32` shortened the
guard enough that rustfmt collapsed the arm's block body onto one line:

```diff
-        value if value == libc::S_IFBLK as u32 || value == libc::S_IFCHR as u32 => {
-            atom_table.intern("device")
-        }
+        value if value == libc::S_IFBLK || value == libc::S_IFCHR => atom_table.intern("device"),
```

Three source lines become one. **No mechanism change** — the same guard, the same
call, the same atom. Every other line in the venue-remediation diff
(`io/uring.rs` included, checked hunk by hunk) is a real edit or a comment
stating one; rustfmt re-flowed nothing else.

### Category 4 — VENUE REMEDIATION: see §9, kept entirely separate

---

## 9. Venue remediation — the clippy leg was RED AT BASE on this box

**Declared as its own diff category so review never reads it as mechanism.**

### 9a. Finding F10a — the `clippy` leg

While closing the leg, the canon's `clippy` leg refused. **The cause is not this
brief's change.** Measured at the pinned base in a separate worktree:

```
$ git worktree add /tmp/base-check 272ca9e034b0f78f50e30ec1068ea1eca967e6d6
$ cd /tmp/base-check && cargo clippy --workspace --all-targets --features beamr/encode \
      --message-format=json --keep-going -- -D warnings
BASE_CLIPPY_EXIT=101      # 15 errors
```

The 15 errors at base are **byte-for-byte the same** as the 15 in the working
tree before remediation, and every one is in a file this brief does not touch:
`crates/beamr/src/io/uring.rs` (7) and `crates/beamr/src/native/file_meta_bifs.rs`
(8). With `--keep-going` clearing those, a second layer surfaced in
`native/file_meta_bifs_tests.rs`, `scheduler/inventory_tests.rs` and
`tests/thread_inventory_disabled.rs`.

**Finding F10a.** The `clippy` and `clippy-all-features` legs of `gates.json`
cannot go green at `272ca9e` on a Linux/glibc box with the pinned `1.97.1`
toolchain. Every failure is platform-conditional — `io/uring.rs` is
`#![cfg(target_os = "linux")]`, the `libc::S_IF*` casts are redundant where
`mode_t == u32` (Linux) and load-bearing where it is not, and three import sets
are used only inside `#[cfg(target_os = "macos")]` blocks. CI is `ubuntu-latest`
on the same pinned toolchain (`.github/workflows/ci.yml`), so this is not a
this-box-only artefact of an unusual environment.

**Disposition.** The gate's own instruction is "a red names its cause — fix the
cause, never the gate", and `crates/` is inside the wall, so the causes were
fixed. The changes are mechanical and behaviour-preserving:

| File | Change | Why it is safe |
|---|---|---|
| `io/uring.rs` | drop the unused `RawFd` and `TryRecvError` imports | unused |
| `io/uring.rs` | `io::Error::new(ErrorKind::Other, m)` → `io::Error::other(m)` | the same error |
| `io/uring.rs` | three nested `if let`s → one let-chain | the crate already uses let-chains; identical control flow |
| `io/uring.rs` | `loop { match try_recv() { Ok => …, Err(Empty\|Disconnected) => break } }` → `while let Ok(m) = try_recv()` | the `Err` arm covered both variants, so the forms are equivalent |
| `io/uring.rs` | `InFlightOp::SendMsg { .. }` / `RecvMsg { .., .. }` → bind the ownership anchors as `data: _data`, `iov: _iov`, `msg: _msg`, `addr_storage: _addr_storage` | **the idiom this very match already uses** (`Connect { storage: _storage }`, `Openat { path: _path }`). Zero behaviour change: the boxes still live to completion, which is their whole job |
| `native/file_meta_bifs.rs`, `file_meta_bifs_tests.rs` | drop `libc::S_IF* as u32` | `mode_t == u32` on the canon's only platform |
| `scheduler/inventory_tests.rs`, `tests/thread_inventory_disabled.rs` | gate/scope the macOS-only imports | the items are used only inside `#[cfg(target_os = "macos")]` blocks |

**No suppression in any spelling.** No `#[allow]`, `#[expect]`, `#[ignore]` or
`#[cfg(any())]` was added anywhere — the gate's suppression sweep is clean, and
these were fixed rather than silenced. The `#[cfg(target_os = "macos")]` on an
import is a platform gate matching the item it feeds, not a suppression.

**Flagged for the seat, since it is a decision and not mine to make.** Dropping
`libc::S_IFMT as u32` is correct for Linux (`mode_t == u32`) and would fail to
compile where `mode_t` is narrower (macOS: `u16`). The canon measures
`ubuntu-latest` only, so this does not weaken any gate — but if beamr intends to
build on macOS, that cast should come back behind a `cfg`, and `gates.json`
should grow a macOS leg so the tree stops discovering this at a venue.

### 9b. Finding F10b — the `tests` leg was ALSO red at base: a thread name the OS cannot carry

The canon's `tests` leg refused too, and again the cause is not this brief's
change. Measured in the same base worktree:

```
$ cd /tmp/base-check && cargo test -p beamr --test thread_inventory_distribution \
      --features beamr/encode
an owned distribution bundle must build both runtime workers: dist-send=1, net-kernel=0
test result: FAILED. 0 passed; 1 failed
BASE_EXIT=101
```

Identical message in the working tree before remediation. **The cause, found by
dumping the OS probe rather than guessing:**

```
OS THREADS: [("beamr-dist-send", 1), ("beamr-io-uring", 2), ("beamr-net-kerne", 1), …]
  service=distribution mode=Owned configured=2 actual=2
      names=["beamr-dist-send", "beamr-netkernel"]
```

`"beamr-net-kerne"` — **truncated**. Linux caps a thread name at
`TASK_COMM_LEN - 1` = **15 bytes**, and `/proc/self/task/*/comm` (the ground
truth the spec §5 inventory is validated against) reports the truncated form.
`NET_KERNEL_THREAD_NAME` was `"beamr-net-kernel"` — **16 bytes**. Its sibling
`DIST_SEND_THREAD_NAME` is `"beamr-dist-send"` — exactly 15, which is why one
worker was found and the other was not. The names were evidently sized on macOS,
where the probe uses mach thread ports and the cap does not bite; the test's own
header names both platforms.

This is a real defect, not a test artefact: `NetKernel::worker_thread_names`
claimed a thread name the OS never carried, so the inventory **silently
over-reported** on every Linux host.

**Fix:** `NET_KERNEL_THREAD_NAME` is now `"beamr-netkernel"` — 15 bytes — with a
doc comment stating the constraint, the prior value, and why the length is
load-bearing, so the next name is not chosen blind. Prose mentions of the old
string were updated in `distribution/mod.rs`, `scheduler/distribution_service.rs`,
`scheduler/inventory.rs`, `scheduler/execution.rs`,
`tests/thread_inventory_distribution.rs` and `tests/with_services.rs`. The test
was NOT touched — the product was.

**Sibling defect, reported and NOT fixed.** The same audit found
`IO_COMPLETION_THREAD_NAME = "beamr-io-completion"` (`crates/beamr/src/io/bridge.rs`)
at **19 bytes**, which Linux must be truncating to `"beamr-io-comple"`. No gate
leg probes it today, so it is not a red and it was left alone rather than
renamed on this leg's initiative — a rename is a public-constant change with no
failing measurement behind it. Recorded so the seat can decide. The full audit of
`*_THREAD_NAME` constants:

| Constant | Bytes | Status |
|---|---|---|
| `DIST_SEND_THREAD_NAME` = `beamr-dist-send` | 15 | fits |
| `NET_KERNEL_THREAD_NAME` = `beamr-netkernel` | 15 | **fixed here** (was 16) |
| `IO_COMPLETION_THREAD_NAME` = `beamr-io-completion` | 19 | **over-length, reported, not fixed** |

---

## 10. Findings — F7 through F10, recorded in the register of F1–F6

Plan findings **F1–F6 are binding and were not re-litigated**. F1 is discharged
in §2d, F2 in §4d, F3 in §5 (full `ablative-io/aion` paths carried throughout),
F4 in §4c, F5 in §3a (no divergence to report), F6 in §3c and §6c. Four further
drifts were measured at the bytes on this leg.

### F7 — a deopt AFTER a frame push duplicates the frame in the raise-time stacktrace. **Pre-existing; measured with a control.**

`crates/beamr/tests/jit_deopt_frame_restart_control.rs`.

When a compiled body executes `Allocate` and then deopts, the native frame is
**not unwound** — `JIT_STATUS_DEOPT` returns `Ok(None)`, the caller re-enters the
callee at its bytecode entry, and the interpreter executes `Allocate` again. The
raise-time stacktrace then carries a duplicated frame.

**This is not introduced by this brief, and the control proves it.** Two
specimens, identical in structure — `Allocate` a frame, take a fail edge —
differing ONLY in the terminal the fail edge reaches:

```
---- BEAMR-R8-DEOPT F7: deopt after a frame push ----
  FuncInfo  (the LEG 1c A2 TEMPLATE, unchanged by this brief):
      native frames=4 arities=[1, 1, 1, 1] | never-JIT'd frames=3 arities=[1, 1, 1]
  Badmatch  (THIS BRIEF, template body verbatim):
      native frames=4 arities=[1, 1, 1, 1] | never-JIT'd frames=3 arities=[1, 1, 1]
  frame delta: template=1 this-brief=1
```

The `func_info` template — which predates this brief and which R2 told this leg to
copy — diverges by **exactly the same one frame**. The defect belongs to the
deopt-restart seam, and it is reachable today by every already-`Supported`
`is_runtime_deopt_capable` instruction that can follow a frame push: an `{f,0}`
arithmetic `Bif`, `CallExt*`, `PutList`, `PutTuple2`, `MakeFun`, `TestHeap`, the
recv-markers.

**Class and reason term are unaffected** — this touches stacktrace SHAPE only,
and only for a specimen that allocates a frame before its trap. **Not fixed
here:** fixing it means changing the deopt seam for every deopt-capable
instruction, which is a different brief and outside this one's Walls ("No new
mechanism — the func_info path is the template"). **Recommendation:** a follow-up
should unwind the native frame on the deopt return, or record the stack depth at
native entry and restore it, before any brief relies on stacktrace shape across a
deopt in frame-bearing code.

### F8 — `call_last` never dispatches compiled code

`interpreter/opcodes/core.rs`'s `call_last` deallocates the frame and jumps; it
does **not** call `try_dispatch_local_jit`. `Call` and `CallOnly` both do.
Measured directly: driving `happy_gate_passed/1 -> gate_passed(gate_result(Sel))`
eight times, erlc emits `{call,1,{f,gate_result}}` then `{call_last,1,{f,gate_passed},0}`,
and the profiler showed `gate_result: calls=Some(2)` but **`gate_passed: calls=None`,
`cached=false`** — the callee was never even profiled, let alone entered natively.

Consequence for the estate-side arm: **a function reached only by a tail call
from a frame-bearing caller is never entered natively, however well it is
classified.** erlc emits `call_last` routinely. This is orthogonal to R8 and not
fixed here; the fixtures work around it by shaping their callers to emit
`call`/`call_only`, and that workaround is documented in
`fixtures/awl_terminals.erl` rather than left silent.

### F9 — nothing in the tree can produce a `ReplayLog`, so the deterministic-replay scheduler cannot host a parity run

`ReplayRecorder` is never constructed anywhere in `crates/beamr/src` or
`crates/beamr-cli/src` — it exists only as a standalone builder. A live run
therefore cannot produce a `ReplayLog`. Running the R4 workflow under
`Scheduler::new_replay_with_registry` with a hand-built empty log fails at the
first scheduling decision, measured:

```
replay mismatch: replay log exhausted before timer expiry decision at replay cursor 0
```

Hand-authoring an event log covering every scheduling decision would be
scaffolding, not evidence, so R4's harness uses the ordinary scheduler in both
arms and says so in its own header (§7). Recorded because a reader may reasonably
expect "replay parity" to mean beamr's `crate::replay`; today it cannot.

### F11 — the `nostd-ratchet` canon leg CANNOT MEASURE on this venue: a BSD `mktemp` idiom in the leg script. **The underlying ratchet is a PASS.**

This is the one canon leg that did not go green, and the cause is the
INSTRUMENT, not the tree. `scripts/gate-nostd-ratchet.sh` line 203:

```sh
LOG="$(mktemp -t nostd-ratchet)"
```

`mktemp -t PREFIX` is the BSD/macOS spelling. GNU coreutils requires the template
to end in at least three `X`s, so on this box:

```
$ mktemp -t nostd-ratchet
mktemp: too few X's in template 'nostd-ratchet'   (mktemp (GNU coreutils) 9.10)
```

`LOG` is therefore empty, `cargo check ... > "" 2>&1` fails with rc=1 having
captured nothing, the tally line is absent, and the script's own REFUSE arm fires
— correctly, because a gate that cannot measure must not report. The leg log
names it in full:

```
  leg nostd-ratchet rc=3 — last 30 lines:
    mktemp: too few X's in template 'nostd-ratchet'
    ./scripts/gate-nostd-ratchet.sh: line 203: : No such file or directory
    ./scripts/gate-nostd-ratchet.sh: line 205: : No such file or directory
    no-std ratchet: cargo rc=1, rustc tally=<absent>, ceiling=1051
    REFUSE: cargo exited 1 but no "due to N previous errors" line was found.
```

**The script already knows.** Its own ceiling comment records this venue
behaviour and what it cost: *"it went unseen because this leg reds STRUCTURALLY
on the Linux venue (#31), so the landing's own differential recorded rc=3 on both
arms and counted two dead instruments as a matching leg."*

**Pre-existing, proven:** `git diff 272ca9e...HEAD -- scripts/ gates.json` is
EMPTY — this leg touched neither. The failure is deterministic on GNU coreutils,
so it is structural at the pinned base and was simply never reached there (the
base run stopped at `deliverable_absent`).

**The tree is a PASS on the merits, measured.** Running the script's own logic
with ONLY the template repaired (`nostd-ratchet` → `nostd-ratchet.XXXXXX`), from
a copy in `/tmp` so nothing in the tree is touched:

```
$ bash /tmp/nostd-fixed.sh
no-std ratchet: cargo rc=101, rustc tally=1051, ceiling=1051
PASS: no-std errors 1051, exactly at the ceiling 1051. Debt held.
VERDICT_RC=0
```

**1051, exactly at the ceiling.** This brief added no no-std debt and removed
none, which is precisely what the ratchet demands.

**Why it is NOT fixed here, stated as the conflict it is.** The one-line repair
lives in `scripts/gate-nostd-ratchet.sh`. `scripts/` is **outside this leg's
wall** — `gate-entry.sh` §3 enforces `^(crates/|conformance/|probes/|\.fleet/)`
and would red with named cause `wall_breach`. So the two reachable states on this
venue are `canon_red` (leave it) or `wall_breach` (fix it); **GATE-GREEN is not
reachable from inside this wall**, and manufacturing one would mean either
editing the gate or widening the wall, both of which the orders forbid. The leg
orders' own instruction for this case is followed: *"Any other red at base is a
rig finding — report it, do not route around it, do not edit the gate."*

**The fix, for whoever holds the wider wall** — one character class:

```diff
-LOG="$(mktemp -t nostd-ratchet)"
+LOG="$(mktemp -t nostd-ratchet.XXXXXX)"
```

Worth noting for the seat: CI is `ubuntu-latest` (`.github/workflows/ci.yml`) on
the same pinned toolchain, so this leg is dead in CI too, and has been. A gate
that cannot measure has been reporting for some time.

### F12 — stacktrace frames are attributed to the FOLLOWING function's header line

Noticed while verifying the R3 transcripts against the fixture source, recorded
because it is checkable and reproducible. It is **pre-existing and independent of
the JIT** — both arms agree exactly, which is why R3's identity property is
unaffected — and it was NOT chased to root cause, being outside this brief.

Measured against `crates/beamr/tests/fixtures/awl_terminals.erl`:

| Frame reported | Function's own source span | Line reported | What is actually at that line |
|---|---|---|---|
| `verdict_code/1` | 55–61 | **73** | `first_failing_target(FailingTargets) ->` — the next function's header |
| `report_presence/1` | 79–82 | **85** | `gate_passed(R) ->` — the next function's header |
| `probe_verdict/1` (frame 2) | 123–128 | **130** | `probe_failing_target(Sel) ->` — the next function's header |
| `probe_verdict/1` (frame 3) | 123–128 | 124 | `try verdict_code(V) of` — **correct** |
| `probe_presence/1` (frame 2) | 137–142 | **149** | `probe_gate_passed(Sel) ->` — the next function's header |
| `probe_presence/1` (frame 3) | 137–142 | 138 | `try report_presence(Report) of` — **correct** |

The pattern is consistent: every frame but the outermost reports the NEXT
line-table entry rather than the entry at-or-before its instruction pointer. The
committed `.beam` files are confirmed to match their `.erl` sources byte for byte
(`erlc` re-run into a temp dir and `cmp`'d), so this is not a stale-fixture
artefact. A reader debugging from a beamr stacktrace today is being pointed one
function too far down the file.

### F-AWL — the AWL `let assert` list burst is STILL rejected after R8, for a reason that is not the terminal. **This bears on R5's estate-side arm.**

`assert_list` (`ablative-io/aion` `crates/aion-awl/src/mir/select/emit/burst.rs:344-396`)
walks with `TypeTestOp::IsNonemptyList` and terminates on `IsNil`. This JIT tier
lowers **neither** — `jit/ir_control_validation.rs` admits only `IsInteger`,
`IsAtom`, `IsPid`, `IsBinary`, `IsList`, `IsTuple`. Measured in §6e: the burst
shape is refused with `unsupported JIT opcode: TypeTest(IsNonemptyList)`, and the
erlc-emitted list destructure in the corpus fixture is refused identically (§6b).

**So a corpus function whose only poison was a `let assert [...]` burst does NOT
clear on the terminals alone.** The `CaseEnd` half of the census (the SelectVal
fall-through trap, `emit/control.rs:133-134`) is fully unblocked by this brief;
the `Badmatch` half is unblocked at the terminal but may still be blocked at the
walk. This is named here so it is not discovered as a surprise at the pin bump,
and so the estate-side measurement can separate the populations by reason.

---

## 11. R6 — estate discipline

- **No suppression in any spelling.** Zero `#[ignore`, `#[allow`, `#[expect`,
  `#[cfg(any()` added anywhere in the `.rs` diff. The gate's own suppression
  sweep enforces this mechanically and reports `clean`. Where the venue lint red
  could have been silenced with one `#[allow]`, the cause was fixed instead (§9).
- **The consistency walk is not weakened.** Only the derived count moved. The
  representative table is untouched, the walk's assertions are unchanged, and it
  still drives all 75 variants through `compile`.
- **No coverage-table wildcard, ever.** All three `Instruction` tables remain
  exhaustive with no wildcard arm; adding a variant still breaks compilation
  until it is classified.
- **Scope held.** `Catch`, `CatchEnd`, `TryCaseEnd`, `Raise`, `RawRaise` and
  `BuildStacktrace` are untouched and still refused — asserted in §6d, not
  assumed.
- **Artemis's stdlib skip count (276/454) was NOT measured and is NOT cited.**
  Stdlib leans on the wave-2 exception machinery, which is out of scope;
  conflating the populations would manufacture a red.

---

## 12. The estate-side R5 arm — OWED, not discharged

**Nothing in this venue can prove 0/9,864, and nothing here claims to.**

Calliope's arm, carried as an **OWED OBLIGATION**: at the next beamr pin bump in
`aion`, re-run the census poisoning measurement at the SAME instrument. The
51/9,864 poisoned set must measure **0/9,864**, and the 75-variant consistency
walk must hold. If ANY of the 51 still rejects, the fix is incomplete, not
"mostly landed."

Two things this leg measured that the re-run must read alongside the count, or it
will mis-grade a correct fix:

1. **F6 (§6c)** — a function performing an observable side effect on a path
   reaching its trap edge is STILL rejected, now by
   `reject_deopt_after_side_effect` rather than `UnsupportedOpcode`. That
   rejection is CORRECT. **Report per-function reasons, never a bare count.**
2. **F-AWL (§10)** — a function poisoned by a `let assert [...]` LIST burst is
   still rejected at `TypeTestOp::IsNonemptyList`, which is not one of the four
   terminals and is not in this brief's scope. The `CaseEnd` population should
   clear; part of the `Badmatch` population may not.
3. **F8 (§10)** — admission is not the same as entry. `call_last` never
   dispatches compiled code, so a cured function reached only by a tail call from
   a frame-bearing caller is admitted but never entered natively. If the
   estate-side arm also measures native entry (not just admission), that is the
   number to watch, and it is orthogonal to this brief.

---

## 13. Files changed

### Mechanism (the brief)

| File | What |
|---|---|
| `crates/beamr/src/jit/coverage.rs` | R1 — the four to `Supported`; the four to `is_runtime_deopt_capable == true`; doc corrections |
| `crates/beamr/src/jit/ir_control.rs` | R2 — pre-pass acceptance + `validate_read_operand`; the four joined to `control_flow_successors`' no-successor arm |
| `crates/beamr/src/jit/compiler/dispatch_core.rs` | R2 — the four lowered with the `func_info` body verbatim |
| `crates/beamr/src/jit/compiler/compiler_tests.rs` | R1 — derived Supported count 60 → 64 |

### Fixtures and tests (R3, R3b, R4, R5, findings)

| File | What |
|---|---|
| `crates/beamr/tests/fixtures/awl_terminals.erl` / `.beam` | corpus-derived specimens for CaseEnd, IfEnd, Badrecord, and the erlc Badmatch rejection specimen |
| `crates/beamr/tests/fixtures/awl_replay.erl` / `.beam` | the R4 workflow, derived from `gates.awl`'s seven-leg battery |
| `crates/beamr/tests/jit_error_terminal_identity.rs` | R3 + R3b — CaseEnd, IfEnd, Badrecord |
| `crates/beamr/tests/jit_badmatch_restart_fidelity.rs` | R3 + R3b — Badmatch, the restart-fidelity fixture |
| `crates/beamr/tests/jit_deopt_replay_parity.rs` | R4 |
| `crates/beamr/tests/jit_error_terminal_prepass_acceptance.rs` | R5 |
| `crates/beamr/tests/jit_deopt_frame_restart_control.rs` | F7, with its `func_info` control |

### Venue remediation (§9 — pre-existing at base, kept separate)

Clippy leg (§9a): `crates/beamr/src/io/uring.rs`,
`crates/beamr/src/native/file_meta_bifs.rs`,
`crates/beamr/src/native/file_meta_bifs_tests.rs`,
`crates/beamr/src/scheduler/inventory_tests.rs`,
`crates/beamr/tests/thread_inventory_disabled.rs`.

Tests leg (§9b): `crates/beamr/src/distribution/mod.rs`,
`crates/beamr/src/scheduler/distribution_service.rs`,
`crates/beamr/src/scheduler/inventory.rs`,
`crates/beamr/src/scheduler/execution.rs`,
`crates/beamr/tests/thread_inventory_distribution.rs`,
`crates/beamr/tests/with_services.rs`.

Everything is inside the wall (`crates/`, `.fleet/`). Nothing outside it was
touched. Nothing was pushed.
