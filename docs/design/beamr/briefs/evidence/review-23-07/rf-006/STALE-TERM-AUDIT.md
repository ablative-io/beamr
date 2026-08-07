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
python3 sweep/r6_sweep.py  /tmp/stage1.json direct              # 37-name collecting set
python3 sweep/r6_stage3.py /tmp/stage1.json /tmp/wide.json wide
cmp sweep/candidates.json        /tmp/wide.json                 # byte-identical, 69 sites
python3 sweep/r6_stage3.py /tmp/stage1.json /tmp/narrow.json narrow
cmp sweep/candidates-narrow.json /tmp/narrow.json               # byte-identical, the historical 36
```

`candidates.json` is the current population under the **corrected (wide)**
binder. `candidates-narrow.json` is the pre-C-vi 36, kept so the C-iv and C-v
provenance chain stays checkable against the numbers those corrections quote.

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
> ruling. Verified two-arm when raised: `direct` + the then-current binder
> reproduced the population **byte-identically** (sha256 `5311ab26…`, now
> carried by `candidates-narrow.json`), `closure` still yielded its ceiling.
> Both binder arms are re-verified byte-identical at the current commit.
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

### ⚠️ Seven corrections — six to the instrument, one to my prose about it

C-i to C-iii were found mid-sweep and moved the population. **C-iv, C-v and
C-vi were all found after the population was already committed and
reported** — C-iv in `sweep/` (the committed instrument did not reproduce the
committed output), C-v in this document (a ratio taken across two instrument
generations), and **C-vi in the binder itself**. C-iv and C-v changed what
the 36 was entitled to claim; **C-vi changed the number, 36 → 69.**

**C-i…C-vi I found myself. C-vii I did not** — the lane lead asked a one-line
question about a claim in my summary (*which is the second file?*), and the
answer was that the claim was wrong in both of its halves. It is recorded with
the others, below the A4 STOP, because **a correction found by the reviewer
belongs in the same list as the ones found by the author, marked as such** —
a document that quietly lists only self-caught errors misrepresents how much
of its own accuracy it is responsible for.

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
`*_tests.rs` declared by neither `cfg(test)` spelling still contribute **5**
of the 69 sites below (it was 3 of 36 before the C-vi widening); they are in
the population and flagged, not silently dropped.

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

## Population — 69 candidate sites

A site is a candidate when a `Term` / `Vec<Term>` / `[Term]` / `*mut u64` /
`*const u64` local is **bound**, a collecting call happens, and **the same
local is read afterwards** inside the same function.

**69 (var, call) sites · 51 distinct functions · 33 distinct files**, under
the corrected (wide) binder. The pre-C-vi figure was 36 · 30 · 22; the
file-by-file table below predates the widening and is **not** re-tabulated
here, because the widened listing in `sweep/candidates.json` supersedes it.

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
| corrected (C-i + C-iii), **narrow** binder | 167 | 36 |
| + C-vi, **wide** binder — current | — | **69** |

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

**POPULATION ENUMERATED (2026-08-07) — it is exactly ONE site.**

| axis | count |
|---|---|
| files containing a stack-slot staging primitive | **1** — `jit/ir_map.rs` |
| functions | **1** — `stage_pairs()` |
| staging-primitive occurrences | **2** — `create_sized_stack_slot` (`:230`) + `stack_addr` (`:231`) |

Searched tree-wide, non-test, for `ExplicitSlot` / `StackSlotData` /
`create_sized_stack_slot` / `stack_store` / `stack_addr`. **No other site
stages a term into a JIT stack slot.**

⚠️ **CITE CORRECTION:** this section previously said
`jit/compiler/ir_map.rs`. **The file is `crates/beamr/src/jit/ir_map.rs`** —
there is no `jit/compiler/` path component. Corrected here; the finding is
unchanged.

⭐ **AND THE POPULATION OF ONE IS THE POINT, NOT A DEFLATION OF IT.** A class
with one member is still a class when the mechanism is structural: the
collector cannot see a JIT stack slot **by construction**, so the count is
one only because the codebase stages terms in exactly one place *today*.
**The denominator 0-of-8 is what bounds the risk; the numerator 1 is what
bounds the work.** A second `stage_pairs`-shaped site would be invisible on
arrival and would not trip any existing check.

**Denominator RE-DERIVED at the bytes (2026-08-07), `process/mod.rs:562`.**
`roots_with_live_x` enumerates exactly **8** root kinds — x regs (to
`live_x`), y regs, mailbox scan, current exception (reason + stacktrace),
dictionary (key + value), group leader, `native_roots`, and
`native_continuations`. **Zero are JIT stack slots.**
✅ **Control on that absence:** the enumeration *does* include two non-register
kinds (`native_roots`, `native_continuations`), so it is demonstrably capable
of naming a non-register root — and still names no stack slot. **The absence
is measured, not merely unobserved.**

---

## New-crossing protocol (R6 acceptance A4)

Any NEW real crossing this sweep finds is **STOPped to the domain owner at
discovery, before any fix design** — the lane-3 protocol. RF-006 does not
silently absorb new sites, and a finding does not authorise its own
follow-up.

**Status: an A4 STOP HAS now been raised** — four real crossings, recorded
above and routed to the lane lead before any fix design. The remaining 65
rows are candidates carrying no verdict. The original form of this sentence
read "no new real crossing has been STOPped"; it was true when written and
is now superseded, and it is kept in this shape so that "no STOP was raised"
is never read as "the sweep found nothing".

---

## Verdict pass — all 69 rows, R6 acceptance A1

Every row in `sweep/candidates.json` carries a verdict in
`sweep/verdicts.json`, taken at the bytes at per-(site, consumer)
granularity. **69 of 69 examined: 65 RULED, 4 DEFERRED** with a named
discharge condition (`UNRULED-PRERESERVE` — see below).

| verdict | n | meaning |
|---|---|---|
| **REAL** | **13** | unrooted carrier live across a collecting call — see the STOP below |
| **REAL-OSIRIS** | **1** | same, in `string_bifs.rs`, **another seat's lane** — recorded, not touched |
| SAFE-C | 19 | immediate (small int / atom / NIL / local pid) — cannot move |
| FP-D | 17 | not a `Term` at all (usize, `&str`, `&[u8]`, `&mut [u64]`, a closure) |
| **UNRULED-PRERESERVE** | **4** | **DEFERRED, not cleared** — rests on a word-count arithmetic nobody has audited; discharge condition named below |
| SAFE-ROOTED | 3 | `with_rooted` + `rooted_push`, re-read after the reserve |
| SAFE-DETACHED | 3 | `ProcessContext::new()` — detached contexts never collect |
| SAFE-A | 2 | passed **as an argument** to the collecting allocator, which roots it |
| SAFE-FIXED | 2 | repaired by this lane (H3, H4) |
| FP-E | 2 | bind and use are on mutually exclusive control-flow paths |
| SAFE-OFFHEAP | 1 | `write_float` into a **stack** array, not the process heap |
| SAFE-REREAD | 1 | stashes in `x_reg(0)`, reserves, re-reads and re-derives |
| FP-F | 1 | the "collecting call" is inside a **`SAFETY` comment** |

**The instrument's own false-positive rate is 20/69 (29%)** — FP-D, FP-E and
FP-F together. That is the price of a binder that must not miss things, and
it is the right direction to be wrong in. **C-vi was the wrong direction.**

### 🔴 UNRULED-PRERESERVE — four rows DEFERRED, with a named discharge condition

Four rows would be safe *because* a caller reserved the whole budget up front,
so the allocations that follow cannot collect. That immunity **is only as good
as the word-count calculation** (`info_proplist_heap_words`,
`value_heap_words`, and whatever backs `system_info_bifs.rs:186`). **Nobody
has audited those calculations**, and an under-count would silently re-arm
every one of these sites. ⇒ **A PRERESERVATION IS A PROMISE MADE BY AN
ARITHMETIC EXPRESSION, AND CLEARING A SITE ON IT INHERITS THAT ARITHMETIC.**

**These were originally labelled `SAFE-PRERESERVE` with the assumption written
into the prose. The lane lead ruled that wrong, and the ruling is right:**

> ⭐⭐ **A CLEARANCE THAT RESTS ON AN UNAUDITED COMPUTATION IS NOT A VERDICT,
> IT IS A HYPOTHESIS WEARING A SAFE-SOUNDING LABEL.**

The defect was not the disclosure — the disclosure was there. The defect was
that **the caveat lived in prose and the claim lived in the label.** Prose
fires when somebody reads it; a label fires at every lookup, every
`startswith('SAFE')` filter, every downstream count. The two disagreed, and
the label is what travels. **An undercount would re-arm all four silently —
the exact silent-arm shape this lane exists to hunt, shipped inside the audit
that hunts it.**

**Discharge condition** (per row, carried in `sweep/verdicts.json` as a
`discharge` field — these are cheap, bounded, and *not* done here):

| row | discharge |
|---|---|
| `ets_bifs::bif_info_1` | `info_proplist_heap_words` must count ≥ the words allocated between the reserve at `:561` and the last prereserved call |
| `process_info_bifs::bif_process_info_1` | the word count backing the `:198` reserve must cover every allocation it fronts |
| `process_info_bifs::alloc_monitor_list` | same `:198` budget — this one allocates on *the caller's* reservation |
| `system_info_bifs::bif_memory_0` | the word count backing the `:186` reserve must cover every allocation to the return |

✅ **One axis of this IS discharged, and it came back in the lane lead's
favour to check and mine to keep.** `alloc_monitor_list` is cleared by the
*caller's* budget, so its safety depends on **every** caller reserving — a
different risk from the arithmetic above. Enumerated at the bytes: it is
private with **exactly one caller** (`process_info_bifs.rs:263`, same file).
**Callership axis clean.** The arithmetic axis remains open.

---

## 🔴 A4 STOP — fourteen real crossings, one shape, and C-vi

Raised to the lane lead at discovery (four crossings), per acceptance A4;
the verdict pass then took it to fourteen. **No fix is designed or applied
here**; A4 requires the stop to precede fix design, and these are
pre-existing `native/` defects, not regressions of this lane. **None of them
is in the `0.16.4` backport set (R2 + H3 + H4), which is jit-side.**

| # | site | carrier | crosses | why it is real |
|---|---|---|---|---|
| 1 | `native/code_management_bifs.rs::all_loaded` | `list` | `alloc_tuple` | boxed cons chain in a bare local; **no `with_rooted` in the function**; `alloc_tuple` roots its own arguments only |
| 2 | `native/stdlib_stubs/uri_bifs.rs::bif_uri_string_dissect_query` | `key` | the value-arm `alloc_binary` | boxed binary, live and unrooted, then written into a tuple |
| 3 | same function | `terms` | every allocation in the loop | `Vec<Term>` accumulator; `alloc_list(&terms)` roots the elements **after** they are already stale |
| 4 | `native/udp_bifs.rs::finish_udp_recv` | `ip` | `alloc_binary(datagram)` | `ipv4_tuple` returns `context.alloc_tuple(..)`, so `ip` is **boxed**, not the immediate it looks like |
| 5 | `distribution/control.rs::alloc_spawn_request` | `mfa` | `spawn_options_to_list` | that helper ends in `alloc_list`, so it allocates; `mfa` is not passed to it |
| 6 | `distribution/pg.rs::members` | `terms` | `alloc_external_pid` | external pids are **boxed** (4 words); local pids beside them are immediates |
| 7 | `native/dictionary_bifs.rs::entries_to_list` | `tuples` | `alloc_tuple` | boxed tuples accumulate across further `alloc_tuple` |
| 8 | `native/file_meta_bifs.rs::finish_list_dir` | `terms` | `alloc_binary` | boxed binaries accumulate across further `alloc_binary` |
| 9 | `native/otp_stubs/erlang_stubs.rs::bif_os_getenv_0` | `variables` | `alloc_binary` | one boxed binary per env var, all unrooted |
| 10 | `native/stdlib_stubs/uri_bifs.rs::bif_uri_string_parse` | `values` | `alloc_binary` | `keys` beside it are atoms and safe; `values` are boxed |
| 11 | `term/json.rs::array_to_list_term` | `tail` | `value_to_term` | boxed cons spine live across a recursive builder that allocates |
| 12 | `beamr-wasm/convert.rs::json_value_to_term` | `tail` | recursive self-call | same shape |
| 13 | `beamr-wasm/convert.rs::array_to_term` | `tail` | `value_to_term` | same shape |
| 14 | `native/stdlib_stubs/string_bifs.rs::bif_split` | `terms` | `alloc_binary` | **OSIRIS' LANE — recorded, not touched** |

### ⭐ These are not fourteen defects, they are ONE — the unrooted accumulator

Eleven of the fourteen are the same three lines: **build a term in a loop,
push it into a `Vec<Term>` (or thread it through a `tail`), hand the whole
collection to `alloc_list` / `alloc_tuple` at the end.** Every allocation
after the first can collect, and nothing roots what is already in the
accumulator. The terminal `alloc_list` **does** root its elements — which is
precisely the trap, because **by then it is rooting values that are already
stale, and rooting a stale pointer does not recover it.**

**The codebase already has the right tool and uses it correctly**, in **11
`with_rooted` scopes across 11 distinct functions in 6 files** (non-test,
outside `native/context/`). ⇒ **THIS IS NOT A MISSING FACILITY, IT IS AN
UNEVENLY APPLIED ONE** — which is why a name-based or facility-based search
would never have found it, and why the fix is a sweep rather than a patch.

**And there are TWO correct shapes, not one:**

| shape | how | where |
|---|---|---|
| **S1 — root as you go** | `with_rooted` + `rooted_push` after each allocation + `rooted()` re-read | 8 functions / 4 files: `ets_bifs` ×4, `etf_bifs::segments_to_iolist`, `json_bifs::{parse_array, parse_object}`, `string_bifs::bif_next_grapheme` (**10** `rooted_push` sites — `parse_object` and `bif_next_grapheme` hold two each) |
| **S2 — root up front, reserve once** | `with_rooted` over the whole input, one `ensure_heap_space`, then **only `_prereserved` allocators**, so nothing can collect again | 3 functions / 2 files: `lists_bifs::list_from_vec`, `maps_bifs::{bif_maps_find, make_map_from_entries}` |

S1 is correct when allocation happens *during* accumulation. S2 is correct
when the inputs already exist. **The fourteen are the shape that needs S1 and
got neither.**

⭐ **`lists_bifs::list_from_vec` is the safe twin of defect #11.** It builds a
list by threading a `tail` — the identical shape to
`term/json.rs::array_to_list_term` — and is safe purely because it takes its
elements as an already-complete `&[Term]` and roots them before reserving.
**Note carefully what that does *not* buy: it is a sink, not a cure.** Calling
it with a `Vec` accumulated across allocations inherits exactly the defect,
because the elements went stale before the call. **Any remedy census must
count `list_from_vec` as a sink alongside `alloc_list`/`alloc_tuple`.**

### ⚠️ C-vii — I miscounted the facility, and the lane lead's question found it

I wrote "**nine** other places … two of those sit in files that *also* contain
an unfixed instance." **Both halves were wrong.**

- **The count** was keyed on `rooted_push`, which is S1's marker. It therefore
  **could not see S2 at all** — `lists_bifs` and `maps_bifs` were invisible to
  it, the same instrument-shaped blindness as C-vi, one level up. It also mixed
  units, counting `json_bifs` by push site and `string_bifs` by function.
  **Measured: 11 scopes / 11 functions / 6 files.** My error ran **under**, so
  the facility is *more* widely and correctly used than I claimed — which
  strengthens "unevenly applied", it does not weaken it.
- **"Two files"** is **one file**. Intersecting the 6 facility files with the
  11 files holding a real crossing gives exactly **`string_bifs.rs`**. The
  phantom second was `native/stdlib_stubs/json_bifs.rs` (3 correct uses)
  conflated with `crates/beamr/src/term/json.rs` (1 real crossing) — **two
  different files in two different modules whose basenames both say "json".**
  A coordinate crossing by name, in my own summary prose.

⭐ **The compounding the lane lead predicted is real, and it arrives on a
different axis than either of us expected.** She asked whether the second file
was `ets_bifs.rs`, because a file that both demonstrates the tool and carries
an unruled row is worth stating up front. It is **not** in the overlap — it
holds no unfixed instance. But `ets_bifs.rs` **does** hold 4 of the facility's
11 correct uses **and** `bif_info_1`, one of the four `UNRULED-PRERESERVE`
rows. **So the file that best demonstrates "the tool was right there" is also
a file with a deferred row — via the prereservation axis, not the unfixed-instance
axis.** Stated here rather than discovered later.

### ⚠️ C-vi — a false-negative class, found by row 4

**The sweep did not flag row 4.** In that function it flagged `port`
(`Term::try_small_int(..)` — an immediate, which cannot move and was never at
risk) and stayed silent on `ip`, the boxed carrier two lines above it. It
**reported the safe variable and missed the dangerous one in the same six
lines.**

Cause: the binder recognised a carrier only by an explicit `: Term`
annotation or by an RHS matching a **hard-coded list of constructor
spellings**. Two real shapes match neither:

- **w1 — bound from a domain helper that returns a `Term`**: `ipv4_tuple(..)`,
  `ok_tuple(..)`. Now recognised by return type (**794** such names).
- **w2 — an unannotated `Vec<Term>` accumulator** from `Vec::with_capacity` /
  `Vec::new`, pushed terms and handed to an allocator at the end. Row 3 is one.
  The push can sit far below the bind, so this rule reads the whole body.

**Both are now handled, and the binder mode is an argument** (`wide` default,
`narrow` retained). Result: **36 → 69 sites, 30 → 51 functions, 22 → 33
files** — **33 carriers that were invisible.** Four controls, all passing:

| control | result |
|---|---|
| `narrow` still byte-reproduces the historical 36 | ✅ `candidates-narrow.json` |
| `wide` is a strict **superset** of `narrow` | ✅ 36 ⊂ 69 |
| `finish_udp_recv/ip` visible (w1 positive control) | ✅ |
| `bif_uri_string_dissect_query/terms` visible (w2) | ✅ |
| H3's pre-fix shape still caught (regression control) | ✅ |

⇒ **C-i, C-ii and C-iii every one made the population smaller, and I read that
as the instrument getting sharper.** They removed false positives, so each was
self-confirming. The two-arm control on C-iii proved the tightened binder
still caught H3's *known* shape; **it could not prove it caught shapes I had
not thought of.** ⇒ **A NARROWING THAT ONLY EVER REMOVES FALSE POSITIVES IS
UNFALSIFIED IN THE DIRECTION THAT HURTS** — a false-negative class costs
nothing visible, which is exactly why it survives.

---

## Status

**Established and committed:** the search space and its denominators; the
seven corrections C-i…C-vii (six to the instrument, one to my own summary of
it); the collecting family and its ruled
exclusions; the 69-site population with its full listing, **byte-reproducible
from the committed instrument in both binder arms** (C-iv, C-vi); the four post-fix
verdicts for the sites this lane repaired; the AUDIT.md relationship in both
directions; the backport-owned rows and the named `&'static` launder
follow-on; N1 and N2.

