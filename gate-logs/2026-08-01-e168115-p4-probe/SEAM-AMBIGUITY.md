# P4 seam ambiguity — REPORT, not a lane abort

The brief (#62) says of arm B:

> the increment wired at the allocation seam(s) the future wiring brief would
> use … **If the correct seam set is ambiguous at the bytes, STOP AND REPORT the
> candidates — seam choice is a design decision that belongs to the wiring
> brief, and the probe must not quietly make it.**

**The seam set IS ambiguous at e168115.** No production patch was applied.
`p4-production.patch` is therefore 0 bytes — the literal output of
`git --no-pager diff --no-ext-diff` with no production file modified. Per the
ruled rider (Cally), this is a report and P5 proceeded regardless.

## Why it is ambiguous

`decrease_virtual_binary_heap` has three production callers, and every one of
them is a **whole-heap or whole-young-region walk** that sums the bytes of
*every* ProcBin it finds, with no regard for how that ProcBin got onto the heap:

| site | walk |
|---|---|
| `crates/beamr/src/gc/mod.rs:300` (`release_refcounted_resources_in_young`) | young region, unreachable only |
| `crates/beamr/src/gc/mod.rs:333` (`release_all_refcounted_resources`) | whole heap (process terminate) |
| `crates/beamr/src/gc/mod.rs:354` (`release_all_refcounted_resources_in_compacted_sources`) | whole heap, unreachable only |

The decrement side is therefore **total**. For the counter to mean anything,
the increment side must be total too: *every* path that writes a ProcBin onto a
process heap must count it. A partial wiring does not merely under-count — it
drives the counter to zero through `saturating_sub` whenever a ProcBin that was
never counted is released, so the counter degrades in **both** directions.

There is no single choke point. `write_proc_bin`
(`crates/beamr/src/term/shared_binary.rs:93`) is the only writer function, but
it takes `&mut [u64]` — it has no `Process`, by design. The call sites split
into three structurally different groups.

## Candidate seams

### Group 1 — `&mut Process` in scope; the increment is directly expressible

| # | site | function |
|---|---|---|
| 1 | `crates/beamr/src/interpreter/opcodes/binary/construction.rs:145-150` | `finalize_builder` |
| 2 | `crates/beamr/src/interpreter/opcodes/binary/construction/segments.rs:359-364` | `allocate_binary` |
| 3 | `crates/beamr/src/interpreter/opcodes/binary/matching.rs:630-634` | `allocate_binary` |
| 4 | `crates/beamr/src/io/standard_io.rs:282-291` | `heap_alloc_binary` |
| 5 | `crates/beamr/src/jit/runtime_binary_match.rs:252-265` | `allocate_binary` |
| 6 | `crates/beamr/src/scheduler/execution.rs:730-737` (alloc at `:735`) | TCP active-message payload |
| 7 | `crates/beamr/src/scheduler/execution.rs:811-819` (alloc at `:816`) | UDP datagram payload |

The other three `alloc_slice_maybe_refcounted` calls in that file —
`scheduler/execution.rs:726`, `:763`, `:796` — allocate an **FdResource**, not
a binary, and must NOT increment a *binary* heap counter. The decrement side
already makes exactly this discrimination: `gc/mod.rs:289-299` accumulates
bytes only under `BoxedTag::ProcBin` and releases `BoxedTag::FdResource`
without adding to `unreachable_bytes`. Any increment wiring has to reproduce
that split.

A further wrinkle on every Group 1 site: they call `alloc_binary` /
`alloc_binary_word_count`, which produce a ProcBin only when the payload
exceeds `REFC_BINARY_THRESHOLD` (64, `term/shared_binary.rs:18`) and an inline
heap binary otherwise. The increment must be conditional on the threshold, not
on the call — a third thing the wiring brief must specify.

### Group 2 — only `&mut Heap`; NOT expressible without a signature change

| # | site | note |
|---|---|---|
| 8 | `crates/beamr/src/mailbox/mod.rs:477-491` (`copy_proc_bin`) | writes the **receiver's** ProcBin; the receiving `Process` is not in scope |
| 9 | `crates/beamr/src/mailbox/mod.rs:495-502` (`copy_sub_binary`) | same |
| 10 | `crates/beamr/src/distribution/etf.rs:733-746` (`alloc_binary_term`) | also used by the replay loader's **scratch heaps, which have no owning process at all** |

Group 2 is the crux. Message delivery is a normal, high-volume way for a
ProcBin to arrive on a process heap, and the GC release walk *will* decrement
for it. Wiring only Group 1 is not a smaller version of the right answer; it is
a counter that drifts.

### Group 3 — `Process` is optional

| # | site | note |
|---|---|---|
| 11 | `crates/beamr/src/native/context/alloc.rs:138-148` (`Context::alloc_binary`) | detached contexts have no process; cf. the existing `mark_last_allocation_maybe_refcounted` no-op at `:163-169` |

## What the wiring brief has to decide

1. Whether Group 2's signatures change (thread the `Process`/counter through
   `mailbox::copy_*` and `etf::alloc_binary_term`), or whether the counter
   stops being an incremental tally and becomes **derived** from a heap walk.
2. What a heap with **no owning process** (replay scratch heaps) counts as.
3. Whether GC's own promotion copies re-increment. They must **not**: the
   release walks already decrement only the *unreachable* bytes, so survivors
   stay counted — an increment on promotion would double-count.

None of those is a measurement. All three are design decisions. The probe
declined to make them.

## What arm B measured instead

A **seam-neutral** arm: the increment is made from the probe program, at the
probe's own ProcBin allocation, through the already-public
`Process::increase_virtual_binary_heap` (`crates/beamr/src/process/mod.rs:396`).
That answers the question the ordering ruling actually asks — *what does the
never-fired trigger do once the counter is non-zero* — while committing to no
production seam. See `README`.
