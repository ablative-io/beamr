# AION-ENCODE-GC-DEFECT — 0.16.3 backport, lane 3: jit site 12 + sibling

**Arc/base/tear:** see `../lane-1-mechanical/README.md` — same branch off
`67f89c4`, lane 3 of the lane-at-a-time sequence, extending the lane-2
span (head `9a19961`). Scope source: AUDIT.md **AMENDMENT 1** (beamr main
`7e56073`), ruled into the 0.16.3 backport by the 2026-07-28 dispatch
because aion builds beamr with the `jit` feature ON (default features).
Amendment chain re-verified at the bytes at this base before build.

## The sites (`jit/runtime_binary_match.rs`)

- **Site 12, inline arm:** `jit_bs_get_binary` takes
  `bytes = context.slice(bits)` — a borrow of the source binary's bytes —
  then `allocate_extracted_binary` → non-ProcBin arm → `allocate_binary`
  → `alloc_words` → `gc::ensure_space` (COLLECTING) → the copy reads the
  stale borrow. Inline sources (≤ 64 B, exactly this arm) are moved and
  zero-filled: output built from zeros. This lane carries the site's
  FIRST dynamic probe (the amendment's verification was
  static-at-the-bytes).
- **Sibling, ProcBin arm:** captures `source = context.source_term()`
  BEFORE `alloc_words(SUB_BINARY_WORDS)` and writes the possibly-stale
  Term into the new sub-binary after — the ProcBin BOX moves under
  collection even though its bytes are off-heap. The result references
  the zeroed old young region.

## Walls (red-first, per consumer, real geometry)

- `bs_get_binary_inline_source_survives_forced_collection` — inline 40-B
  source, match started, nursery forced so the 20-byte extraction's
  allocation must collect; asserts `old_used() > 0` and exact bytes
  `[1..=20]`.
- `bs_get_binary_procbin_source_box_referent_survives_forced_collection`
  — 100-B ProcBin source (box on young heap, bytes off-heap), nursery
  forced so the `SUB_BINARY_WORDS` allocation must collect; asserts the
  extraction still reaches live parent bytes (the box REFERENT) — a stale
  pre-alloc Term capture leaves it referencing zeroed memory.

Both walls root the source in X0 and the match context in X1 — the jit
allocation path roots via `ensure_space(process, words, 256)` (all
x-registers), and the GC traces MatchContext boxes' source word
(`gc/mod.rs` rewrite of word 3), so the walls' geometry exercises exactly
the helper-local staleness, not un-rooted test artifacts.

## Discoveries reported before the fix (routed, NOT fixed in this lane)

Reported to the domain owner at discovery (2026-07-28), recommendation
RF-006, before any fix byte:

- **D1:** `jit_bs_start_match` captures `source` before its own
  collecting `alloc_words(4)` and writes it into the new match-context
  box after — the sibling's exact class. Escaped the audit because the
  enumeration criterion was `as_bytes` callers and `start_match` never
  calls `as_bytes`.
- **D2 — UPGRADED AND FIXED IN-LANE:** `jit_bs_get_binary`'s post-alloc
  `context.set_position_bits` writes through the PRE-collection box
  pointer. Originally read as a lost position update and routed; the
  ProcBin wall then OBSERVED it at the bytes
  (`runs/red-d2-stale-position-write.txt`): with the own-then-allocate
  fix applied and the write-back still post-alloc, the wall reds with a
  single-byte read-modify-write corruption — `left[16] = 176` where 16
  was expected, exactly `0x10 + 0xA0`: the stale pointer's word-1
  read-modify-write (`position + 160`) landed inside the freshly
  reallocated result binary. D2 is a WILD HEAP WRITE, not a lost update;
  it is a THIRD CONSUMER of site 12 (per-consumer verdict unit,
  Amendment 1 doctrine), so its fix belongs to this lane: the position
  advance is reordered BEFORE the allocation (a write through the
  still-valid pointer; on allocation failure the match context is
  abandoned and never resumed, so the early advance is unobservable).
  `jit_bs_get_integer`/`get_utf*` write-backs never cross an allocation
  and stay SAFE.
- **D3 (root shape):** helper ARGUMENTS (`match_ctx` raw, `binary` raw)
  and the `JitMatchContext` pointer are Rust locals, not GC roots —
  `ensure_space` forwards x-registers, never these. Post-collection use
  of any pre-collection raw is stale by construction; in-helper fixes do
  not exist. This is the ABI-level question RF-006 owns.

## Fix shape (fix commit of this lane)

Ruled by the domain owner 2026-07-28 (copy shape, do-not-stop, with
disclosure conditions). Uniform own-then-allocate in `jit_bs_get_binary`:
copy `bytes` to an owned `Vec` at capture, single `allocate_binary` path
for BOTH arms, position advance moved BEFORE the allocation (the D2
consumer above). This eliminates the sibling's stale-Term write by
construction (no sub-binary, no parent term) and also covers
sub-binary-over-inline-parent sources, which the ProcBin arm mishandled
the same way. Soundness of the copy: taken BEFORE the collecting
allocation, so no borrow crosses it; the off-heap source bytes are
Arc-retained across the collection because the GC traces MatchContext
word 3 (verified independently at the tearer's hands).

**DISCLOSED COST (rides the verdict to coordination for aion visibility):**
ProcBin-source extractions lose O(1) sub-binary sharing and copy instead —
`bs_get_binary` extraction loops over large sources go O(len) per step.

**NAMED DEBT (RF-006 / 0.17.0):** restoring sub-binary sharing requires
real rooting of helper-held terms across collections — the D3 argument-
staleness family (including D1 `start_match`, and the tearer's F3:
accumulated result Terms in Rust vecs across a second mid-loop
collection) is routed there; the walls here assert bytes and referent
correctness only, never allocation strategy, so a rooted sharing
implementation flips nothing when it lands.
