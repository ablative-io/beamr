# RF-006 R6 — stale-`Term` / raw-pointer capture audit

Brief: `docs/design/beamr/briefs/RF-006.json`, requirement R6 (the priority-2
rider). Sweep base: **`afd9257`** (branch `artemis/rf-006`).

Instruments are committed alongside this table under `sweep/` so the numbers
can be re-derived rather than believed: `sweep/r6_sweep.py` (index +
collecting family), `sweep/r6_stage3.py` (capture-side narrowing),
`sweep/candidates.json` (the population), `sweep/family-exclusions.json`,
`sweep/family-owners.txt`.

Re-derive with:

```sh
python3 sweep/r6_sweep.py /tmp/stage1.json direct   # 37-name collecting set
python3 sweep/r6_stage3.py /tmp/stage1.json /tmp/candidates.json
cmp sweep/candidates.json /tmp/candidates.json      # byte-identical
```

> **Correction C-iv — until this commit, the committed instrument did not
> reproduce the committed output, and the sentence above was false.**
> `r6_sweep.py` always ran the transitive fixpoint and never read
> `family-exclusions.json`; the closed-list reading and the 14 family
> rulings were applied by **hand-editing the script between runs**, and the
> edited version was never committed. A reader running the committed bytes
> got **1,630** candidates, not the committed **36**, and nothing in the pack
> said so. The mode is now an argument (`direct` | `closure`, defaulting to
> the reading R6 specifies) and the exclusions are loaded from their
> committed record, with a **positive control that exits non-zero if any
> excluded name has left the tree** — an exclusion that no longer matches
> anything is a silent no-op, and a typo is indistinguishable from a correct
> ruling. Verified two-arm at this commit: `direct` reproduces
> `candidates.json` **byte-identically** (sha256
> `5311ab26…`), `closure` still yields the 167-site ceiling.
> ⇒ **"THE INSTRUMENTS ARE COMMITTED" IS A CLAIM ABOUT THE FILES, NOT ABOUT
> REPRODUCIBILITY** — the only proof is running the committed bytes and
> diffing against the committed output. This is C-ii's shape again at one
> remove: the correction lived in my shell history instead of the artifact.

---

## Relationship to AUDIT.md — stated in both directions

`AUDIT.md`
(`evidence/aion-encode-gc-defect/asbytes-sweep/AUDIT.md`) swept
**`as_bytes`-derived SLICE borrows**: 68 sites, 11 real, now 12 per R5's
AMENDMENT 1. This rider sweeps **stale `Term` and raw-pointer captures**.

**The two are complementary and neither subsumes the other:**

- **AUDIT.md does not cover this shape.** H3 (`jit_bs_start_match`) and H4
  (`write_map_entries`) are stale-`Term` captures with no `as_bytes` borrow
  anywhere in them, and they appear **nowhere in AUDIT.md's 68**. A complete
  slice-borrow sweep found neither, because neither is a slice borrow.
- **This rider does not cover that shape.** A borrow of `as_bytes()` held
  across a collection is invisible to a sweep that looks for `Term` locals;
  the carrier is a `&[u8]`, not a `Term`.
- **H3-b is in BOTH shapes**, and that overlap is the whole lesson.
  `jit_bs_get_binary` held an `as_bytes`-derived slice *and* a pre-allocation
  source-`Term` capture. AUDIT.md **found the site** — and **mis-verdicted the
  consumer**, because it assessed the slice borrow and stopped. ⇒ **A SITE CAN
  BE FOUND AND STILL BE WRONGLY CLEARED IF THE SWEEP THAT FOUND IT ONLY KNOWS
  ONE OF ITS HAZARDS.** That is why R6's acceptance demands
  per-(site, consumer) granularity rather than per-site.

---

## Search space, stated as a denominator

| axis | count |
|---|---|
| `.rs` files in `crates/beamr/src` + `crates/beamr-wasm/src` | **333** |
| lines (beamr 147,163 / beamr-wasm 10,882) | **158,045** |
| `fn` definitions indexed, all | **7,317** |
| — in `cfg(test)` | **2,287** |
| — **production** | **5,030** |

### ⚠️ Five corrections to my own instrument, all found by me

C-i to C-iii were found mid-sweep and moved the population. **C-iv and C-v
were found after the population was already committed and reported** — C-iv
in `sweep/` (the committed instrument did not reproduce the committed output)
and C-v in this document (a ratio taken across two instrument generations).
Neither changed the 36; both changed what the 36 is entitled to claim.

