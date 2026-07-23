# AION-ENCODE-GC-DEFECT — lane 3: as_bytes borrow-across-alloc audit

**Lane:** AION-ENCODE-GC-DEFECT lane 3 (GO from Artemis Peach 2026-07-23
23:06Z, her scope of record). Build seat: Osiris Yogo. Sweep base: main
`f684d60`, worktree `.worktrees/asbytes-sweep`. Enumeration mechanical
(sub-agent, grep + receiver-type tracing); every verdict verified at the
bytes at this seat. STOP sent to the domain owner at discovery, before any
fix design, per the lane scope.

## Hazard model (verified at the bytes)

`BinaryRef::as_bytes` (and the sibling accessors on `Binary`, `ProcBin`,
`SubBinary`) return `&'static [u8]` — a laundered borrow of either
process-heap words (inline `Binary` ≤ 64 B; `SubBinary` over an inline
parent) or off-heap Arc data (`ProcBin`; `SubBinary` over a ProcBin;
`SharedBinary`). A crossing exists when such a slice is READ after a call
that can collect on the same process:

- `context.alloc_*` (non-`_prereserved`) → `ensure_heap_space` →
  `gc::ensure_space` → `collect_minor`/`collect_major`. Crucially,
  `context.alloc_binary(slice)` reads `slice` INSIDE the call, after its
  own reserve — a slice passed directly into it crosses.
- After a minor collection, `HeapRegion::reset()` ZERO-FILLS the young
  region (heap.rs `reset`), so a stale borrow of moved inline bytes reads
  zeros — the face is SILENT CORRUPTION, not SEGV. Rooting the source term
  does not help: the promoted copy moves, the borrow points at the zeroed
  original.
- Immune shapes: ProcBin-backed bytes (off-heap, Arc retained while the
  term is rooted — BIF args are rooted via `live_x = arity`); slices
  consumed into owned storage (`.to_vec()`, `extend_from_slice`, `String`
  building, hashing, comparison) before any allocation point.
- NON-collecting allocations are safe to cross: the mailbox message-copy
  path allocates via bare `Heap::alloc` (bump-only; `HeapFull` becomes
  `SendError`; cannot collect, existing data never moves).

**Observed, not just reasoned** (probe applied locally to a test file and
reverted — never committed; probe record retained at the seat):
`binary:part(<<1..40>>, 10, 20)` with the input live in X0 and the nursery
filled so the result allocation must collect returned **twenty zero bytes**
(`collected=true, expected=[11..=30], actual=[0×20]`) at main `f684d60`.

## Verdicts — REAL crossings (11 sites, one class)

| # | Site | Function | Borrow span → crossing | Notes |
|---|------|----------|------------------------|-------|
| 1 | `native/etf_bifs.rs:92` | `bif_binary_to_term` | `bytes` borrows source binary across the ENTIRE `decode_term` recursion, which allocates terms on the process heap throughout | Worst span; small ETF payloads are inline binaries |
| 2 | `native/etf_bifs.rs:102` | `bif_binary_to_term_2` | same via `decode_term_with_options` | + `alloc_tuple` after decode with `used` |
| 3 | `native/gate3_bifs/mod.rs:824` | `bif_list_append` | `context.alloc_binary(binary.as_bytes())` — slice read inside the alloc | `Binary ++ []` arm |
| 4 | `native/stdlib_stubs/misc_bifs.rs:128` | `bif_binary_part` | `context.alloc_binary(&bytes[offset..end])` | **Probed: returns zeros under forced geometry** |
| 5 | `native/stdlib_stubs/uri_bifs.rs:63-90` | `bif_uri_string_parse` | up to 6 input-derived component slices passed to sequential `alloc_binary` calls | Partial-corruption shape: first collecting alloc zeroes the source; later components read zeros |
| 6 | `native/stdlib_stubs/uri_bifs.rs:257` | `error_tuple` | `detail` is input-derived in its callers (parse + `dissect_query` error paths) → `alloc_binary(detail.as_bytes())` | |
| 7 | `native/stdlib_stubs/string_bifs.rs:64` | `bif_trim` | `trimmed` slices input text → `alloc_binary` | |
| 8 | `native/stdlib_stubs/string_bifs.rs:86` | `bif_split` | `parts` slice input; per-part `alloc_binary` loop | Multi-slice like URI parse |
| 9 | `native/stdlib_stubs/string_bifs.rs:98` | `bif_find` | `alloc_binary(&input[index..])` | |
| 10 | `native/stdlib_stubs/string_bifs.rs:152` | `bif_pad` | early-return `alloc_binary(input)` | General path is safe (`out` owned) |
| 11 | `native/stdlib_stubs/string_bifs.rs:209` | `bif_slice` | `alloc_binary(&text.as_bytes()[start..end])` | |

