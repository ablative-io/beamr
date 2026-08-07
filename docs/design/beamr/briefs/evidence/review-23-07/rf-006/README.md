# RF-006 — evidence

Brief: `docs/design/beamr/briefs/RF-006.json` (R1..R6).
Build base: **`4055cbe`**. The brief pins at `a0359a2`; the re-pinned defect
table is the authority for coordinates — see "Re-pin" below.

---

## Re-pin (2026-08-07)

The brief's coordinates are stale by 125 commits. Two of them do not merely
shift:

- **`c7609b6` is not in main's history.** It is the 0.16.3 release-line
  commit. Main's discharge of H3-b is its forward-port **`7af68d2`**,
  established by content (`git log -S'let bytes = bytes.to_vec();' HEAD`,
  sole result), not by commit message.
- **R3 names `allocate_extracted_binary` with a JIT-only file list.** The
  JIT-local function by that name was **deleted** by the forward-port. The
  name now resolves only to
  `interpreter/opcodes/binary/matching.rs:636` — a same-named,
  different-signature function in another subsystem, which is **safe there**
  (bump-only `alloc` plus caller pre-reservation). Cite by function and
  subsystem, never by line alone.

State at `4055cbe`:

| defect | state | coordinates |
|---|---|---|
| **H3** (D2) `jit_bs_start_match` | **OPEN** | capture `:18` → collecting `alloc_words` `:25` → stale write `heap[3] = source.raw()` `:33` |
| **H3-b** (D3) `jit_bs_get_binary` | **DISCHARGED** by `7af68d2` | ownership `:96`, position advance moved pre-alloc `:103` |
| **ProcBin arm** | **DISCHARGED BY DELETION, NOT BY REPAIR** | JIT-local `allocate_extracted_binary` no longer exists |
| **H4** (D4) `write_map_entries` | **OPEN** | `keys` `:130` · `values` `:131-134` · `gc::ensure_space` `:135` · `alloc_slice` `:138` · `write_map` `:139` |

**The ProcBin distinction binds the ordering.** The arm was removed, and its
removal *is* the O(1)→O(m) extraction regression. Restoring sharing
re-introduces the identical stale-`Term` capture unless rooting exists
first.

---

## R1 walls — status at this base

Two of R1's three walls already existed before this lane:

| wall | test | state |
|---|---|---|
| W2 (H3-b) | `bs_get_binary_inline_source_survives_forced_collection` | pre-existing, green |
| — (ProcBin sibling) | `bs_get_binary_procbin_source_box_referent_survives_forced_collection` | pre-existing, green |
| **W1 (H3)** | `start_match_source_survives_forced_collection` | **added here, RED** |
| **W3 (H4)** | `map_update_boxed_values_survive_forced_collection` | **added here, RED** |

The two pre-existing walls come from `f6deb03` (trued by `0653809`) and pass
at this base. **They count as existing, not as observed-red at my hands:**
"RED at this commit by design" is `f6deb03`'s own commit message, not a
measurement taken here. Post-fix they are regression tests. They are **not**
red-first evidence for work not yet done.

### W1's absence was structural, not incidental

Before this lane the only test caller of `jit_bs_start_match` was the helper
`start_match_rooted`, which runs its nursery fill **after** the call — so
`start_match`'s allocation had never collected in any test. `runtime_map.rs`
had no `#[cfg(test)]` module at all.

**H3 and H4 were not under-tested; they were untested by construction.** The
geometry that trips them had never been arranged. A green suite over a code
path whose hazard was never staged is not evidence of safety.

---

## Observed RED at `4055cbe`

Artifacts (captured by redirect, rc recorded separately):

```
runs/red-w1-w3-targeted.txt   rc → runs/red-w1-w3-targeted.rc    = 101
runs/red-w1-w3-full-lib.txt   rc → runs/red-w1-w3-full-lib.rc    = 101
```

Full-suite denominator, quoted from `runs/red-w1-w3-full-lib.txt`:

```
test result: FAILED. 1784 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out
```

The suite was 1784 tests before this lane; it is 1786 after. **The two new
walls are the only failures — nothing else moved.**

### Corruption face, verbatim

**W1 (H3)** — the match context stored a term **bit-identical to the
pre-collection capture**, while the register file holds the forwarded one:

```
assertion `left == right` failed: context stored the pre-move source:
  stored=0x00000008dc401004 forwarded=0x00000008dc401804 original=0x00000008dc401004
  left: Term(38054924292)
 right: Term(38054926340)
```

`stored == original` exactly. That is the defect stated directly, not
inferred from a downstream refusal.

**W3 (H4)** — the merged map stored a pre-move value term:

```
assertion `left == right` failed: merged map stored a pre-move value for key 0:
  stored=0x00000008dc400004 forwarded=0x00000008dc40086c
  left: Term(38054920196)
 right: Term(38054922348)
```

Both faces are **zero-filled-read refusals**, which is the expected class per
R1's acceptance (`HeapRegion::reset()` zero-fills the vacated young region).
**Neither wall red with a segfault or with garbage**, so R1's STOP condition
was not triggered. Before the assertions were sharpened to compare terms
directly, the same two walls red as `BINARY_HELPER_FAILURE` (`u64::MAX`) and
as a failed `BinaryRef::new` respectively — the downstream symptoms of the
same zero-fill.

### Riders, both satisfied by both walls

- Forced geometry is **asserted**, not arranged and trusted:
  `available() < needed` before the subject call.
- The collection is **asserted to have run**: `old_used() == 0` before,
  `old_used() > 0` after.
- The live input is asserted to have **moved** (`assert_ne!` on the register).

---

## GREEN — R2 + R3-remainder (H3) + R4 (H4)

