# LEG — build (single builder). BEAMR-R8-DEOPT

Read `.fleet/BRIEF.md` whole and `.fleet/plan.md` whole before touching a file.
The plan's **findings F1-F6** are binding on you: they are the places where the
brief and the tree disagree, already checked at the bytes. Do not re-litigate
them and do not paper over them.

Base `272ca9e034b0f78f50e30ec1068ea1eca967e6d6`. Branch `fleet/beamr-r8-base`.
You change nothing outside `crates/`, `conformance/`, `probes/`, `.fleet/`.
Do not push. Report to `.fleet/reports/build.md`.

First-party fixture note: beamr is our own product, this box is our own fleet
venue, and `ablative-io/aion` is our own public repo. Every corpus document, key
and estate you touch is a first-party local test fixture. Say so in your report.

---

## 0. Before you edit anything

Run the gate once at base:

```sh
sh .fleet/gate-entry.sh
```

**Expect a RED with named cause `deliverable_absent`** — at the rig tip the diff
against base is the three `.fleet/` files, all inside the wall, so the census arm
sees zero files under `crates/` (`gate-entry.sh` §3). That red proves the rig arms
are live. Paste the receipt into your report. **Any other red at base is a rig
finding — report it, do not route around it, do not edit the gate.**

Also record the baseline, which I measured at `3357073f`:

```
cargo test -p beamr --lib --features beamr/encode coverage_
→ 4 passed, 0 failed. Supported count = 60 (compiler_tests.rs:3902).
```

---

## 1. Table changes — `crates/beamr/src/jit/coverage.rs`

Three tables take arms. The fourth does not. This is F1 and it is binding.

### 1a. `coverage` (`:40`)

Remove `Badmatch` / `Badrecord` / `CaseEnd` / `IfEnd` from the
`RejectedIncremental { reason: "wave 2: error-raising terminals" }` arm at
`:123-128` — **delete that arm entirely**, do not leave it empty. Add the four to
the `Supported` arm, in enum order, with a comment in the register of the
`func_info` comment at `:105-108`: reached via a fail edge only; lowered as a
DEOPT terminal; the restarted interpreter raises. Name this brief in the comment.

Leave `:112-114` (`SelectTupleArity`), `:115-122` (the exception machinery) and
`:129-131` (`UpdateRecord`) untouched. They stay wave-2 — that is the Wall.

### 1b. `is_observable_side_effect` (`:167`)

The four are at `:239-242`, on the `false` side. They **stay false** — a terminal
that raises performs no effect a deopt-restart would duplicate; the raise happens
once, in the interpreter, after the restart.

Checked at the bytes: the `false` arm here is one flat `|`-chain running to
`=> false` at `:249` with **no per-group comments inside it**, so there is no
stale grouping to escape and **no arm move is required — expect zero diff in this
table.** That is the correct outcome, not an omission. State the classification
and its reason in your report so R1's consistency claim is discharged explicitly
rather than by silence. (Contrast `is_runtime_deopt_capable` in §1c, where a
grouping comment DOES exist and DOES go stale.)

### 1c. `is_runtime_deopt_capable` (`:291`)

The four are at `:362-365`, under the comment
`-- non-Supported variants: never lowered (pre-pass rejects first) --` (`:354`).
That comment stops being true of them. **Move them to the `true` arm**, beside
`FuncInfo` at `:321`, with a comment matching `:320`.

Read the doc comment at `:280-290` and update the sentence that ends "and every
non-`Supported` variant (never lowered …)" so it no longer claims these four are
non-Supported.

### 1d. `is_no_fail_label` (`:376-378`) — NO EDIT

Per F1 this is a `&Operand` predicate (`matches!(operand, Operand::Label(0))`),
not an `Instruction` table, and cannot classify the four. Its four call sites
(`ir_control.rs:227`, `:409`, `:537`; `dispatch_core.rs:195`) all pass a Bif's
`parsed.fail`.

**Do not** reshape it into an `Instruction` table — that is inventing mechanism,
which R2 forbids in its own words. **Do not** add a wildcard. **Do not** skip R1
silently either. Discharge it by DECLARATION in your report, checked at the bytes:

> `Badmatch`/`Badrecord`/`CaseEnd` each carry exactly one `value: Operand` and
> `IfEnd` carries nothing (`loader/decode/instruction.rs:243-252`) — none of the
> four carries a fail label of its own, so none ever reaches `is_no_fail_label`.
> The module header at `coverage.rs:3-9` over-claims by grouping this `&Operand`
> predicate with the three exhaustive `Instruction` tables; R1's "all four
> classify the four terminals" is not literally dischargeable. Recorded, not
> worked around.