All eleven share one defect class and one mechanical fix shape (own the
bytes before the allocating call — the pattern `bif_json_decode` already
uses, with its "Own the bytes" comment), except the ETF pair, where the
borrow spans the whole decoder recursion and the fix is structural (own
the input up front). **No fix is designed or applied in this lane — STOP
issued per scope.**

## Verdicts — SAFE sites (57)

Grouped by why the borrow never crosses a collection point. Every site
listed was verified at this seat; receiver types per the enumeration.

**Owned-copy before any alloc** (`.to_vec()` / `extend_from_slice` /
`String`-building / base64-hex encode consume the slice first):
`meridian_ffi.rs:88,92,96` (collect_bytes); `tcp_bifs.rs:683`;
`udp_bifs.rs:93`; `file_bifs.rs:444`; `otp_stubs/erlang_stubs.rs:269`;
`misc_bifs.rs:79,97` (collect_chardata / characters_to_list);
`gate3_bifs/additional.rs:118` (binary_part gate3 variant — copies at
:122 before alloc); `gate3_bifs/type_conversion.rs:101,220`
(binary_to_list builds owned elements; collect_iodata);
`encoding_bifs.rs:205` + its four callers (hex/base64 encode/decode — all
build owned output first); `io_bifs.rs:133,169`;
`gleam_stdlib_ffi2.rs:59`; `stdlib_stubs/json_bifs.rs:72` (decode owns
bytes — the documented pattern); `string_bifs` bif_reverse/lowercase/
uppercase/next_grapheme (owned `String`/`Vec` before alloc);
`uri_bifs` dissect_query main path (form_decode returns owned Vecs);
`standard_io.rs:261`; `distribution/etf.rs:448`; `etf/encode.rs:499`;
`term/json.rs:161,163`; `beamr-wasm/convert.rs:291,297,345` (JS-heap
copies).

**Read-only consumption, no allocation while live** (checks, parses,
comparisons, hashing, bit reads): `json_bifs.rs:45,144,147,196` (utf8
check / escape copies into `Vec` before the alloc — the shape lane-1
wall 5 proved under forced collection); `type_conversion_bifs.rs:26,142,
170` (parse-then-drop before result alloc); `string_bifs`
bif_length/bif_equal/bif_is_empty; `term/format.rs:184`;
`term/hash.rs:157`; `term/compare/mod.rs:319` (+ comparison callers —
no process-heap alloc in compare); `interpreter/.../segments.rs:150,173`,
`construction.rs:120`, `matching.rs:797,815` (BitWriter/BinaryBuilder
buffers and bit reads — no context, no collection);
`jit/runtime_binary_match.rs:175`, `jit/runtime_binary_build.rs:90`;
`file_bifs.rs:325`, `file_meta_bifs.rs:282` (PathBuf copies);
`code_management_bifs.rs:78` (code-server facility, no process-heap
motion); `type_conversion.rs:232` binary_to_utf8 → atom-table intern
(atom table, not process heap); accessor internals
(`binary_ref.rs:32-34`, `accessors.rs:286,359`).

**Non-collecting allocator on the far side** (bump `Heap::alloc`, cannot
collect; `HeapFull` → error return; existing data never moves):
`mailbox/mod.rs:466` (copy_binary), `:483` (copy_proc_bin — also off-heap
source, and `alloc_words` precedes `as_bytes`), `:496` (copy_sub_binary).
Self-send shares the heap object but the copy path still cannot trigger a
collection mid-copy. `ets/copy.rs:150,153,156` (into ETS Rust storage),
`:296,299,302` (ETS-stored stable source → destination heap distinct;
allocs are bump-only on that heap).