**C-i — the whole-file `cfg(test)` spelling.** The first index counted only
inline `#[cfg(test)] mod x { … }`. beamr also uses `#[cfg(test)] mod x;` with
the tests in their own file — **112 of the first spelling, 39 of the second**.
Missing the second put **44 whole test files** into the production population:
`cfg(test)` fns read **1,013** when the true figure is **2,287**, and
production read **6,304** against a true **5,030**. A 20% error in the
denominator from one unrecognised spelling. ⇒ **A COUNT OF ONE SPELLING IS NOT
A COUNT OF THE THING.**

**C-ii — the same defect, surviving its own fix.** The repair was applied to
the indexer's *caller* in `r6_sweep.py`; `r6_stage3.py` calls the same indexer
and did its own span-only filtering, so it kept every test file. That showed
up as `gc/tests.rs` contributing 30 candidate sites. ⇒ **A CORRECTION APPLIED
AT ONE CALL SITE OF A SHARED INSTRUMENT LEAVES THE OTHER CALL SITES
UNCORRECTED** — and the second one looks fixed, because the first one is.

**C-iii — the unchecked destructuring arm.** The binder registered *every*
name in a destructuring `let` as a term carrier, with **no type or
initialiser check at all**. So

```rust
let (fail, source, destination) = match (op, operands) { … };
```

put three `&Operand` references into the population — and instruction
operands are **module-resident**: they never move and never need rooting.
That single unchecked arm accounted for **all 20** sites in
`interpreter/opcodes/binary/matching.rs` and **47 of 83** overall. Requiring
a destructuring bind to also look like it carries terms takes the population
from **83 → 36**.

> **Two-arm control on the tightening**, because narrowing an instrument is
> exactly when it goes blind: the tightened binder must still see a KNOWN
> positive. It does — `jit_bs_start_match` `source`, bound at `:19` by
> `let source = Term::from_raw(binary);`, crossing `:30`, used at `:34`. That
> is H3's pre-fix shape verbatim, and it survives the tightening because it
> arrives through the typed-`let` arm rather than the destructuring one.
> ⇒ **A NARROWED INSTRUMENT IS ONLY NARROWER IF IT STILL CATCHES WHAT IT
> CAUGHT BEFORE**, and that has to be shown, not assumed.

All three corrections moved the answer, so all three are stated rather than
quietly absorbed. Residual known to me and **not** yet excluded: files named
`*_tests.rs` declared by neither `cfg(test)` spelling still contribute **3**
of the sites below; they are in the population and flagged, not silently
dropped.

### The operand immunity class, established by C-iii

Ruling out those 20 sites is not bookkeeping — it names an immunity class the
brief's other classes did not cover:

**Operand-carried, re-read after reserve.** The interpreter's binary opcodes
reserve *first* and then read the term *from its bytecode operand*:

```rust
gc::ensure_space(process, MATCH_CONTEXT_WORDS, 256)…;
let source = core::read_term(process, module, source)?;   // AFTER the reserve
```

The carrier across the collection is an `&Operand`, not a `Term`. **This is
the capability `put_map` has and a JIT helper does not** — which is precisely
why R4's "mirror `put_map`" spec was unrealisable, and why R4 depends on R2.
The same shape is safe in the interpreter and impossible in the JIT, so the
class has to be stated in terms of the *mechanism* rather than the *shape*.

---

## The collecting family — what counts as "a call that can collect"

R6's spec reads *"a call which can reach `gc::ensure_space` (transitively:
…)"*, and the parenthetical is a **closed enumeration of the allocation
primitives**, not an instruction to close a call graph.

That distinction is load-bearing and I got it wrong first. A name-based
transitive closure over this crate marks **2,985 of 5,030 production
functions — 59%** — as "can collect", which is a **ceiling on reachability,
not a measurement of it**, and useless as a discriminator: almost every BIF
eventually allocates.

Measured like-for-like on the **corrected** instrument, the closed-list
reading narrows **167 → 36 candidate sites (4.6×)**; the closure marks
**2,983 of 5,030 production functions — 59%** — as "can collect". ⇒ **AN
OVER-BROAD SEARCH SPACE DOES NOT PRODUCE A CAUTIOUS ANSWER, IT PRODUCES NO
ANSWER** — 59% of a crate is not a finding, and it would have shipped looking
like rigour.

