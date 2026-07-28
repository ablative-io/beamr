# AION-ENCODE-GC-DEFECT — 0.16.3 backport, lane 2: ETF structural pair

**Arc/base/tear:** see `../lane-1-mechanical/README.md` — same branch
(`fix/0163-borrow-across-alloc` off `67f89c4`), same discipline, lane 2 of
the lane-at-a-time declaration sequence. This lane extends the lane-1 span
(head `dc678b8`).

## The sites (audit rows 1–2 — the WORST span)

`bif_binary_to_term` (`native/etf_bifs.rs`) and `bif_binary_to_term_2`
take `bytes = BinaryRef::new(*binary)...as_bytes()` — a laundered
`&'static [u8]` borrow of the source binary — and hold it across the
ENTIRE `decode_term` / `decode_term_with_options` recursion, which
allocates terms on the process heap throughout. Any allocation inside the
decode that collects moves + zero-fills an inline (≤ 64 B) source; the
decoder then reads zeroed memory.

This lane is separate from lane 1 because the borrow spans a whole
recursion rather than a single allocating call: the fix must take
ownership UP FRONT, before the recursion — interior spot-fixes inside the
decoder are a tear FAIL by the dispatch's explicit rider.

## Walls (red-first, per consumer)

- `binary_to_term_survives_forced_collection` — `binary_to_term/1`.
- `binary_to_term_2_used_survives_forced_collection` — `binary_to_term/2`
  with `[used]`, asserting both the decoded structure and the returned
  used-byte count (its extra `alloc_tuple` consumer rides the same borrow).

Payload: ETF for `[<<"aaaa">>, <<"bbbb">>]` (25 bytes — inline), built
from the crate's own `etf::tags` constants. Two binary elements make the
multi-slice partial-corruption shape: the first element's allocation
collecting invalidates every later read of the source. Geometry and
assertions are the lane-1 shape (input rooted in X0, nursery filled until
the first allocation must collect, `old_used() > 0` asserted, byte-exact
element assertions, shared `AtomTable`).

## Records

- `runs/red-etf-pair.txt` — both walls FAILED at the unfixed tree
  (`dc678b8` + walls): `expect` panics on `Err(Term(73))` = badarg. The
  red face here is a DECODE ERROR, not silent zeros: the source is zeroed
  mid-decode, so the decoder trips on a zeroed tag byte. (Whether a
  payload exists whose zeroed continuation still decodes cleanly — silent
  corruption — is exactly why the span is the defect; the wall pins the
  span, not one face.)
- Green run in the fix commit of this lane.

## Fix shape (fix commit of this lane)

Ownership UP FRONT: copy the source bytes into an owned `Vec<u8>` at BIF
entry, before the decode recursion begins, in both BIFs. No decoder
changes; no interior fixes.
