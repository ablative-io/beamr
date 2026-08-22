# BRIEF — beamr: retire the `&'static` accessor exposure at the source

Tom's ruling, 2026-08-22, governs this flight: memory-safety bugs are
unacceptable ANYWHERE, including features anyone considers optional — and
in beamr the "optional" was already a fiction, because `jit` cannot be
disabled in any build that retains `threads`. The project's own README
advisory (2026-08-07) says it plainly: no released version is a clean bill
of health, and the remaining JIT-reachable sites are unfixed in every
release. This flight ends the exposure AT THE SOURCE — the accessor
signatures — not with another call-site sweep.

## The defect, precisely

`BigInt::limbs`, `ProcBin::as_bytes`, `SubBinary::as_bytes`
(`term/boxed/accessors.rs` — audit-time lines 113-118, 281-285, 340-345)
and `shared_binary.rs:79-85` hand out `&'static` slices over heap/`Arc`
memory. Holding one across a GC-triggering allocation dangles. What landed
in 0.16.2/0.16.3 was a call-site sweep of five `string_bifs.rs` BIFs —
each sweep fixes the sites it visits and leaves the signature that
manufactures the next unsound caller. The fix-list's Done criterion offered
two dispositions; Tom's ruling selects the strong one: **slices
lifetime-tied to `&Process`/`&Heap` so the compiler enforces the bound.**
Debug-asserts are not the fix (release builds hold the real data); they may
EXIST as defense-in-depth, but the criterion is compiler enforcement.

Repo: locate the beamr workspace on disk (audit found the checkout 173
commits AHEAD of `main` with `main` an ancestor — **base this flight on
`main`**, the landing target; the divergent working branch belongs to its
owner and re-merging the fix into it is FLAGGED in the report as follow-on
work, not solved here). Audit provenance:
`docs/tracking/fix-list-audit-20260822-beamr-aion.md` item 4 (ablative-docs
`dfe7e3a`) and the README advisory itself.

## R0 — SCOPE. BINDING.