> **Correction C-v — the 4.6× above replaces a 10.3× I previously reported,
> and the earlier figure was not a like-for-like comparison.** This paragraph
> used to read *"holding every other setting fixed, the sweep returns 1,630
> under the closure against 158 under the closed list"*. **The settings were
> not held fixed.** 1,630 and 158 were both measured **before** corrections
> C-i and C-iii existed; 36 was measured after. Comparing them credits the
> spec-reading with a narrowing that C-i and C-iii did most of the work for.
> On one instrument the split is 167 → 36. ⇒ **A RATIO BETWEEN TWO NUMBERS
> TAKEN FROM DIFFERENT GENERATIONS OF AN INSTRUMENT MEASURES THE EDITS, NOT
> THE VARIABLE** — and it flatters whichever change you were writing up.
> Both readings still nest correctly on the current instrument
> (36 ⊆ 167 ⊆ 1,630 keyed on `(file, fn, var)`), which is what makes the
> generation the only difference.

The collecting set is built as: auto-discovered `alloc_*` (non-`_prereserved`,
non-test) **43**, plus the **5** seed names not already in it, minus the
hand-ruled exclusions that are still in the set, leaving **37**. Each
exclusion was ruled at its definition bytes (`sweep/family-exclusions.json`):

| step | count |
|---|---|
| `alloc_*` family (non-`_prereserved`, non-test) | 43 |
| + seed names not already present | +5 → 48 |
| − hand-ruled exclusions **that bite** | −11 → **37** |

**11 of the 14 listed exclusions bite; 3 never reach the set** — `alloc_floats`
and two `#[test]` fn names live in whole-`cfg(test)` files, so the C-i filter
removes them before the family is formed. They stay in the record because a
ruling that is currently redundant is not a ruling that is wrong, and the
C-iv positive control now fails loudly if any of them leaves the tree
entirely. (An earlier draft of this section said *"auto-discovery yielded 51
names; 14 are ruled out, leaving 37"* — arithmetically tidy, and not what the
instrument does: 51 was never measured and the 14 are not all subtractions.)

| class | members | reason |
|---|---|---|
| **bump-only** | `Heap::{alloc_old, alloc_in_region, alloc_in_region_maybe_refcounted, alloc_maybe_refcounted, alloc_slice, alloc_slice_maybe_refcounted}`, `HeapRegion::{alloc_slice, alloc_with_kind, alloc_slice_with_kind}` | the brief's named immunity class — cannot collect, never moves existing data |
| **not an allocator** | `alloc_binary_word_count` (`const fn → usize`), `WasmScheduler::alloc_pid` (`→ u64`, mints a number) | name-matched into the family; neither touches a heap |
| **test-only** | `alloc_young_tuple`, `alloc_floats`, and two `#[test]` fn names that begin `alloc_` | not production |

`alloc_binary_word_count` is the one worth naming: it is a **size
calculation** that a name-based family pulled in as an allocator, and it
appears at the head of several real allocation sequences, so it would have
made every one of them look like it collected one line earlier than it does.

---

## Population — 36 candidate sites

A site is a candidate when a `Term` / `Vec<Term>` / `[Term]` / `*mut u64` /
`*const u64` local is **bound**, a collecting call happens, and **the same
local is read afterwards** inside the same function.

**36 (var, call) sites · 30 distinct functions · 22 distinct files.**

| file | sites |
|---|---|
| `distribution/control.rs` | 4 |
| `jit/runtime_binary_match.rs` | 3 |
| `scheduler/execution.rs` | 3 |
| `beamr-wasm/capability.rs` | 3 |
| `beamr-wasm/convert.rs` | 3 |
| `native/stdlib_stubs/string_bifs.rs` | 2 |
| `native/stdlib_stubs/uri_bifs.rs` | 2 |
| `beamr-wasm/capability_tests.rs` | 2 |
| 14 further files | 1 each |

Narrowing walk. The honest form of this is **two axes, not one chain** — the
figures below were taken across three generations of the instrument, and only
the last row is a comparison at fixed settings (see C-v):

| instrument generation | closure reading | closed-list reading |
|---|---|---|
| as first written (pre C-i, pre C-iii) | 1,630 | 158 |
| + C-i, whole-file `cfg(test)` in both scripts | — | 83 |
| **corrected (C-i + C-iii), current** | **167** | **36** |

Read down a column for what the instrument corrections were worth; read
across the bottom row for what the spec-reading is worth (**4.6×**). Reading
diagonally — 1,630 → 36 — is the mistake C-v records.

The full list with file, function, variable, bind line, collecting call line,
callee name and first post-call use is `sweep/candidates.json`.

