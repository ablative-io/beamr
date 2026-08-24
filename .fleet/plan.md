# PLAN — BEAMR-R8-DEOPT (fleet run beamr-r8-flight1)

- **Rig branch**: `fleet/beamr-r8-base`, base `272ca9e034b0f78f50e30ec1068ea1eca967e6d6`
  (beamr main, pinned — the brief's own floor commit). Tip at planning: `3357073f`.
- **Brief**: `.fleet/BRIEF.md`, RATIFIED AS AMENDED by Artemis Peach 2026-08-24.
  Both grill amendments (R1 all four tables; R2 pre-pass operand-validation arms)
  are folded in and BINDING.
- **Author of this plan**: planning counsel. I change NOTHING outside `.fleet/`.
- **Fixture note**: beamr is our own product and this box is our own fleet venue.
  Every estate, key, corpus document and repository named below is a first-party
  local test fixture; the `ablative-io/aion` clone used for citation grounding is
  our own public repo.

---

## 1. Citation ledger — every brief citation, checked at the bytes

The commission was to ground each citation myself and record drift rather than
paper over it. Eight citations, checked. **Five GROUNDED, three DRIFTED.**

| # | Brief citation | Verdict | What is actually there |
|---|---|---|---|
| C1 | `func_info` precedent, `ir_control.rs:361-368` | **GROUNDED** | `crates/beamr/src/jit/ir_control.rs:361-365` is the LEG 1c A2 comment ("a DEOPT terminal reached only via a dispatch fail edge … raises error:function_clause"); `:366-368` is `Instruction::FuncInfo { .. } => { block_starts.insert(index + 1); }`. Exact. |
| C2 | whole-function rejection walk, `ir_control.rs:369-381` | **GROUNDED** | `:369` `other =>`, `:372-377` the `debug_assert_ne!(coverage(other), Coverage::Supported, …)`, `:378-380` `return Err(JitError::UnsupportedOpcode { opcode: opcode_name(other) })`. The walk errs on the FIRST non-Supported variant — whole-function, no compile-around. Exact. |
| C3 | `coverage` table | **GROUNDED** | `crates/beamr/src/jit/coverage.rs:40`. The four terminals sit at `:123-128` as `RejectedIncremental { reason: "wave 2: error-raising terminals" }` — the wave-2 comment the brief cites, verbatim. |
| C4 | `is_observable_side_effect` table | **GROUNDED** | `coverage.rs:167`. The four sit at `:239-242` on the `false` side. Exhaustive, no wildcard. |
| C5 | `is_runtime_deopt_capable` table | **GROUNDED** | `coverage.rs:291`. The four sit at `:362-365` on the `false` side, under the comment `-- non-Supported variants: never lowered (pre-pass rejects first) --` (`:354`). Exhaustive, no wildcard. |
| C6 | `is_no_fail_label` as the fourth classification table, "its own header, lines 3-9" | **DRIFTED — see F1** | The header at `coverage.rs:3-9` does say what the brief quotes. The function at `coverage.rs:376-378` does NOT match it: it takes `&Operand`, not `&Instruction`, and is a one-line `matches!`. It cannot classify the four terminals. |
| C7 | CaseEnd construction site, `emit/control.rs:134` | **GROUNDED — but foreign repo, see F3** | `ablative-io/aion` `crates/aion-awl/src/mir/select/emit/control.rs:134` = `self.push(Instruction::CaseEnd { value });`, immediately after `:133` `self.push(Instruction::Label { label: fail });`. Confirms "the SelectVal fall-through trap", cold edge only. No such path in beamr. |
| C8 | Badmatch construction sites, `emit/burst.rs:294/393` | **GROUNDED — but foreign repo, see F3/F4** | `crates/aion-awl/src/mir/select/emit/burst.rs:294` and `:393`, both `self.push(Instruction::Badmatch { value: Operand::X(0) });`. `:386-388` carries the BC-2b-5 comment verbatim. No such path in beamr. |

Supporting bytes checked while grounding the above:

- The four `Instruction` variants: `crates/beamr/src/loader/decode/instruction.rs:243-252` —
  `Badmatch { value: Operand }`, `Badrecord { value: Operand }`, `CaseEnd { value: Operand }`,
  `IfEnd` (fieldless). `FuncInfo { module, function, arity }` at `:8-12`.
- The `func_info` LOWERING (the R2 template): `crates/beamr/src/jit/compiler/dispatch_core.rs:423-426`.
- The native entry skips the prelude: `crates/beamr/src/jit/compiler/dispatch.rs:271-282`.
- The 75-variant consistency walk: `crates/beamr/src/jit/compiler/compiler_tests.rs:3852`,
  with the representative table at `:3538` and the derived-count assertion at `:3901-3904`.
- The deopt-after-side-effect dataflow: `crates/beamr/src/jit/ir_control.rs:582-616`,
  over `control_flow_successors` (`:460-497`).
- Deopt restart semantics: `crates/beamr/src/interpreter/opcodes/core.rs:950` and `:958-960`.

**Measured at base**: `cargo test -p beamr --lib --features beamr/encode coverage_` →
4 passed, 0 failed. `coverage_walk_agrees_with_prepass_and_dispatch_for_all_75_variants`
and `coverage_table_has_one_entry_per_instruction_variant` are both green, with
Supported = 60 (`compiler_tests.rs:3902`). That is the baseline the change moves to 64.

---

## 2. Findings — recorded, not papered over

### F1 — `is_no_fail_label` is not an instruction-classification table. R1's amendment is not literally dischargeable.

`crates/beamr/src/jit/coverage.rs:376-378` in full:

```rust
pub(crate) fn is_no_fail_label(operand: &crate::loader::decode::Operand) -> bool {
    matches!(operand, crate::loader::decode::Operand::Label(0))
}
```

It takes an `Operand`. It has no match over `Instruction` and therefore no
exhaustiveness property at all — adding an `Instruction` variant cannot break it.
Its four call sites (`ir_control.rs:227`, `:409`, `:537`; `dispatch_core.rs:195`)
all pass `parsed.fail` from a `Bif`: it answers "is this Bif's fail label the
`{f,0}` sentinel", never "what class is this instruction". Its own doc comment
(`coverage.rs:373-375`) says exactly that: "erlc's `{f,0}` fail-label sentinel".

The module header at `coverage.rs:3-9` over-claims — it groups this predicate with
the three real tables and asserts all four are "EXHAUSTIVE WITH NO WILDCARD ARM".
The brief inherited that over-claim. R1's "ALL FOUR must classify the four
terminals" is structurally impossible for this one.

**Disposition (binding on the builder).** Three tables take arms: `coverage`,
`is_observable_side_effect`, `is_runtime_deopt_capable`. `is_no_fail_label` is
discharged by INSPECTION and DECLARATION, not by edit. The semantic R1 gestures at
is true and checkable: the four terminals carry no fail label of their own —
`Badmatch`/`Badrecord`/`CaseEnd` carry one `value: Operand` each and `IfEnd`
carries nothing (`instruction.rs:243-252`), so none of them ever reaches
`is_no_fail_label`. Record that in the flight report against those bytes.

Do NOT reshape `is_no_fail_label` into an `Instruction` table — that is inventing
mechanism, which R2 forbids in its own words. Do NOT add a wildcard anywhere —
the Walls forbid it. Do NOT quietly skip R1 either: the declaration is owed.

### F2 — `Badrecord` has no interpreter execution arm. The one-sentence contract is false for one of the four.

`crates/beamr/src/interpreter/opcodes/mod.rs:381-383` dispatches three of the four:

```
381: Instruction::Badmatch { value } => exceptions::badmatch(process, module, value),
382: Instruction::CaseEnd { value }  => exceptions::case_end(process, module, value),
383: Instruction::IfEnd              => exceptions::if_end(process),
```

There is no `Badrecord` arm. `crates/beamr/src/interpreter/opcodes/exceptions.rs`
defines `badmatch` (`:143`), `case_end` (`:154`), `if_end` (`:165`) — and no
`badrecord`. `Instruction::Badrecord` therefore falls to the catch-all at
`interpreter/opcodes/mod.rs:451`: `other => Err(ExecError::UnsupportedOpcode { … })`.
The variant is known only as a name (`:502`).

The brief's contract says the four "deopt to the resident interpreter, which
raises the error". For `Badrecord` the resident interpreter does not raise
`error:{badrecord, V}` — it returns a host `ExecError`.

**Disposition.** Error identity still holds, but trivially: today a `Badrecord`
function is rejected from JIT, runs interpreted, and yields `UnsupportedOpcode`;
after the change it is JIT'd, deopts at the terminal, restarts interpreted, and
yields the same `UnsupportedOpcode`. Both arms agree. The R3 fixture for
`Badrecord` is therefore BUILDABLE but HOLLOW — it witnesses identity of a
non-raise. It must be reported as such, in those words, and must not be presented
as evidence that the terminal raises correctly. Nothing in this brief licenses
implementing `badrecord` in the interpreter; that is out of scope and out of the
Walls.

This does not touch R5: the census says AWL's 27 emitted variants put only
`Badmatch` and `CaseEnd` outside coverage, so no corpus function depends on
`Badrecord`.

### F3 — Two citations are foreign-repo paths. A judge grepping beamr will find nothing.

`emit/control.rs` and `emit/burst.rs` are AWL-side, in `ablative-io/aion` (public,
first-party, clonable from this box — verified). beamr has no `burst.rs` at all,
and its only `control.rs` is `crates/beamr/src/distribution/control.rs`, unrelated.
Both citations resolve exactly once the repo is named — see C7/C8. The plan and the
report must carry the full `ablative-io/aion crates/aion-awl/src/mir/select/emit/…`
paths so the grounding is reproducible.

### F4 — The Badmatch operand is ALWAYS `X(0)`. The subject-vs-tail distinction lives in the PRECEDING `Move`.

`aion crates/aion-awl/src/mir/select/emit/burst.rs:386-393`:

```
386: // Trap on the SUBJECT list, not the walked tail X0 happens to hold —
387: // `let assert [...] = subject` reports the whole subject (BC-2b-5
388: // carried fix; the walked-tail operand misattributed the mismatch).
389: self.push(Instruction::Move {
390:     source: Operand::Y(self.home(list)?),
391:     destination: Operand::X(0),
392: });
393: self.push(Instruction::Badmatch { value: Operand::X(0) });
```

Both construction sites emit `Badmatch { value: X(0) }`. **No lowering can report
the walked tail by mis-reading the operand — the operand is identical either way.**
The regression R3's fixture actually guards is different and sharper: a lowering
that lets the interpreter observe `X0` WITHOUT re-executing the `Move` at
`:389-392` — i.e. any lowering that resumes mid-function, or marshals native
register state across the deopt instead of restarting from the function entry.

**Disposition.** R3's Badmatch fixture is a RESTART-FIDELITY fixture. It must
assert the reason term is `{badmatch, <whole subject>}` and not `{badmatch, <tail>}`,
on a specimen where subject and walked tail are distinguishable terms. Recording
"operand was X(0)" proves nothing.

### F5 — R2's "carrying enough state" reads against its own template. The template carries no state.

The `func_info` lowering, `crates/beamr/src/jit/compiler/dispatch_core.rs:423-426`:

```rust
Instruction::FuncInfo { .. } => {
    return_status_raw(builder, JIT_STATUS_DEOPT, JIT_DEOPT_SENTINEL);
    Ok(Some(true))
}
```

A bare sentinel return. No operand marshalling, no state block. The state reaches
the interpreter by RESTART: `JIT_STATUS_DEOPT => return Ok(None)`
(`interpreter/opcodes/core.rs:950`), and the sentinel arm at `:958-960` does the
same — `Ok(None)` means "the JIT declined", so the caller runs the function from
its bytecode entry. `coverage.rs:149-151` states the contract in prose: "deopt
restarts the callee interpreted from its start".

Read literally, R2's "a deopt block transfer carrying enough state for the
interpreter to raise the correct error against the correct operand" invites
building a state-marshalling mechanism — which R2's very next sentence forbids
("No new mechanism — the func_info path is the template"). The two halves of R2
are reconciled by the restart contract, not by transfer.

**Disposition.** The four terminals copy `dispatch_core.rs:423-426` exactly. Any
operand marshalling is the "divergence from the template" R2 itself calls a
finding to record.

### F6 — RISK, and the most likely way R5 goes red while the fix is correct.

Classifying the four as `is_runtime_deopt_capable == true` puts them under
`reject_deopt_after_side_effect` (`ir_control.rs:582-616`): a runtime-deopt-capable
instruction reachable after an observable side effect is rejected, because a
deopt-restart would replay the effect. That rejection is CORRECT — it is the same
guard that protects `func_info` and the recv-markers — but it is still a rejection.

Consequently: any of the 51 poisoned corpus functions that performs an observable
effect (`Send`, `RemoveMessage`, a mutated receive marker, or ANY call form —
`coverage.rs:167-243`) on a path reaching its assert or case-fallthrough edge will
STILL be rejected after this change, now with the deopt-after-side-effect reason
instead of `UnsupportedOpcode`. At Calliope's instrument that measures as
"not 0/9,864" and reads as an incomplete fix when it is not.

**Disposition.** Named here so it cannot be discovered as a surprise at the pin
bump. The builder must report the poisoned-set measurement with the per-function
REJECTION REASON, never a bare count, so the two populations are separable. The
call on whether a side-effect-guarded rejection satisfies R5 is the judge's and
Calliope's, not the builder's.

Corollary the builder must not miss: the four terminals also have to join the
"leaves the function: no in-slice successor" arm of `control_flow_successors`
(`ir_control.rs:485-497`, where `FuncInfo` sits at `:493`). If they are left out,
the union dataflow propagates taint through a fall-through edge that does not
exist, and the guard mis-fires.

---

## 3. Build order

Six steps. R3's error-IDENTITY fixtures and R4's replay parity are first-class
build steps with their own acceptance, not report-writing afterthoughts.

### R0 — Instrument check before any edit (no code)

Run `sh .fleet/gate-entry.sh` once at base. **It will RED, and the expected named
cause is `deliverable_absent`**: at `3357073f` the diff against `272ca9e` is the
three `.fleet/` rig files, all inside the wall, so `NCRATES=0` and the census arm
fires (`gate-entry.sh` §3). That red is the proof the rig arms are live. Record
the receipt. A different red at base is a rig finding — report it, do not route
around it.

### R1 — Classification

Move `Badmatch`, `Badrecord`, `CaseEnd`, `IfEnd` in the three real tables:

1. `coverage.rs:123-128` — out of `RejectedIncremental { reason: "wave 2: error-raising terminals" }`,
   into the `Supported` arm, with a table comment stating the treatment in the
   func_info register: reached via fail edge only; deopts; the restarted
   interpreter raises. Model the comment on `coverage.rs:105-108`.
2. `coverage.rs:239-242` — `is_observable_side_effect`. These stay **false**.
   A terminal that raises performs no effect a restart would duplicate; the raise
   happens in the interpreter, once, after the restart. The `false` arm is one
   flat chain to `=> false` at `:249` with no per-group comments, so this table
   correctly takes **zero diff**; the classification is discharged by declaration
   in the report, not by an edit.
3. `coverage.rs:362-365` — `is_runtime_deopt_capable`. These become **true**, and
   they move out from under the `-- non-Supported variants: never lowered --`
   comment at `:354` (that comment stops being true of them) into the true arm
   beside `FuncInfo` at `:321`.
4. `is_no_fail_label` — **no edit**. Discharge per F1 by inspection + declaration.

Acceptance: `coverage_walk_agrees_with_prepass_and_dispatch_for_all_75_variants`
green with the derived Supported count updated 60 → 64 at `compiler_tests.rs:3901-3904`,
and the comment at `:3898-3900` extended to name this brief. Still exhaustive,
still no wildcard, walk not weakened.

### R2 — Lowering and pre-pass

- **Lowering**: add the four to `dispatch_core.rs` beside `:423-426`, copying the
  `func_info` body verbatim (`return_status_raw(builder, JIT_STATUS_DEOPT, JIT_DEOPT_SENTINEL); Ok(Some(true))`).
  No state marshalling — F5.
- **Pre-pass acceptance + operand validation** (the grill amendment): arms in
  `ir_control.rs`, placed with the `FuncInfo` arm at `:366-368`. Block-start
  insertion follows that arm. `Badmatch`/`Badrecord`/`CaseEnd` each validate their
  `value` through the existing `validate_read_operand` shape used at `:345`, `:353-354`,
  `:358` — accepted, not accepted blind. `IfEnd` is fieldless: block-start only.
- **Successors**: add the four to the no-in-slice-successor arm at
  `ir_control.rs:485-497` — F6 corollary. Extend the comment at `:480-484`.
- Verify by reading that the `debug_assert_ne!` at `:372-377` now agrees: a
  Supported variant must no longer reach `other =>`.

### R3 — Error-IDENTITY fixtures (first-class; four fixtures)

For each terminal, one specimen whose error edge is **TAKEN** in native code,
run twice **in the same test binary**: JIT'd-with-deopt, and never-JIT'd.
Acceptance is IDENTITY — same class, same reason term, same stacktrace shape.
"An error was raised" is not acceptance and must not appear as one.

- **Badmatch** — the discriminating one. Per F4, build the specimen so subject and
  walked tail are DISTINGUISHABLE terms, and assert the reason is
  `{badmatch, <whole subject>}`. This is a restart-fidelity assertion.
- **CaseEnd** — reason `{case_clause, V}` (`exceptions.rs:161`).
- **IfEnd** — reason is the **BARE atom** `if_clause`, never `{if_clause, []}`
  (`exceptions.rs:165-173`, which carries the prior regression in its own comment).
- **Badrecord** — per F2, both arms yield `ExecError::UnsupportedOpcode`. Build it,
  assert the identity, and label it in the report as identity-of-a-non-raise.

Each fixture must make the taken-ness of the error edge legible in its own
assertions — a judge must be able to see that the edge fired, not infer it.

### R3b — The null arm, inside the same run

A poisoned-then-cured function's HAPPY path, byte-identical JIT'd vs interpreted,
measured in the SAME run as its deopt arm. Specimens derived from the census
corpus's own population — `gates.awl` and `investigate.awl` — not synthetic
minimums. Sourcing in `.fleet/legs/build.md` §5.

### R4 — Replay parity across the deopt boundary (first-class)

A workflow-shaped execution that TAKES the error edge, byte-identical interpreted
vs JIT'd-with-deopt. Distinct from R3: R3 compares one function's error; R4
compares a whole replay's output across the boundary. The existing happy-path
parity run does not witness this and may not be substituted for it. If beamr has
an existing replay-parity harness, extend it with an error-edge arm rather than
building a second one; if it does not, the new harness is part of this step and
its shape is reported.

### R5 — The falsifier (beamr-side arm only)

Specimen functions containing case-fallthrough and assert edges compile — the
pre-pass accepts — and their happy paths run native. Report per-function
acceptance, and where a specimen is rejected, the REASON (F6).

The estate-side arm is Calliope's and is **owed, not discharged here**: at the next
beamr pin bump in aion, the census poisoning measurement re-runs at the same
instrument, and 51/9,864 must become 0/9,864 with the 75-walk holding. The report
carries the pointer.

Artemis's stdlib skip count (276/454) is explicitly NOT this brief's acceptance —
stdlib leans on the wave-2 exception machinery. Do not measure it, do not cite it;
conflating the populations manufactures a red.

### R5b — fmt-forced rejoins declared

Every hunk that exists only because rustfmt rejoined lines is declared as its own
category in the flight report, so review reads mechanism changes against mechanism
hunks. Method in `.fleet/legs/build.md` §6.

### R6 — Estate discipline

No suppression in any spelling (`#[ignore]`, `#[allow]`, `#[expect]`,
`#[cfg(any())]` — the gate sweeps for all four). The consistency walk is not
weakened to admit the change. No coverage-table wildcard, ever. Scope stays the
four terminals: `Catch`, `CatchEnd`, `TryCaseEnd`, `Raise`, `RawRaise`,
`BuildStacktrace` stay wave-2 and untouched at `coverage.rs:115-122`.

---

## 4. What the gate PROVES vs what the JUDGE must RE-RUN

### The gate proves, mechanically (`sh .fleet/gate-entry.sh` from the repo root)

- **Environment floors** — ≥20G disk; `cargo`, `jq`, `ast-grep` present; a loud
  note if `wasm-bindgen-test-runner` is absent (that leg then reds as its own finding).
- **Tree receipt** — HEAD sha recorded; working tree CLEAN at gate time
  (named cause `tree_dirty` otherwise).
- **Paths census** — every changed file against `272ca9e` matches
  `^(crates/|conformance/|probes/|\.fleet/)`, and at least one file is under
  `crates/`. This is the wall, enforced (`wall_breach` / `deliverable_absent`).
- **Suppression sweep** — zero added `#[ignore` / `#[allow` / `#[expect` /
  `#[cfg(any()` lines in the `.rs` diff (`suppression_added`). This is R6's
  no-suppression clause, mechanically enforced.
- **The repo's own 9-leg canon**, each leg run exactly as `ci.yml` runs it
  (subshell, redirected not piped, no `set -e` across legs), graded by
  `scripts/ci-verdict.sh` — the one copy of the truth, which additionally keeps
  per-leg exit contracts, distinguishes CANNOT-MEASURE from FAIL, reds a malformed
  rc, and self-tests its own verdict classes before grading:
  `fmt`, `clippy`, `wasm32-check`, `wasm-tests`, `tests`,
  `blocking-call-in-native-bif`, `clippy-all-features`, `tests-all-features`,
  `nostd-ratchet`.
- **Table consistency as a compiled property** — inside the `tests` and
  `tests-all-features` legs, `coverage_table_has_one_entry_per_instruction_variant`
  (`compiler_tests.rs:3834`) and
  `coverage_walk_agrees_with_prepass_and_dispatch_for_all_75_variants` (`:3852`)
  run. Exhaustive-no-wildcard is enforced by the Rust compiler itself: an
  unclassified variant fails to build. The derived Supported count at `:3901`
  fails loudly if the tables and the walk disagree.
- **That the committed fixtures pass.** Not what they assert.

### The judge must re-run and read at the bytes — the gate cannot see these

1. **The four R3 fixtures, both modes.** The gate cannot distinguish a fixture that
   TAKES the error edge from one that never reaches it, nor one asserting class +
   reason term + stacktrace shape from one asserting "an error was raised".
   Read each of the four and confirm: (a) the error edge is forced and its
   taken-ness is asserted, (b) both arms run in the same binary, (c) identity is
   asserted on all three components.
2. **The Badmatch SUBJECT assertion specifically** — reason `{badmatch, <whole subject>}`
   on a specimen where subject and walked tail differ (F4). A green Badmatch
   fixture that cannot tell them apart is worthless here.
3. **The Badrecord fixture's honesty** (F2) — that the report labels it
   identity-of-a-non-raise and does not present it as a raise.
4. **R3b's null arm** — that it rides inside the same run as the deopt arm, and
   that specimens are derived from real `gates.awl` / `investigate.awl`, not
   synthetic minimums.
5. **R4 replay parity across the deopt boundary** — that the execution takes the
   error edge and that outputs are compared byte-for-byte; and that the happy-path
   parity run has not been silently substituted.
6. **Table consistency at the bytes** — read `coverage.rs` and confirm the same
   four variants moved in all three `Instruction` tables, that `is_no_fail_label`
   was left alone with the F1 declaration made rather than a fourth arm invented,
   and that no wildcard arm appeared anywhere.
7. **The lowering is the template** — `dispatch_core.rs` arms are the bare
   `return_status_raw(…DEOPT, …SENTINEL)` shape, with no state marshalling (F5),
   and the four joined `control_flow_successors`' no-successor arm (F6).
8. **R5's poisoned-set measurement is reported with per-function reasons**, so a
   side-effect-guarded rejection is separable from an incomplete fix (F6).
9. **R5b** — the fmt-rejoin category is declared. The `fmt` leg proves formatting
   is correct; it cannot prove the rejoin hunks were disclosed.
10. **R5's estate-side arm is OWED, not discharged.** Nothing in this venue can
    prove 0/9,864. The report carries the pointer; the judge confirms it is
    carried as an obligation and not claimed as evidence.

---

## 5. Walls, restated as build constraints

- Change nothing outside `crates/`, `conformance/`, `probes/`, `.fleet/`.
- Scope is the four error-raising terminals ONLY. The exception machinery stays
  wave-2 and untouched.
- The interpreter remains resident and reachable — the whole deopt contract rests
  on it; F2 is a note about a missing arm, not a licence to add one.
- No coverage-table wildcard, ever.
- No suppression in any spelling.
- Commit `.fleet/` artifacts with the work. Do not push.

The builder's marching orders are `.fleet/legs/build.md`.
