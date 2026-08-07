# RF-006 R6 — stale-`Term` / raw-pointer capture audit

Brief: `docs/design/beamr/briefs/RF-006.json`, requirement R6 (the priority-2
rider). Sweep base: **`afd9257`** (branch `artemis/rf-006`).

Instruments are committed alongside this table under `sweep/` so the numbers
can be re-derived rather than believed: `sweep/r6_sweep.py` (index +
collecting family), `sweep/r6_stage3.py` (capture-side narrowing),
`sweep/candidates.json` (the population), `sweep/family-exclusions.json`,
`sweep/family-owners.txt`.

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

### ⚠️ Two corrections to my own instrument, both found mid-sweep

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

Both corrections moved the answer, so both are stated rather than quietly
absorbed. Residual known to me and **not** yet excluded: four files named
`*_tests.rs` that are declared by neither spelling still contribute **6** of
the sites below; they are in the population and flagged, not silently dropped.

---

## The collecting family — what counts as "a call that can collect"

R6's spec reads *"a call which can reach `gc::ensure_space` (transitively:
…)"*, and the parenthetical is a **closed enumeration of the allocation
primitives**, not an instruction to close a call graph.

That distinction is load-bearing and I got it wrong first. A name-based
transitive closure over this crate marks **2,985 of 5,030 production
functions — 59%** — as "can collect", which is a **ceiling on reachability,
not a measurement of it**, and useless as a discriminator: almost every BIF
eventually allocates. Under the closure the capture sweep returned 1,630
candidate sites. Under the brief's actual closed list it returns 83. ⇒ **AN
OVER-BROAD SEARCH SPACE DOES NOT PRODUCE A CAUTIOUS ANSWER, IT PRODUCES NO
ANSWER** — 59% of a crate is not a finding.

Auto-discovery of `alloc_*` yielded 51 names; **14 are ruled out**, each at
its definition bytes (`sweep/family-exclusions.json`), leaving **37**:

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

## Population — 83 candidate sites

A site is a candidate when a `Term` / `Vec<Term>` / `[Term]` / `*mut u64` /
`*const u64` local is **bound**, a collecting call happens, and **the same
local is read afterwards** inside the same function.

**83 (var, call) sites · 47 distinct functions · 29 distinct files.**

| file | sites |
|---|---|
| `interpreter/opcodes/binary/matching.rs` | 20 |
| `distribution/control.rs` | 7 |
| `native/stdlib_stubs/string_bifs.rs` | 7 |
| `native/stdlib_stubs/uri_bifs.rs` | 5 |
| `native/tcp_bifs.rs` | 5 |
| `native/process_info_bifs.rs` | 4 |
| `jit/runtime_binary_match.rs` | 3 |
| `scheduler/execution.rs` | 3 |
| `beamr-wasm/capability.rs` | 3 |
| `beamr-wasm/convert.rs` | 3 |
| 19 further files | 1–2 each |

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
has been taken yet.** The 83 rows are candidates. That sentence is here so
that "no STOP was raised" is never read as "the sweep found nothing".

---

## Status

**Established and committed:** the search space and its denominators; both
instrument corrections; the collecting family with 14 ruled exclusions and
their reasons; the 83-site candidate population with its full listing; the
four post-fix verdicts for the sites this lane repaired; the AUDIT.md
relationship in both directions; the backport-owned rows and the named
`&'static` launder follow-on; N1 and N2.

**Remaining:** the per-site verdict pass over the 83 candidates —
SAFE-with-immunity-reason or REAL-CROSSING, at per-(site, consumer)
granularity, each taken at the bytes — plus the `stage_pairs` site population
for N2. R6's acceptance A1 is not met until every row carries a verdict, and
this document does not claim otherwise.