**This is a candidate population, not a verdict set.** The collecting family
is deliberately over-inclusive in the safe direction, and a candidate is only
a real crossing if the carrier is **boxed** (immediates never move) and
**not** register- or `native_roots`-resident. Per-site verdicts are the
remaining work of this requirement and are **not** claimed here — see
"Status" below. Recording a population without its verdicts is the honest
half-way point; recording verdicts I have not taken would not be.

---

## Sites this brief repaired — post-fix verdicts (R6 acceptance A5)

The record is true at the landing head, not at the start of the lane.

| site | consumer | pre-fix | post-fix verdict | wall |
|---|---|---|---|---|
| `jit/runtime.rs` `alloc_words_rooted` | — | did not exist | **SAFE by construction** — roots, allocates, reads back before truncating, and **refuses** rather than returning an unrecoverable term | `rooting_tests` ×5 |
| `jit/runtime_binary_match.rs` `jit_bs_start_match` | H3 (D2) | **REAL CROSSING** — capture at entry, `alloc_words` collects, pre-move term written to `heap[3]` | **SAFE** — source rooted, forwarded value written | W1 |
| `jit/runtime_binary_match.rs` `jit_bs_get_binary` ProcBin arm | H3-b sibling | **DISCHARGED BY DELETION** — the arm was removed, and its removal *is* the O(1)→O(m) regression | **SAFE** — arm restored, parent rooted, forwarded box written | W4 |
| `jit/runtime_map.rs` `write_map_entries` | H4 (D4) | **REAL CROSSING** — keys/values copied into Rust-owned vectors before a collecting reservation | **SAFE** — every key and value rooted, forwarded values written | W3 |

All four verdicts are backed by observed red at a named commit and by
mutation evidence (`mutations/`), not by inspection.

---

## AUDIT.md's eleven — OWNED BY THE 0.16.3 BACKPORT, no fix motion here

The five `native/stdlib_stubs/string_bifs.rs` sites and the other six of
AUDIT.md's eleven are **out of scope for this lane by ruling**, not by
oversight. `string_bifs.rs` is at **zero diff lines** against the lane base
and stays that way. It contributes **7** rows to the population above and
every one of them is marked **OWNED-BY-0.16.3-BACKPORT**.

### Named follow-on, explicitly NOT actioned here

**The `binary_bytes` / `utf8_str` `&'static` launder**
(`string_bifs.rs:335-343`). The backport's own fix shape leaves this
standing: the helpers hand back a `&'static [u8]` / `&'static str` derived
from heap bytes that are **not** `'static`. The fix repairs the *callers*;
the *launder* remains, and **it re-arms the trap for the next author** —
whoever writes the twelfth caller inherits a signature that promises a
lifetime the data does not have. Recorded as a follow-on with its rationale,
owned by neither this lane nor the backport, and **not actioned here**.

---

## Two findings that belong in this table and are NOT defects

Promoted to rows in their own right, because filing them as footnotes is how
they get read as small.

### N1 — JIT safepoint stack maps are produced and never consumed

`SafepointBuilder` records allocation-site roots, `StackMapEntry` /
`RootLocation` carry them, and the AOT path serialises them. But
`stack_maps()` and `RootLocation` have **zero** references outside `jit/`
(control: 29 references unfiltered). **The collector never reads them.**

Not a defect: nothing is wrong today, because nothing depends on them. The
exposure is one of **reading**: a future author finds a builder, a type and a
serialiser, and concludes the JIT has root tracking. ⇒ **A PRODUCER WITH NO
CONSUMER IS NOT A MECHANISM** — and it is indistinguishable from one until
you go looking for the read.

### N2 — the `stage_pairs` staging class, with a population and a denominator

`jit/compiler/ir_map.rs` stages pair terms into a Cranelift `ExplicitSlot`.
`Process::roots_with_live_x` enumerates x registers, y registers, mailbox,
exception roots, the process dictionary, group leader, native roots and
continuations — **and no JIT stack slot**.

So *every* instance of this staging shape is invisible to the collector **by
construction**, not by accident. That makes it a **class**, and a class needs
a denominator rather than a single-site note:

| axis | count |
|---|---|
| root kinds enumerated by `roots_with_live_x` | **8** |
| of those that are JIT stack slots | **0** |
| enumerated-root kinds a `ExplicitSlot` staging could hide behind | **0** |

The population of staging sites is **not yet enumerated** — that count is
part of the remaining per-site pass, and stating the class without it would
be the "single-site note" this row exists to avoid. What *is* established is
the denominator that makes the class real: **0 of 8**.

---

## New-crossing protocol (R6 acceptance A4)

