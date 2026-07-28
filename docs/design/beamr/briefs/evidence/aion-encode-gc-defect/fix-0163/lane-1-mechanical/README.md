# AION-ENCODE-GC-DEFECT — 0.16.3 backport, lane 1: mechanical own-the-bytes fixes

**Arc:** beamr 0.16.3 corruption backport (Waffles' full dispatch 2026-07-28
13:29Z, superseding; Tom's GO on record; tear: Artemis Peach, lane-at-a-time
declarations on this one branch, her ruling 2026-07-28 13:32Z). Build seat:
Osiris Yogo. This directory is lane 1 of the branch; later lanes (ETF
structural, jit site 12 + sibling, tripwire walls, release mechanics) get
sibling directories under `fix-0163/`.

**Base pin:** branch `fix/0163-borrow-across-alloc` from **`67f89c4`**
("release: beamr 0.16.2 — memory-safety patch (REVIEW-23-07 C1+C2)"), NOT
main — main is 72+ commits ahead and carries `1ab619a`
(`refactor(scheduler)!: remove spawn_link_dirty`), a breaking API removal
that cannot ride a patch release. There is NO `v0.16.2` tag (tags stop at
`v0.15.2`); all pins in this record are by SHA. Verified at this seat:
every file lane 1 touches is byte-identical between `67f89c4` and main
`ba5c5fa`, so the audit's verdicts (taken at main `f684d60`) hold unchanged
here.

**Scope references (both on beamr main):** the landed audit
`asbytes-sweep/AUDIT.md` (main `ba5c5fa`) and its AMENDMENT 1 (main
`7e56073`, per-consumer verdict unit; site 12 = lane 3 of this branch).
Amendment 1's chain was re-verified at the bytes AT THIS BASE before build.

**COUNT:** the ruling language once said "mechanical ten"; the audit table
is the count of record and yields **NINE** non-ETF mechanical crossings
(11 real crossings − 2 ETF). Disclosed to Anubis at discovery (my
arithmetic slip in the original STOP), reconfirmed by Waffles' dispatch:
"if your build finds a tenth it names itself." None named itself.

## Defect (one class, audited)

`BinaryRef::as_bytes` returns a laundered `&'static [u8]`. For an inline
heap binary (≤ 64 B) the slice points into the young region; if it is read
after (or inside) a collecting allocation — `context.alloc_binary(slice)`
reads the slice AFTER its own `ensure_heap_space` — a collection triggered
at that moment moves the source and `HeapRegion::reset()` ZERO-FILLS the
young region. The output is silently built from zeros: no error, no crash.

## The nine sites — walls red-first PER CONSUMER, then fixed

| # | Site | Consumer | Wall |
|---|------|----------|------|
| 1 | `native/gate3_bifs/mod.rs` | `bif_list_append`, `Binary ++ []` arm | `list_append_binary_arm_survives_forced_collection` |
| 2 | `native/stdlib_stubs/misc_bifs.rs` | `bif_binary_part` | `binary_part_survives_forced_collection` |
| 3 | `native/stdlib_stubs/uri_bifs.rs` | `bif_uri_string_parse` (multi-slice: partial-corruption shape) | `uri_parse_components_survive_forced_collection` |
| 4 | `native/stdlib_stubs/uri_bifs.rs` | `error_tuple` ← `dissect_query` error path (input-derived detail) | `uri_dissect_query_error_detail_survives_forced_collection` |
| 5 | `native/stdlib_stubs/string_bifs.rs` | `bif_trim` | `string_trim_survives_forced_collection` |
| 6 | `native/stdlib_stubs/string_bifs.rs` | `bif_split` (multi-slice) | `string_split_survives_forced_collection` |
| 7 | `native/stdlib_stubs/string_bifs.rs` | `bif_find` | `string_find_survives_forced_collection` |
| 8 | `native/stdlib_stubs/string_bifs.rs` | `bif_pad`, early-return arm | `string_pad_early_return_survives_forced_collection` |
| 9 | `native/stdlib_stubs/string_bifs.rs` | `bif_slice` | `string_slice_survives_forced_collection` |

**Per-consumer verdicts at this site set (Amendment 1 discipline):**

- `error_tuple` has TWO calling consumers. The `dissect_query` error path
  passes input-derived `part` — REAL crossing, walled (row 4). The `parse`
  invalid-port path passes the literal `":"` — a `&'static str` not on any
  process heap; the detail alloc cannot read moved young bytes, and the
  subsequent `alloc_tuple` roots its arguments (proven by the existing
  `alloc_tuple_roots_boxed_arguments_across_gc` test). Verdict: SAFE, no
  red owed. Reported at discovery; verified at the domain owner's hands
  and recorded as **AUDIT.md AMENDMENT 2** (beamr main `92a9d2e`,
  forward-only): row 6's "input-derived in its callers (parse +
  dissect_query error paths)" overstated — only dissect_query's detail is
  input-derived. Site 6's fix scope is unchanged (owning inside
  `error_tuple` covers every caller); the red drives through a
  dissect_query error path; the parse-path consumer stays untouched.
- `bif_pad`'s general path builds an owned `out` Vec before allocating —
  SAFE per the audit; only the early-return arm crosses (row 8).

## Wall shape (generalizes the audit's observed probe)

Inline (≤ 64 B) input binary rooted in X0; nursery filled with cons cells
until `available() < alloc_binary_word_count(result_len)`; BIF called with
`live_x` = arity; asserted: `old_used() > 0` (the collection really
happened) AND byte-exact output. One shared `AtomTable` across every
context in a wall — the BIF interns atoms (uri map keys, direction atoms)
and assertions must compare the same interned indices. The dissect_query
wall expects the `{error, invalid_query, Detail}` tuple as an Ok VALUE per
the OTP `uri_string:dissect_query/1` contract (return
`QueryList | {error, Atom, term()}`, not an exception).

These reds are the REAL hazard, not mutations: the red is the defect
itself and flips green with the fix (dispatch requirement, verbatim). Wall
2 reproduces the lane-3 probe byte-for-byte
(`binary:part(<<1..=40>>, 10, 20)` → expected `[11..=30]`, red face
`[0 × 20]` — cf. `asbytes-sweep/runs/probe-binary-part-red.txt` at main).

## Records

- `runs/red-nine-walls.txt` — all nine walls FAILED at the unfixed base:
  every assertion shows zeroed left vs expected bytes right. `cargo test`
  exit 101 is the test binary reporting those failures — labeled; no crash
  face; the corruption is silent.
- Green run follows in the fix commit of this lane; battery + gates
  evidence ride the final consolidated range before the last declaration.

## Fix shape (fix commit of this lane)

Own the bytes before the allocating call — the documented
`bif_json_decode` "Own the bytes" pattern: single-slice sites copy with
`.to_vec()` before `alloc_binary`; multi-slice sites (`uri_string:parse`,
`string:split`, and the `string_bifs`/`uri_bifs` text helpers) own one
`String`/`Vec` at entry so every derived slice borrows the owned copy, not
the heap. Zero new `#[allow]`; the audit's 57 SAFE sites untouched;
`loader/`, `encode/**` zero bytes changed.