If you believe the header should be corrected to match the code, say so in the
report as a recommendation. **Do not edit the header on your own initiative** —
it is a claim about the tree's own standards and that call is Artemis's.

### 1e. The consistency walk — `crates/beamr/src/jit/compiler/compiler_tests.rs`

Update the derived count at `:3901-3904`, **60 → 64**, and extend the comment at
`:3898-3900` to name this brief and the four terminals. The count stays DERIVED
from the walk — do not turn it into a second literal, and do not weaken the walk
or the representative table at `:3538` to admit the change.

---

## 2. Lowering — `crates/beamr/src/jit/compiler/dispatch_core.rs`

The template is `:423-426`, in full:

```rust
Instruction::FuncInfo { .. } => {
    return_status_raw(builder, JIT_STATUS_DEOPT, JIT_DEOPT_SENTINEL);
    Ok(Some(true))
}
```

Add the four beside it with **that body, verbatim**. One arm is fine if the
comment covers all four; keep it adjacent to the `func_info` arm so the precedent
is legible in the diff.

**Carry no state.** F5: R2's phrase "carrying enough state for the interpreter to
raise the correct error against the correct operand" is satisfied by the RESTART
contract, not by transfer — `JIT_STATUS_DEOPT => return Ok(None)`
(`interpreter/opcodes/core.rs:950`) and the sentinel arm at `:958-960` make the JIT
decline, so the caller re-enters the function from its bytecode entry and the
interpreter recomputes every operand itself. `coverage.rs:149-151` states it:
"deopt restarts the callee interpreted from its start". If you find yourself
marshalling an operand into the deopt, stop — that is the divergence R2 tells you
to record as a finding, not to build.

---

## 3. Pre-pass acceptance + operand validation — `crates/beamr/src/jit/ir_control.rs`

The grill amendment. Three edits, all in this file.

### 3a. Acceptance arms, placed with the `func_info` arm (`:366-368`)

Block-start insertion follows that arm exactly: `block_starts.insert(index + 1);`.
Comment in the same register as `:361-365`.

### 3b. Operand validation — accept, do not accept blind

