# AR-1 REMEDY-SHAPE PROPOSAL — r1.0 DRAFT FOR RULING. Nothing here is self-authorizing.

Artemis Peach. Ground: beamr main `639191b` (rows 1 and 3 landed); census `9587d2f`
(ancestor of the gate — row 5's DAG proof); gate `AR-1-LANDING-GATE.md` with
amendments 1–6. **No build starts until the questions at the end are ruled.**

## 1. The tension this proposal exists to resolve

The pre-registered criterion: *a remedy is IN only if the unrooted-accumulator
shape CANNOT BE WRITTEN* — strengthened by Amendment 1 (must cover
`Vec<(Term, Term)>` and tuple-literal pushes, i.e. the shape nobody had named
yet). The census: the only total shape proposed so far — rooted handles in the
sink signatures — touches **205 sites** and is **DEAD by the lane lead's own
pre-set bracket** ("20 ⇒ yes, 200 ⇒ no").

So either a shape exists that satisfies the criterion at a measured cost inside
an acceptable bracket, or the criterion needs an explicit relaxation ruling.
**This proposal claims the former exists and prices it.** The pricing key is the
census's own second number: **only 65 of 205 sink calls are
accumulator-capable** (62 variable + 3 other); **140 pass a fixed literal array
and can never be an accumulator.** A remedy that leaves the 140 textually
untouched and converts only the 65 is a different animal from the dead one.

## 2. Candidate A — rooted handles in the sink signatures. DEAD, listed for completeness

205 sites, ruled out by the bracket set before measurement. Not argued further.

## 3. Candidate B — sealed source-trait at the five collection sinks (the core proposal)

The five collection sinks (`alloc_tuple` 107 · `alloc_list` 77 · `list_from_vec`
11 · `alloc_map` 8 · `alloc_list_with_tail` 2) stop accepting `&[Term]` and
accept a **sealed trait** (working name `TermSource`) implemented for exactly
two things:

1. **`&[Term; N]`** — fixed-size literal arrays. A literal call site
   `context.alloc_tuple(&[a, b])` passes a `&[Term; 2]` *before* slice
   coercion, so **the 140 literal sites are expected to compile UNCHANGED.**
   ⚠️ *That is a claim, not a measurement* — see §7, the compile probe is owed
   before any ruling relies on it. A literal array is constructed in one
   expression with no allocation between element evaluation and the call, so it
   cannot be an across-allocation accumulator.
2. **A rooted-accumulator handle** obtainable **only inside a `with_rooted`
   scope** — the S1 idiom given a type. Accumulate through the handle, hand the
   handle to the sink; the roots live in the scope, so every element survives
   any mid-sequence collection.

A bare `Vec<Term>`, `Vec<(Term, Term)>`, slice, or iterator implements nothing
and **cannot reach a sink in safe code**. That is the operational form of
"cannot be written": the shape can still be *typed into a file*, but it cannot
be *completed* — there is no safe path from an across-allocation accumulator to
an allocation sink except through a construct that roots it.

- **Shape coverage:** S3a (`Vec<Term>`) and S3e (`Vec<(Term, Term)>` /
  tuple-literal pushes) die at the sink boundary — the map/pair sinks get the
  same treatment. S3b (threaded tail) is NOT covered by B — that is Candidate C.
- **Closes the census's own disclosed hole:** the sink census's closed-list
  bound (only direct calls to 37 named primitives are recognisable) means a
  detector-shaped remedy is blind to one level of indirection. **A sink-level
  type wall is not** — indirect callers reach the same five functions, so the
  compiler checks what no sweep can see. This is the strongest argument that B
  beats D on exactly the ground Amendment 1 staked out.
- **Control-fixture survival:** by Amendment 3's constraint 1, the R10 control
  fixtures are local types with their own `alloc_cons`, unreachable by any
  type-level change to `ProcessContext`'s surface — they survive B by
  construction, as do the row-1 BAD/GOOD ordering fixtures (also local types).
- **Row-2 dispositions:** the 17 sites migrate to the rooted accumulator and
  are accounted **(b) STRUCTURALLY-ELIMINATED**, each citing the replacement
  construct at file:line, machine-verified per Amendment 2 — the named
  construct exists (the handle type), so the disposition is checkable, not
  asserted.

**Measured blast radius of B:** 5 sink signatures + 65 accumulator-capable call
sites (62 variable + 3 other, all row-addressable in `sink-census.json`) + 0
expected at the 140 literal sites (probe owed). The 17 defect sites are inside
the 65 — their migration IS the fix, not an extra cost.

## 4. Candidate C — the threaded-tail residue (S3b), because B cannot see it

`list = context.alloc_cons(item, list)?` threads a bare `Term` tail; no
collection crosses the sink boundary, so a source-trait catches nothing.
Census: **10 production `alloc_cons` sites** (6 variable · 4 other — the 4
"other" are 3 JIT runtime sites consing a `Term::small_int(1)` head and
`udp_bifs.rs:473`).

Proposal: the bare public `alloc_cons` goes away; cons-building happens through
(i) the **prereserved form** where the outer reserve is exact and **walled by a
row-3-pattern exactness test** (the whole-call heap delta equals the reserve
recomputed from the same production arithmetic), or (ii) a **rooted list
builder** whose tail lives inside the rooted scope. 10 sites to migrate.
Blast radius small; the shape argument identical to B's.

## 5. Candidate D — patches + detector-as-gate. Available ONLY by explicit criterion relaxation

17 S1 patches + the row-1 ordering detector promoted to a CI leg. This is the
gate's own named fallback ("fourteen patches plus a lint is the honest
answer") — **but Amendment 1 strengthened the criterion on precisely the point
that kills D**: a detector keyed on known shapes is blind to the shape nobody
has named yet, and S3e proved the next shape exists before anyone names it.
D also inherits the closed-list indirection hole (§3). Choosing D is therefore
a RULING that relaxes the criterion, not a design choice I can make.

## 6. A disposition class row 3 opened, proposed for ratification

Row 3's walls make **S2 + exactness wall** a verifiable fix shape for sites
whose inputs pre-exist (no accumulation across allocation): root up front,
reserve exactly once, prereserved allocators only, and a committed test that
reds if the arithmetic under-counts by one word. Proposed as an accepted
disposition for such sites — it is what the four UNRULED-PRERESERVE rows now
are, with the "promise with no wall behind it" objection discharged by the wall.

## 7. Measurements owed BEFORE build, pre-registered here

1. **The zero-change compile probe** for the `&[Term; N]` claim: prototype the
   sealed trait on one sink (`alloc_map`, 8 sites, smallest) and count literal
   sites that compile unchanged. If literal sites need edits, the 140 re-enter
   the blast radius and the bracket question changes — measured before argued.
2. **The migration list**: the 65 sites enumerated by file:line from
   `sink-census.json` r3, split S1-needed vs literal-convertible vs
   S2+wall-eligible (§6).
3. **R10 five-class control re-run** on the prototype — the controls must PASS
   under the remedy, per Amendment 3's fault-domain matching.

## 8. What I ask to be ruled (Cally / Waffles; Tom where it touches the road)

1. **Does B+C satisfy the criterion?** Operational reading proposed: *no safe
   path from an across-allocation accumulator to any allocation sink except
   through a rooted construct.* (`unsafe` and a reimplemented allocator remain
   writable forever; no remedy reaches them.)
2. **The bracket call:** measured cost is 5 signatures + 65 + 10 sites — between
   the pre-set 20 and 200. The bracket's author rules where between them the
   line falls.
3. **Sequencing vs the #193 road:** AR-1 is the standing hypothesis class for
   the unattributed soak death. If the road wants the 17 sites dead FAST,
   patches-first (S1 at the 17, dispositions (a)) then the structural remedy
   behind it is a legitimate ordering — but it must be chosen, not drifted
   into, because the patch set changes the migration list §7.2 measures.
4. **§6 ratified or refused** as a disposition class.

**Until ruled: no sink signature changes, no site migrations, no criterion
movement. The compile probe (§7.1) is the only build-shaped act this proposal
authorizes at my seat, and its output is a measurement, not a patch.**