Any NEW real crossing this sweep finds is **STOPped to the domain owner at
discovery, before any fix design** — the lane-3 protocol. RF-006 does not
silently absorb new sites, and a finding does not authorise its own
follow-up.

**Status: no new real crossing has been STOPped, because no per-site verdict
has been taken yet.** The 36 rows are candidates. That sentence is here so
that "no STOP was raised" is never read as "the sweep found nothing".

---

## 🔴 A4 STOP — four real crossings, and C-vi

Raised to the lane lead at discovery, per acceptance A4. **No fix is designed
or applied here**; A4 requires the stop to precede fix design, and these are
pre-existing `native/` defects, not regressions of this lane. **None of them
is in the `0.16.4` backport set (R2 + H3 + H4), which is jit-side.**

| # | site | carrier | crosses | why it is real |
|---|---|---|---|---|
| 1 | `native/code_management_bifs.rs::all_loaded` | `list` | `alloc_tuple` | boxed cons chain in a bare local; **no `with_rooted` in the function**; `alloc_tuple` roots its own arguments only |
| 2 | `native/stdlib_stubs/uri_bifs.rs::bif_uri_string_dissect_query` | `key` | the value-arm `alloc_binary` | boxed binary, live and unrooted, then written into a tuple |
| 3 | same function | `terms` | every allocation in the loop | `Vec<Term>` accumulator; `alloc_list(&terms)` roots the elements **after** they are already stale |
| 4 | `native/udp_bifs.rs::finish_udp_recv` | `ip` | `alloc_binary(datagram)` | `ipv4_tuple` returns `context.alloc_tuple(..)`, so `ip` is **boxed**, not the immediate it looks like |

### ⚠️ C-vi — a false-negative class, found by row 4

**The sweep did not flag row 4.** In that function it flagged `port`
(`Term::try_small_int(..)` — an immediate, which cannot move and was never at
risk) and stayed silent on `ip`, the boxed carrier two lines above it. It
**reported the safe variable and missed the dangerous one in the same six
lines.**

Cause: the binder recognises a carrier only by an explicit `: Term`
annotation or by an RHS matching a **hard-coded list of constructor
spellings**. A local bound from a **domain helper that returns a `Term`** —
`ipv4_tuple(..)`, `ok_tuple(..)` — matches neither. Adding one rule (RHS calls
a function whose signature returns a `Term`; **794** such names) makes **+19
sites across 15 functions** visible, with the baseline still reproducing
**exactly 36** and `finish_udp_recv/ip` as the positive control. A **second
sub-shape remains unhandled**: an unannotated `Vec<Term>` accumulator from
`Vec::with_capacity` (row 3 is one, and the widening does not see it either).

⇒ **C-i, C-ii and C-iii every one made the population smaller, and I read that
as the instrument getting sharper.** They removed false positives, so each was
self-confirming. The two-arm control on C-iii proved the tightened binder
still caught H3's *known* shape; **it could not prove it caught shapes I had
not thought of.** ⇒ **A NARROWING THAT ONLY EVER REMOVES FALSE POSITIVES IS
UNFALSIFIED IN THE DIRECTION THAT HURTS** — a false-negative class costs
nothing visible, which is exactly why it survives.

---

## Status

**Established and committed:** the search space and its denominators; the five
instrument corrections C-i…C-v; the collecting family and its ruled
exclusions; the 36-site population with its full listing, now
**byte-reproducible from the committed instrument** (C-iv); the four post-fix
verdicts for the sites this lane repaired; the AUDIT.md relationship in both
directions; the backport-owned rows and the named `&'static` launder
follow-on; N1 and N2.

**⚠️ The 36 is NOT a sound denominator, and this document no longer offers it
as one.** C-vi shows it is the population the binder *could see*, not the
population of candidate carriers. Per-site verdicts over it would have been 36
correct verdicts over an unsound denominator — and would have read as
completeness.

**Remaining, in this order:** (1) the lane lead's ruling on the A4 stop —
whether the four crossings become RF-006 rows or a separate lane, and whether
the binder is widened before the verdict pass; (2) the widened re-run;
(3) the per-site verdict pass, SAFE-with-immunity-reason or REAL-CROSSING at
per-(site, consumer) granularity, each at the bytes; (4) the `stage_pairs`
site population for N2. **R6 acceptance A1 is not met**, and the gap is now
larger than it was when this document was first written.

`native/stdlib_stubs/string_bifs.rs` is another seat's lane; three
newly-visible sites fall in it and are **recorded, not touched**.
