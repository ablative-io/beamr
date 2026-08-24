# BRIEF — beamr: lower the error-raising terminals as DEOPT terminals (R8)

- **Status**: **RATIFIED AS AMENDED by Artemis Peach (beamr seat),
  2026-08-24 — two grill amendments folded in place (R1: all four
  classification tables; R2: pre-pass operand-validation arms). The
  queue entry may flip READY on this commit.**
- **Date**: 2026-08-24
- **Author**: Calliope Tim Tam (AWL seat), from the two measured
  records; every stake below carries its citation.
- **Trace**: Tom 04:45Z (the zero-compile-wake commission) →
  `design/beamr-persisted-native-requirements-20260824.md` R8
  (Artemis, the scoping and blast radius) →
  `design/awl-beam-variant-census-20260824.md` (the stakes) →
  `design/aion-wake-cost-breakdown-20260824.md` (the re-ordering
  that put this brief first).
- **Venue**: the beamr repository, at/after 272ca9e.

## The one-sentence contract

`Badmatch`, `CaseEnd`, `Badrecord`, and `IfEnd` become DEOPT
terminals under the exact treatment `func_info` already receives —
reached only via fail edges, deopt to the resident interpreter,
which raises the error — so that a cold error trap no longer rejects
its entire containing function from JIT/AOT coverage.

## The measured stakes (why this is first)

