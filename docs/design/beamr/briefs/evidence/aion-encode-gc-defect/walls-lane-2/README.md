# AION-ENCODE-GC-DEFECT — walls lane 2 (accounting-sanity walls) run record

**Lane:** AION-ENCODE-GC-DEFECT walls lane 2 (GO from Artemis Peach
2026-07-23 22:53Z on the lane-1 landing, built to her spec of record from
her ~21:50Z DM). Build seat: Osiris Yogo. Domain owner/tear: Artemis Peach.
The walls are permanent in-tree tests in `crates/beamr/src/gc/tests.rs`,
beside the C1 collision wall they compose with; this directory is the
lane's red evidence per the lane-1 mutation discipline.

## The three walls

One wall per release-walk entry point, each on an adversarial heap of live
ProcBins with KNOWN byte totals sitting adjacent to `[false | _]` conses
(the C1 misread shape), in young AND old regions, with `virtual_binary_heap`
asserted at the EXACT expected value after the walk and every ProcBin's
`Arc` accounted exactly via `SharedBinary::ref_count()`:

1. `minor_release_walk_decrements_pacing_by_exactly_the_unreachable_proc_bin_bytes`
   — young walk via `collect_minor`: 100 live + 77 dead bytes → pacing
   lands at exactly 100; live Arc stays at ref 2 across promotion, dead
   Arc released exactly once (ref 1); promoted bytes read back exact.
2. `terminate_release_walk_releases_each_proc_bin_arc_exactly_once_and_zeroes_pacing`
   — full walk via `Process::terminate`: old-region (64 B, promoted behind
   a minor) and young-region (33 B) ProcBins, both live at termination →
   pacing zeroed, each Arc released exactly once.
3. `major_release_walk_accounts_compacted_source_proc_bins_exactly`
   — compacted-sources walk via `collect_major`: live+dead ProcBins in
   BOTH regions (80/55 old, 40/21 young) → pacing lands at exactly 120;
   live Arcs ref 2, dead Arcs ref 1; compacted bytes read back exact.

All three walls are GREEN at unmutated main — no STOP owed. Composition
with the collision wall: that wall pins WHICH allocations the release
walks may visit; these pin WHAT the visit does (exactly-once Arc release,
exact pacing decrement).

## The red (lane-1 discipline: committed diff + observed red, never applied)

`mutations/c1-filter-revert.diff` removes the C1 fix's
`AllocKind::MaybeRefcounted` filter from
`HeapRegion::visit_allocated_boxed_objects` — restoring the 0.16.0
word[0]-inference behavior the production defect shipped with. Observed
red: `runs/c1-filter-revert-red.txt`.

**Face record (per Artemis's spec):** all three walls die by the FATAL
face — SIGSEGV before any accounting assert is reached — because the
adversarial adjacency places a raw-0x19-tagged word (a ProcBin header or
another false-head) at each misread cons's word[2], and `Arc::from_raw`
dereferences a small raw. Same face the parent lane's deployment-path
repro rolled: dense `[false|_]` adjacency rolls the fatal face first. The
survivable face (silent pacing corruption → inexact `virtual_binary_heap`)
is what the exact-value asserts pin if a future regression misreads more
gently. The collision wall reds in the same run on its assert face —
recorded in the run file as the composition cross-check.

## Reproduce the red

```sh
git apply mutations/c1-filter-revert.diff
cargo test -p beamr --lib release_walk   # SIGSEGV (fatal face)
git checkout -- crates/beamr/src/process/heap.rs
```

## Environment (recorded at run time, 2026-07-24 AEST)

- macOS 26.5.2 (25F84), Darwin 25.5.0, Apple M5 Pro (arm64)
- rustc 1.95.0, cargo 1.95.0; branch `lane/encode-walls-2` off main `928f886`
- Gates battery evidence rides the branch as its own forward commit per
  doctrine (see `gate-logs/` at the branch tip).