1. **R1 — the design artifact.** Committed BEFORE implementation:
   `docs/design/accessor-lifetimes.md` in-repo, stating (a) the chosen
   lifetime-tying mechanism (borrow the accessor result from `&Process` /
   `&Heap` / the owning guard — whichever the codebase's ownership
   structure supports; name the alternatives you rejected and why), (b) the
   full inventory of the four accessor families' callers (counted, per
   crate, jit included), (c) the API-breakage assessment — which public
   signatures change, what that means for the next version (report the
   fact; version decisions are NOT this flight's), (d) how the GC
   generation/borrow interaction is made unrepresentable rather than
   checked-at-runtime.
2. **R2 — implementation**: the signatures change, every caller compiles
   against the new bounds, no transmutes/laundering anywhere in the path
   (a lifetime fix that launders internally has fixed the signature and
   kept the bug).
3. **R3 — the proof + report** (below).

⛔ NOT in scope: performance work, new features, the 173-ahead branch,
version bumps, changelog, release. A measured red beyond this scope is
reported, not chased.

## R1 note — the red that must exist first

Before the fix: a test (or miri/asan harness run, whichever the codebase
supports — say which) that DEMONSTRATES the dangle: obtain a slice from one
of the four accessors, force a GC-triggering allocation while holding it,
observe the stale read / miri failure. Committed verbatim:
`docs/evidence/beamr-accessor-dangle-red.txt`. After the fix, the same
program must FAIL TO COMPILE (that is the entire point — capture the
compiler error as the green evidence: the bug is now a type error). If a
true runtime repro is impractical for some accessor, say so and show the
miri/borrow-level evidence instead — but do not skip the red silently.

## R3 — REPORT

- The caller inventory re-counted after implementation (zero remaining
  `'static` returns over heap memory in the four families — a sweep
  command with output, not an assertion).
- The compile-fail evidence per accessor family.
- The API-breakage list, and the flagged follow-on: re-merging into the
  divergent working branch (owner's work, named, not done here).
- Every claim OBSERVED or REASONED with `file:line` or command output.

## Process walls — binding

- No lint suppressions in any spelling; no `#[ignore]` in any spelling;
  runtime env-gates only. No file over 500 lines. Never
  `.unwrap()`/`.expect()` in library code. `unsafe` is expected in a VM's
  term internals but every unsafe block this flight touches carries its
  safety comment re-derived against the NEW lifetimes.
- ⛔ Never one blocking sleep sized to expected duration; poll + deadline +
  timeout-as-distinct-outcome.
- Report path per the workflow's wire contract if injected; otherwise
  `.fleet/reports/` with a `git ls-tree` receipt.
- Measured red = success outcome under the same F6-style wall as every
  flight: if the design pass concludes the lifetime tie cannot be done
  without restructuring beyond this scope, that CONCLUSION with its
  evidence is the deliverable — committed, reported, no heroics.


---

# Gate verdict — beamr accessor lifetimes (retire the `&'static` exposure)

2026-08-22 ~23:05Z, Vesper Lynd (gate seat). Brief gated:
`briefs/beamr-accessor-lifetimes.md` at ablative-docs `36990de`. Dispatch
authority verified AT THE DOCUMENT: day file
`tracking/waffles-thread-state-20260805.md`, addendum "⚑⚑ 22:46Z — TOM'S
THREE RULINGS EXECUTED" (commit `b339c54`) — Tom 22:43Z: memory-safety
bugs unacceptable anywhere, jit not optional, GO top priority.

## Verdict: FIT TO FLY on rocketfish (`fleet_dev_rf`) after its whole-chain
## push proof for the beamr repo. Binding notes below.

The design-first shape is right for a change of this blast radius: R1's
design artifact (mechanism + full caller inventory + API-breakage
assessment + how the GC/borrow interaction becomes UNREPRESENTABLE) lands
before any signature moves; the red is a demonstrated dangle and the green
is a COMPILE ERROR — the strongest possible cure evidence, the bug becomes
a type error. The no-laundering clause (a lifetime fix that transmutes
internally has fixed the signature and kept the bug) is the load-bearing
wall; the judge checks it at the artifacts.

## Binding note 1 — sites verified on beamr `origin/main` at this gate

`crates/beamr/src/term/boxed/accessors.rs`: `pub fn limbs(self) ->
&'static [u64]` at :113, `pub fn as_bytes(self) -> &'static [u8]` at :281
(ProcBin) and :340 (SubBinary); `crates/beamr/src/term/shared_binary.rs`:
`bytes_from_raw_word(raw: u64) -> &'static [u8]` at :79. All four
families confirmed as briefed.

**KNOWN ANSWER for the sweep**: accessors.rs ALSO carries
`fn parent_bytes(parent: Term) -> Option<&'static [u8]>` at :357 — a
private helper in the SubBinary family the audit lines did not name. The
R3 sweep criterion (zero remaining `'static` returns over heap memory in
the four families) already covers it; a final sweep that does not list it
as found-and-retired is a failed sweep. The repo path prefix is
`crates/beamr/src/…`, not `src/…` — the leg re-derives all coordinates.

## Binding note 2 — the audit's checkout claim does not describe the
## current local checkout; the flight is unaffected but the follow-on flag
## must be re-derived

The local estate checkout sits on `pr20-numeric-eq`: zero commits ahead of
`origin/main` and main NOT an ancestor — not the audit's "173 ahead with
main an ancestor". The flight clones from GitHub and bases on `main`, so
nothing blocks; but the report's flagged follow-on ("re-merging into the
divergent working branch, owner's work") must name the actual divergent
branch by MEASUREMENT at flight time (`git for-each-ref` + ahead/behind
counts against main), not repeat the audit's stale shape.

## Binding note 3 — the lane boundary, preserved

Tom's ruling covers the WORK. Landing into beamr resolves at landing time:
owner-lands-own-repos — Artemis's seat ratifies the landing (or Tom's word
if her seat stays dark). The flight's result branch is a candidate, never
a landing; nothing merges to beamr main at any seat of mine.

## Binding note 4 — dispatch mechanics (my hand)

- Venue: rocketfish (`fleet_dev_rf`), free since the 65r3 kill (process
  table verified clean at Waffles' hand). One flight per box holds:
  73r3 queues behind this.
- NEW REPO = UNPROVEN PUSH LEG: authenticated `git push --dry-run` on the
  beamr repo under the rocketfish worker's credentials before dispatch.
- The red may want miri/asan; the brief already rules the fallback ("say
  which"; miri/borrow-level evidence acceptable; never skip the red
  silently). I record at dispatch whether miri exists on the venue so the
  choice is informed, not discovered.
- Report path verbatim in ALL prompts; fresh token; the measured-red wall
  + disposition algorithm verbatim in all four prompts; judge carries
  R1-artifact-before-implementation ordering, the no-laundering grep
  (transmute/pointer-cast census over the diff), the compile-fail evidence
  per family, the sweep with :357 as the known answer, and #135
  authorship.
- The standing rf rider: any rf-only red cross-checks on .205 before
  belief.
- `unsafe` blocks the flight touches carry safety comments re-derived
  against the NEW lifetimes — the judge reads them, not just counts them.

— Vesper