`Badmatch { value }`, `Badrecord { value }` and `CaseEnd { value }` each validate
`value` through the existing `validate_read_operand` shape — the same call used at
`:345` (`WaitTimeout`'s `timeout`), `:353-354` (`RecvMarkerBind`) and `:358`
(`RecvMarkerClear`/`RecvMarkerUse`). Mirror those shapes; do not invent a new
validator.

`IfEnd` is fieldless (`instruction.rs:252`): block-start insertion only, no
operand to validate.

### 3c. Successors — do not miss this (F6 corollary)

Add the four to the "leaves the function: no in-slice successor" arm at
`:485-497`, where `FuncInfo` sits at `:493`, and extend the comment at `:480-484`
to name them. They terminate the function: a raise never returns to a sibling
instruction by fall-through.

If you skip this, `reject_deopt_after_side_effect` (`:582-616`) propagates taint
through a fall-through edge that does not exist and the guard mis-fires. That is
a silent correctness bug the 75-walk will not catch.

### 3d. Check the assert agrees

After the arms land, the `debug_assert_ne!` at `:372-377` must never fire: a
Supported variant must no longer reach `other =>` at `:369`. Confirm by running
the walk in a debug build (the gate's `tests` leg is a debug build, so this is
covered — but read it and satisfy yourself).

---

## 4. Fixture protocol — R3, R3b, R4

Four error-identity fixtures, one null arm, one replay-parity run. These are
build steps with their own acceptance, not report material.

### 4a. R3 — error IDENTITY, all four

Each specimen's error edge must be **TAKEN in native code**, and each fixture runs
BOTH modes **in the same test binary**: JIT'd-with-deopt, and never-JIT'd.
Acceptance is IDENTITY of **class**, **reason term**, and **stacktrace shape**.

> "An error was raised" is not acceptance. If your assertion would still pass
> against a different reason term, it is not an identity fixture — rewrite it.

Make the taken-ness of the error edge legible in the fixture's own assertions. A
judge must be able to SEE the edge fired, not infer it from a passing test.

Reason terms, from the bytes:

| Terminal | Reason | Source |
|---|---|---|
| `Badmatch` | `{badmatch, V}` | `interpreter/opcodes/exceptions.rs:150` |
| `CaseEnd` | `{case_clause, V}` | `exceptions.rs:161` |
| `IfEnd` | **bare atom** `if_clause` — NEVER `{if_clause, []}` | `exceptions.rs:165-173` |
| `Badrecord` | none — see 4c | — |

`IfEnd` carries a prior regression in its own comment (`exceptions.rs:166-169`):
a tuple-wrapped `{if_clause, []}` does not match `catch error:if_clause` in loaded
bytecode. Assert the bare atom.

### 4b. THE BADMATCH SUBJECT-OPERAND TRAP — read this twice

R3 says the Badmatch specimen must trap on the SUBJECT operand, not the walked
tail. **The naive reading of that is a trap, and F4 is why.**

At both AWL construction sites — `ablative-io/aion`
`crates/aion-awl/src/mir/select/emit/burst.rs:294` and `:393` — the emitted
instruction is identical:

```rust
self.push(Instruction::Badmatch { value: Operand::X(0) });
```

The subject-vs-tail distinction does **not** live in the operand. It lives in the
`Move` immediately before `:393` (`burst.rs:389-392`), which re-points `X0` at the
whole subject from its `Y` home, under the BC-2b-5 comment at `:386-388`.

So: **no lowering can report the walked tail by mis-reading the operand — the
operand is `X(0)` either way.** Recording "the operand was X(0)" proves nothing
and is not this fixture's evidence.

What the fixture actually guards is RESTART FIDELITY: a lowering that lets the
interpreter observe `X0` **without re-executing that `Move`** — i.e. anything that
resumes mid-function or carries native register state across the deopt instead of
restarting from the function entry.

Therefore:

1. Build the specimen so the **subject and the walked tail are distinguishable
   terms** — e.g. a subject list long enough that the tail at the point of failure
   is a proper suffix, not equal to the subject. If your specimen cannot tell them
   apart, the fixture is vacuous.
2. Assert the reason is `{badmatch, <whole subject>}` and **explicitly assert it is
   not** `{badmatch, <tail>}`.
3. State in the report that this is a restart-fidelity assertion and why the
   operand check alone would be vacuous.

### 4c. `Badrecord` — F2, build it and label it honestly

`Instruction::Badrecord` has **no interpreter execution arm**.
`interpreter/opcodes/mod.rs:381-383` dispatches `Badmatch`, `CaseEnd` and `IfEnd`;
there is no `Badrecord` arm, `exceptions.rs` defines no `badrecord` function, and
the variant falls to `other => Err(ExecError::UnsupportedOpcode { … })` at
`interpreter/opcodes/mod.rs:451`.

So the brief's "deopt to the resident interpreter, which raises the error" is
false for this one. Identity still holds — both arms yield the same
`ExecError::UnsupportedOpcode` — so the fixture is buildable.

**Build it. Assert the identity. Then label it in the report, in these words:
identity of a NON-RAISE.** Do not present it as evidence that the terminal raises
correctly.

**Do not implement `badrecord` in the interpreter.** That is outside the brief's
scope and outside the Walls. If you think it should be implemented, say so as a
recommendation in the report.

### 4d. R3b — the null arm rides inside the same run

A poisoned-then-cured function's HAPPY path, byte-identical JIT'd vs interpreted,
measured **in the same run as the deopt arm**. The point is that the cure does not
perturb the path that never traps. A separate happy-path test in a different
binary does not satisfy this.

### 4e. R4 — replay parity across the deopt boundary

A **workflow-shaped** execution that TAKES the error edge, byte-identical
interpreted vs JIT'd-with-deopt. This is distinct from R3: R3 compares one
function's error, R4 compares a whole replay's output across the boundary. The
existing happy-path-only parity run does not witness this and **may not be
substituted for it**.

Look first for an existing replay-parity harness in the tree and extend it with an
error-edge arm. If none exists, building one is part of this step — report its
shape so the judge can assess it.

---

## 5. Corpus specimen sourcing (R3b / R5) — `ablative-io/aion`, public, first-party

The census corpus documents live in our own public repo. Fetch narrowly; **never
commit the foreign repo**, only fixtures you derive. Verified reachable from this
box.

```sh
git clone --filter=blob:none --no-checkout --depth 1 \
  https://github.com/ablative-io/aion.git /tmp/aion-src
cd /tmp/aion-src
git checkout HEAD -- workflows/gates/gates.awl workflows/investigate/investigate.awl
```

Paths confirmed present at the tip of `main`:

- `workflows/gates/gates.awl` — R3b/R5 specimen source
- `workflows/investigate/investigate.awl` — R3b/R5 specimen source
- `crates/aion-awl/src/mir/select/emit/control.rs:134` — the `CaseEnd`
  construction site (SelectVal fall-through trap; `:133` pushes the fail label
  immediately before it)
- `crates/aion-awl/src/mir/select/emit/burst.rs:294`, `:393` — the two `Badmatch`
  construction sites; `:386-392` is the BC-2b-5 subject `Move`

Also present, if you need more of the poisoned population:
`workflows/gates/repo_gates.awl`, `examples/cargo-gates/cargo_gates.awl`,
`examples/remote-gates/remote_gates.awl`.

**Specimens must be DERIVED from these production documents, not synthetic
minimums** — R3b says so explicitly. Commit the derived fixtures under the wall
(`crates/`, `conformance/`, or `probes/`), and record in the report which source
document each specimen came from and what you cut.

---

## 6. R5b — fmt-forced rejoins, declared up front

House rule at the beamr seat. Method:

1. Make your mechanism changes.
2. Run `cargo fmt --all` (the gate's `fmt` leg is `--check`, so the tree must
   already be formatted).
3. Identify every hunk that exists **only** because rustfmt rejoined or re-split
   lines with no mechanism change — the arms you touched in `coverage.rs` will
   produce these, since collapsing a four-variant `RejectedIncremental` arm
   re-flows the surrounding `|`-chains.
4. Declare them as **their own named category** in the flight report, listed
   file-by-file with line ranges, separate from the mechanism hunks.

The purpose is that review reads mechanism changes against mechanism hunks. The
`fmt` gate leg proves formatting is correct; it cannot prove you disclosed which
hunks are formatting-only. That disclosure is yours.

---

## 7. Report — `.fleet/reports/build.md`

Everything the brief's Evidence duties and the staging rig require, plus the
findings:

1. **The coverage-table diff** — all three `Instruction` tables, shown.
2. **The F1 declaration** for `is_no_fail_label`, at the bytes, per §1d.
3. **The 75-walk green**, with the derived count 60 → 64.
4. **The four parity fixtures' transcripts** — error edge TAKEN, both modes, with
   class + reason term + stacktrace shape shown for each.
5. **The Badmatch subject assertion** called out as restart fidelity (§4b), with
   the subject and tail terms shown so the judge can see they differ.
6. **The Badrecord fixture labelled identity-of-a-non-raise** (§4c), with the F2
   citations.
7. **The R3b null-arm run**, showing it rode inside the same run.
8. **The R4 replay-parity run** across the deopt boundary, with the harness shape.
9. **The pre-pass acceptance specimens** (R5 beamr-side arm), reported
   per-function. Where a specimen is REJECTED, give the REASON — never a bare
   count. F6: a function with an observable side effect before its trap edge will
   now be rejected by `reject_deopt_after_side_effect` (`ir_control.rs:582-616`),
   which is a CORRECT rejection, not an incomplete fix, and must be separable in
   your numbers from a classification failure.
10. **The R5b fmt-rejoin category** (§6).
11. **The pointer to the estate-side R5 arm** — Calliope's, owed at the next beamr
    pin bump in aion: the 51/9,864 poisoned set must measure 0/9,864 with the
    75-walk holding. Carry it as an OWED OBLIGATION. Do not claim it as evidence;
    nothing in this venue can prove it.
12. **The gate receipt** — the base red (`deliverable_absent`) and the final green.
13. **Any further drift you find** at the bytes, in the same register as F1-F6.

**Do not measure or cite Artemis's stdlib skip count (276/454).** Stdlib leans on
the wave-2 exception machinery, which is out of scope; conflating the two
populations manufactures a red. The brief says so explicitly.

---

## 8. Closing the leg

```sh
sh .fleet/gate-entry.sh
```

must go **GATE-GREEN**. A red names its cause: **fix the cause, never the gate.**
The gate script is not yours to edit.

Commit your work with the `.fleet/` artifacts. Do not push — the engine handles
the result branch.

### The Walls, one more time

- Nothing outside `crates/`, `conformance/`, `probes/`, `.fleet/`.
- Four terminals ONLY. `Catch`/`CatchEnd`/`TryCaseEnd`/`Raise`/`RawRaise`/
  `BuildStacktrace` stay wave-2, untouched at `coverage.rs:115-122`.
- The interpreter stays resident and reachable. F2 is a note about a missing arm,
  not a licence to add one.
- No coverage-table wildcard, ever.
- No suppression in any spelling — the gate sweeps `#[ignore`, `#[allow`,
  `#[expect`, `#[cfg(any()`.
- The consistency walk is not weakened to admit the change.