`crates/beamr-cli`: no qualifying sites.

## NAMED LOAD-BEARING FACT (mailbox pair) + banked rider

The mailbox verdicts above rest on one fact: **the message-copy path
allocates via bare `Heap::alloc` (bump-only), which cannot trigger a
collection — `HeapFull` propagates as `SendError` — and bump allocation
never moves existing data.** If message copy is ever rerouted through a
collecting allocator (`gc::alloc` / `ensure_space`), the mailbox pair
become crossings of exactly this audit's class. **Banked rider (Artemis's
ruling, 2026-07-23):** the future fix lane adds a tripwire wall pinning
this fact so the reroute cannot land silently.

## Exposure precondition (stated precisely)

Corruption fires when ALL of:
1. The input binary's bytes live on the process young heap: an inline
   heap binary (**≤ 64 bytes**, `REFC_BINARY_THRESHOLD`) or a sub-binary
   over an inline parent. ProcBin-backed inputs (> 64 bytes) and
   sub-binaries over ProcBins are immune (off-heap bytes; Arc retained
   while the arg is rooted, and BIF args are rooted via `live_x`).
2. The allocating call that consumes the input-derived slice triggers a
   collection at that moment: nursery `available()` below the request, or
   virtual-binary pressure at the threshold. (For the multi-slice sites —
   `uri_string:parse`, `string:split` — ANY earlier allocation in the
   sequence collecting corrupts ALL later slices.)
3. Effect: the young region is reset and zero-filled; the output binary
   is built from zeros. No error, no crash, no log — silent corruption.

**Affected BIF surface:** `erlang:binary_to_term/1,2`; `erlang:'++'/2`
(the `Binary ++ []` arm); `binary:part/3`; `uri_string:parse/1`;
`uri_string:dissect_query/1` (error path); `string:trim/2`,
`string:split/3`, `string:find/2`, `string:pad/4`, `string:slice/3`.

## aion reachability (per Artemis's escalation ask)

ALL ELEVEN sites are registered by the registration sets aion's embedding
loads (verified in this tree): the ETF pair via `register_gate1_bifs`
(bifs.rs:63 → `register_etf_bifs`); `'++'/2` via `register_gate3_bifs`
(gate3_bifs/mod.rs:111); `binary:part/3`, both `uri_string` BIFs, and all
five `string` BIFs via `register_stdlib_stubs`
(stdlib_stubs/registrations.rs:345, :247-260, :294-323). aion registers
gate1 + gate3 + stdlib stubs + gleam ffi + otp stubs (its
handle/registration.rs; this lane's red/green harness mirrors it), so the
full surface is CALLABLE from aion workloads. Which of them aion's Gleam
code paths actually call under load is aion-side knowledge (Vesper's
seat); `string:*` delegation from gleam_stdlib and `binary_to_term` on
session payloads are the likely hot candidates.

## Status — RULING OF RECORD (Artemis, 2026-07-23 23:23Z)

- **Lane 3 closes AUDIT-ONLY.** This record (all 68 verdicts, immunity
  split, named load-bearing fact + banked tripwire rider) plus the probe
  as committed red evidence (`mutations/probe-binary-part.diff` +
  `runs/probe-binary-part-red.txt` — real-hazard red, no mutation needed,
  never applied in a commit) are the deliverable. NO suite walls in this
  lane (they would red the battery at main — a red main does not ship).
- **NO FIX MOTION**: eleven silent-corruption sites are present in 0.16.2
  as deployed; whether the fix rides the open 0.17.0 window or backports
  as 0.16.3 is Tom's cadence call informed by aion's exposure — escalated
  to Waffles by the domain owner with this record's severity picture.
- Expected fix shape when the word comes down (planning only): mechanical
  ten in one production lane (own-the-bytes-before-alloc, the
  `bif_json_decode` pattern), red-first PER SITE with the real hazard,
  walls flip green with the fix and ride the suite; ETF pair as its own
  structural lane; mailbox tripwire wall rides the mechanical lane.
- The lane-1 during-collection wall (json encode) remains the proof that
  the SAFE pattern holds under forced geometry; the probe is the proof
  the UNSAFE pattern fails under the same geometry.
