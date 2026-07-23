# AION-ENCODE-GC-DEFECT — walls lane 1 (multibyte-encode walls) run record

**Lane:** AION-ENCODE-GC-DEFECT walls lane 1 (Tom's ratified item 4; GO from
Artemis Peach 2026-07-23 22:30Z on the evidence-lane PASS). Build seat:
Osiris Yogo. Domain owner/tear: Artemis Peach. The walls are permanent
in-tree tests in `crates/beamr/src/native/stdlib_stubs/json_bifs_tests.rs`;
this directory is the lane's mutation evidence per the tear riders.

## The five walls

1. `encode_binary_passes_multibyte_utf8_through_byte_exact` — OTP `json`
   passes non-ASCII UTF-8 through unescaped; em-dash and mixed
   multibyte/escape output asserted against hand-written exact bytes
   (`"\"\xE2\x80\x94\""` literal for the lone em-dash).
2. `encode_binary_rejects_invalid_utf8` — badarg on truncated em-dash
   prefix, never-valid byte, lone continuation, overlong encoding,
   UTF-8-encoded surrogate; the complete em-dash still encodes (no
   over-reject).
3. `encoded_multibyte_binary_round_trips_through_decode` — encode→decode
   returns the original bytes; `—` decodes to em-dash bytes and
   re-encodes as raw passthrough.
4. `multibyte_binary_moved_by_minor_gc_encodes_byte_exact` — a ≤64-byte
   inline binary is physically moved by a minor collection (asserted:
   forwarded term differs), then encodes byte-exact from the moved bytes.
5. `encode_result_allocation_collects_with_multibyte_input_live` — FORCED
   GEOMETRY (rider 3): the nursery is filled until
   `available() < alloc_binary_word_count(escaped_len)` (asserted as a
   precondition), so the encode's result allocation deterministically
   collects with the multibyte input live in X0 (`live_x = 1`). The
   collection is ASSERTED to have happened (`old_used()` 0 → >0 and the
   input term forwarded), and both input and output are byte-exact after.

All five walls are GREEN at unmutated main — no STOP owed under rider 4.
The BIF's read/escape completes before its result allocation, so no borrow
crosses the collection at main.

## Mutations (riders 1–2: one minimal semantic mutation per wall, committed
## as diff files, never applied to the tree)

Each mutation was applied locally, its red observed and captured, then
reverted; the tree never carried a mutation in any commit. Diffs in
`mutations/`, observed red runs in `runs/`.

| Mutation | Defect class | Designated wall | Walls red |
|---|---|---|---|
| m1-escape-high-bytes | byte-wise `\u`-escape of bytes ≥ 0x80 (mangles UTF-8) | 1 | 1, 3, 4, 5 |
| m2-remove-utf8-check | encode accepts invalid UTF-8 | 2 | 2 only |
| m3-decode-masks-high-bytes | decode strips the high bit of plain bytes | 3 | 3 only |
| m4-minor-copy-zeroes-binary-tail | minor-GC copy loses a moved binary's final payload word | 4 | 4, 5 |
| m5-ensure-space-grow-only | allocation pressure grows the heap instead of collecting | 5 | 5 only |

Every wall kills its designated mutation; no single mutation flattens all
five (m1 leaves wall 2 green; m2/m3/m5 are killed by exactly one wall
each). m1's overlap onto walls 3–5 and m4's onto wall 5 is inherent —
those walls all assert multibyte output exactness downstream of the same
bytes — and is recorded here rather than tuned away. m5 is the
production-residency defect class from the parent lane (collections stop
firing under allocation pressure); its red fires on the wall's explicit
"must have run a collection" assert.

## Reproduce a red

```sh
git apply mutations/<name>.diff
cargo test -p beamr --lib json_bifs_tests   # designated wall goes red
git checkout -- crates/beamr/src/native/stdlib_stubs/json_bifs.rs \
                crates/beamr/src/gc/minor.rs crates/beamr/src/gc/mod.rs
```

## Environment (recorded at run time, 2026-07-24 AEST)

- macOS 26.5.2 (25F84), Darwin 25.5.0, Apple M5 Pro (arm64)
- rustc 1.95.0, cargo 1.95.0; branch `lane/encode-walls-1` off main `8beea17`
- Full `cargo test -p beamr --lib` at the walls commit: 1732 passed, 0
  failed; `cargo fmt --check` and `cargo clippy --lib --tests` clean.