- beamr's pre-pass rejection is WHOLE-FUNCTION: the walk errs on the
  first non-Supported variant (ir_control.rs:369–381 at 272ca9e,
  Artemis's reading) — no compile-around exists.
- AWL's direct compiler emits exactly 27 variants; the only two
  outside coverage are `Badmatch` and `CaseEnd`, constructed ONLY as
  cold failure edges (census: CaseEnd = the SelectVal fall-through
  trap, emit/control.rs:134; Badmatch = assertion fail edges,
  emit/burst.rs:294/393). They poison 51 of 9,864 corpus functions
  (0.52%) including production documents (gates.awl,
  investigate.awl, the embedded assistant).
- Wake measurement: mid-replay JIT compilation sits on the wake path
  today; this fix plus the existing v1 warmup-set captures the
  compile tax for milliseconds — it is the cheap end of Tom's
  commission and unblocks the entire AWL happy path into coverage
  independent of any persistence work.
- `func_info` is the in-tree precedent: Supported AS a deopt
  terminal, reached only via a fail edge (ir_control.rs:361–368).
  The coverage table's own wave-2 comment already classes these four
  as "error-raising terminals" — the same treatment, named by the
  table itself.

## Requirements

**R1 — Classification (amended at Artemis's grill).** The four
variants move from RejectedIncremental to Supported-as-deopt-terminal
in the coverage table, with the table comment stating the treatment
(reached via fail edge only; deopt; interpreter raises). **coverage.rs
keeps FOUR exhaustive classification tables of record — `coverage`,
`is_observable_side_effect`, `is_runtime_deopt_capable`,
`is_no_fail_label` (its own header, lines 3–9) — and ALL FOUR must
classify the four terminals, consistently: `is_no_fail_label` is
semantically loaded here (these terminals carry no fail label of
their own), and a variant classified in one table but not another is
exactly the divergence the tables exist to make uncompilable.** The
75-variant consistency walk stays exhaustive-with-no-wildcard and
passes.

**R2 — Lowering.** Each terminal lowers as `func_info` does: a deopt
block transfer carrying enough state for the interpreter to raise
the correct error against the correct operand. No new mechanism —
the func_info path is the template; divergence from it is a finding
to record, not a licence to invent. **(Grill addition:) The pre-pass
gains acceptance + operand-validation arms for the four — Badmatch
carries the subject operand that R3's fixture protects, and its arm
mirrors the existing `validate_read_operand` shapes
(ir_control.rs:340–360) rather than accepting blind; block-start
insertion follows the func_info arm (ir_control.rs:366–368).**

**R3 — Error IDENTITY parity (the discriminating fixtures; Artemis's
prior 1, adopted verbatim).** For each of the four: a specimen
function whose error edge is TAKEN in native code must deopt and
raise the IDENTICAL error — same class, same reason term, same
stacktrace shape — as a never-JIT'd run. **Error identity is the
acceptance property; "an error was raised" is not.** The Badmatch
specimen must trap on the SUBJECT operand (AWL deliberately
re-points X0 at the whole subject before trapping — the BC-2b-5
carried fix; a lowering that reports the walked tail instead is the
exact regression this fixture exists to catch).

**R3b — The null arm rides INSIDE the same run (prior 2).** A
poisoned-then-cured function's HAPPY path must produce byte-identical
results JIT'd vs interpreted, measured in the same run as the deopt
arm — the cure must not perturb the path that never traps. Specimens
come from the census corpus's own population — functions from
gates.awl and investigate.awl (production documents in the poisoned
set), not synthetic minimums.

**R4 — Replay parity across the deopt boundary** (Artemis's own
precision, 04:47Z): a workflow-shaped execution that takes the error
edge must produce byte-identical results interpreted vs JIT'd-with-
deopt — the happy-path-only parity run does not witness this.

**R5 — The falsifier, PRE-REGISTERED (prior 3).** Beamr-side:
specimen functions containing case-fallthrough and assert edges
compile (pre-pass accepts) and their happy paths run native.
Estate-side arm (mine, named here so it is owed): at the next beamr
pin bump in aion, I re-run the census poisoning measurement at the
SAME instrument — **the 51/9,864 poisoned set must measure 0/9,864,
and the 75-variant consistency walk must hold. If ANY of the 51
still rejects, the fix is incomplete, not "mostly landed."**
Artemis's stdlib skip count (276/454 at the tax run) is NOT this
brief's acceptance — stdlib leans on the full exception machinery
(Catch/CatchEnd/TryCaseEnd/Raise), which stays wave-2 and out of
scope here; conflating the two populations would manufacture a red.

**R5b — fmt-forced rejoins declared up front** (house rule at the
beamr seat): any hunk in the diff that exists only because rustfmt
rejoined lines is declared as its own category in the flight report,
so review reads mechanism changes against mechanism hunks.

**R6 — Estate discipline.** Whatever beamr's own standards demand at
Artemis's seat, plus: no suppression in any spelling; the consistency
walk is not weakened to admit the change.

## Walls

- **Scope is the four error-raising terminals ONLY.** The exception
  machinery (Catch, CatchEnd, TryCaseEnd, Raise and siblings) is a
  different mechanism (control flow, not terminal traps) and stays
  wave-2; pulling it in here turns a small precedented fix into an
  open-ended one.
- The interpreter remains resident and reachable (the deopt
  contract R1–R9 of the requirements doc depends on it; nothing here
  may assume otherwise).
- No coverage-table wildcard, ever — adding a variant must still
  break compilation until classified.

## Evidence duties

The flight report: the coverage-table diff; the 75-walk green; the
four parity fixtures' transcripts (error edge TAKEN, both modes);
the replay-parity run across the deopt boundary; the pre-pass
acceptance specimens; and the pointer to the estate-side R5 arm as
an owed follow-up at the pin bump.

— Calliope Tim Tam, for Artemis's grill; her grade, her amendments,
her word flips the queue entry READY


---

# STAGING RIG — BEAMR-R8-DEOPT (binding alongside the ratified brief above)

Rig branch fleet/beamr-r8-base, base 272ca9e034b0f78f50e30ec1068ea1eca967e6d6 (beamr main, pinned — the brief's own floor
commit is the tip). Work in your worktree only. The gate at .fleet/gate-entry.sh is
THE gates — its verdict half is the repo's OWN gates.json canon graded by
scripts/ci-verdict.sh; run it via sh from the repo root; a red names its cause, fix
the cause never the gate. Corpus specimen sourcing: the census corpus documents
(gates.awl, investigate.awl) live in the ablative-io/aion repository, which is
PUBLIC and clonable from this box — fetch what R3b needs, commit derived fixtures
under the wall, never the whole foreign repo. Report to .fleet/reports/build.md:
the coverage-table diff, the 75-walk green, the four parity fixtures' transcripts
(error edge TAKEN, both modes), the replay-parity run, the pre-pass acceptance
specimens, the R5b fmt-rejoin category, and the pointer to the estate-side R5 arm
(Calliope's, owed at the next beamr pin bump in aion). Commit .fleet artifacts with
your work — they travel on the fleet ref only. Do not push; the engine handles the
result branch.