Wall commit (RED): **`5908a38`**. The fix commit is its child.

**R2 — `alloc_words_rooted` (`jit/runtime.rs`).** Pushes the caller's terms
onto the native root stack, allocates, reads the forwarded values back into
the caller's slice, and truncates to entry depth. Single exit path, so depth
is restored on success, on `words == 0`, and on allocation failure alike.
The read-back happens **before** the truncate and on **every** path — a
failed allocation can still have run a collection that moved the terms.

**R3-remainder (H3).** `jit_bs_start_match` roots `source` across the
match-context allocation and writes the forwarded value into `heap[3]`.

**R4 (H4).** `write_map_entries` roots every key and value across the
allocation and writes the forwarded values into the fresh map. It now uses
`alloc_words_rooted` in place of `gc::ensure_space` + `alloc_slice`; the
live-register count (`256`) and every refusal value are unchanged.

⚠️ **R4's spec as written was unrealisable.** It said to move the reservation
ahead of the captures "mirroring `put_map`". `put_map` is safe because it
**re-reads the source from its operand** after reserving
(`interpreter/opcodes/closures.rs:476-480`). This helper has no operand, and
generated code stages the update pairs into a Cranelift `ExplicitSlot` the
collector cannot see — so re-reading returns the same stale word. **Hoisting
alone does not fix H4; R4 depends on R2.** Ruled and accepted before build.

Artifacts:

```
runs/green-r2r3r4-full-lib.txt   rc → runs/green-r2r3r4-full-lib.rc   = 0
runs/green-r2r3r4-clippy.txt     rc → runs/green-r2r3r4-clippy.rc     = 0
```

```
test result: ok. 1790 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

`cargo fmt --all -- --check` rc 0. Clippy artifact carries **zero** lines
matching `^(warning|error)`.

**Denominator walk:** 1784 before this lane → 1786 at the wall commit (W1,
W3) → 1790 after the fix (four R2 unit tests). Every delta named.

**Fail-first is in history, not asserted.** W1 and W3 are red at `5908a38`
and green at its child, with the production diff between them. No
revert-and-restore was needed to demonstrate it.

**Bar:** zero new `#[allow]`, zero `_ =>` arms, zero `unwrap`/`expect`/`panic`
in production across all three files. Hard stops verified untouched at zero
diff lines each: `native/stdlib_stubs/string_bifs.rs` (Osiris' H5 lane),
`loader/encode/**` (Vesper's claim), `jit/compiler/ir_helpers.rs` (JIT ABI
signatures, so generated code is byte-identical). The refcount marking in
`allocate_binary` is unchanged and its `gc_release_tests` wall stays green.

## Review finding on `alloc_words_rooted` — fixed

Raised by the domain lead against the first version of the facility, and it
was right. The read-back was:

```rust
if let Some(forwarded) = process.native_root(depth + index) {
    *root = forwarded;
}                    // and on None: *root keeps the PRE-COLLECTION term
```

`native_root` returns `None` for an out-of-bounds slot. On `None` the caller
silently continued with the stale term — **precisely the defect this lane
exists to remove, wearing a safe-looking `if let`.** The silent arm was the
dangerous arm.

Two changes:

1. **The unreadable case is now loud**: the allocation is REFUSED (null
   return) rather than handing back a pre-collection term. Callers already
   treat null as a refusal — `jit_bs_start_match` returns `0`,
   `write_map_entries` returns `None` — so this rides existing, generated-code
   -visible paths and introduces no new edge.
2. **The indices come from `push_native_root`'s return value** instead of
   being recomputed as `depth + index`. One fact, one source; the arithmetic
   relationship that could disagree with it is gone rather than guarded.

The invariant that makes `None` unreachable — that a collection forwards the
native root stack in place and never truncates it — is a claim about **the
collector, not about this function**. It is now pinned by
`collection_preserves_native_roots`, which forces a real collection over a
pushed root and asserts the depth is unchanged, the slot is still readable,
the term was forwarded, and the bytes survive. **A function whose safety
rests on an invariant elsewhere should assert it, not assume it.**

```
runs/green-r2-hardening-full-lib.txt   rc → .rc = 0
runs/green-r2-hardening-clippy.txt     rc → .rc = 0
test result: ok. 1791 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Denominator: 1790 → **1791**, the one new invariant test.

## Still owed in this lane

- **Mutation evidence** (one minimal semantic mutation per wall, as a diff
  file plus its observed red, never applied to the tree) — produced **after**
  the fix, since its job is to prove the wall catches a regression rather
  than that it catches the current defect. The red above already proves the
  latter.
- R2 (rooting facility), then H3, H4, and the sharing restoration.
- R6's site table, including two rows that are **not defects**:
  the unconsumed JIT stack-map machinery, and the `stage_pairs` staging class
  (see below).

## Two findings that belong in R6 but are not defects

1. **JIT safepoint stack maps are unconsumed.** `SafepointBuilder` records
   allocation-site roots, `StackMapEntry`/`RootLocation` carry them, and AOT
   serialises them — but `stack_maps()`/`RootLocation` have **zero**
   references outside `jit/` (control: 29 unfiltered). The collector never
   reads them. A producer with no consumer is not a mechanism; the exposure
   is that a future reader finds a builder, a type and a serialiser and
   concludes the JIT has root tracking.
2. **The `stage_pairs` staging class.** `ir_map.rs` stages pair terms into a
   Cranelift `ExplicitSlot`. `Process::roots_with_live_x`
   (`process/mod.rs:562-591`) enumerates x/y registers, mailbox, exception
   roots, dictionary, group leader, native roots and continuations — **and no
   JIT stack slot**. Every instance of this staging shape is invisible to the
   collector by construction, so the class needs a population and a
   denominator, not a single-site note.