**The verdict pass covers the whole widened population: 69 of 69 rows carry a
verdict** (`sweep/verdicts.json`), at per-(site, consumer) granularity, each
taken at the bytes. **Of those, 65 are RULED and 4 are DEFERRED with a named
discharge condition** (`UNRULED-PRERESERVE`, above).

**⇒ R6 acceptance A1 reads: 65 of 69 ruled, 4 deferred, 0 unexamined.** It
does *not* read "69 of 69 ruled" — that was the original claim, and it rested
on four rows whose clearance was a hypothesis. The lane lead ruled the label
before it reached anyone downstream.

**⚠️ What A1 does NOT mean, even restated.** It means every row in *this*
population has been examined. C-vi and C-vii are the standing reminder that a
population is an instrument's field of view — and that the same blindness
recurs one level up, in the prose an instrument's author writes about it. The
binder now sees two carrier shapes it could not see yesterday; **nothing
proves there is not a third.** The honest claim is *complete over what the
instrument can see, with the blind spots named* — not *complete*.

**Remaining:** the `stage_pairs` site population for N2. The fourteen real
crossings are **ruled out of this document** — the lane lead ruled them a
separate lane with its own brief and acceptance, so that a document carrying
two unrelated defect classes cannot let the smaller ride in on the larger
one's evidence.

`native/stdlib_stubs/string_bifs.rs` is another seat's lane; three
newly-visible sites fall in it and are **recorded, not touched**.
