# AUDIT-80 — native/ as_bytes sites re-verdicted under the type-derived producer set

Artemis Peach, 2026-08-16. Tree: `765764d`. Board origin: Cally's #150 — the
original as_bytes AUDIT.md discriminated by the SPELLING `as_bytes`; the
producer set is really "whatever launders a `&'static` borrow over process
memory", and the spelling exclusion was the mechanism of the site-12 miss.
This audit re-verdicts all of `native/` under a discriminator derived from
the type system, per the boarded design constraint.

## VERDICT: ZERO REAL SITES in native/ — the filed advisory's site set stays complete

153 occurrences verdicted (137 `.as_bytes()` + 13 `.limbs()` + wrapped-str
derivations), across every file in `crates/beamr/src/native/`:

| class | count |
|---|---|
| REAL | **0** |
| SAFE-BY-CONSTRUCTION | 84 |
| SAFE-BY-PRE-RESERVATION | 0 |
| TYPE-SAFE (receiver never process memory) | 69 |
| UNRESOLVED | 0 |

(71 of these are in test code; verdicted anyway, tagged TEST.) Every SAFE
names its kind, per the boarded verdict rule — the dominant constructions are
"copied to owned memory at the mint point" and "last read precedes first
allocation"; several files carry explicit own-up-front comments showing the
hazard is known at those seams.

## The instrument (type-derived, as ruled)

Producer set derived by asking what CAN return a `&'static` borrow over
process memory — two censuses (signature + unsafe-mint), each self-tested on
known positives BEFORE any zero was believed. **Both self-tests earned their
keep, live:**

1. A `from_raw_parts`-only mint census MISSED `bytes_from_raw_word`
   (term/shared_binary.rs:79), which mints via `unsafe { (*ptr).as_slice() }`
   — a raw-pointer deref. The census gained a second prong.
2. The `-> &'static` signature census MISSED wrapped returns
   (`Result<&'static str, _>` / `Option<&'static [u8]>`): `utf8_str`
   (string_bifs.rs:346), `binary_to_utf8` (gate3/type_conversion.rs:230),
   two file-local `binary_bytes` (string_bifs.rs:350, encoding_bifs.rs:203),
   `parent_bytes` (accessors.rs:357), and the interpreter/JIT-local
   `slice`/`bytes` family. Caught by cross-checking the walkers' rows against
   the set; the wrapped-form census then run to closure.

Final producer set reachable from native/: `Binary::as_bytes`,
`ProcBin::as_bytes` (→`bytes_from_raw_word`), `SubBinary::as_bytes`
(→`parent_bytes`), `BinaryRef::as_bytes`, `BigInt::limbs` (&'static [u64] —
same hazard class the old u8 scoping structurally excluded), plus the four
wrapped re-exporters above. `slice_from_words` and the JIT builders are not
reachable from native/.

Walk executed by two independent read-only workers (stdlib_stubs | rest),
each with a self-testing census and the kind-required vocabulary; results
verified at my hands: every alarming-shaped row re-walked (below), SAFE rows
sampled (`bif_split`, `bif_find`, `binary_to_atom`, `binary_to_list`,
`load_module`), zero discrepancies of substance.

## The two alarming candidates, dissolved at the receiver type

`context.alloc_bigint(value.is_negative(), value.limbs())` at
native/bifs.rs:380 and stdlib_stubs/bitwise_bifs.rs:181 passes a `limbs()`
borrow INTO a collector-reaching call. Verified at my hands: both receivers
are the **owned** `BigIntValue` (term/bigint_math.rs — `limbs()` borrows a
Rust-heap `Vec<u64>` field), NOT the heap accessor `boxed::BigInt`. GC cannot
move a Rust-heap Vec. TYPE-SAFE — and the dissolution is itself the
discriminator working: surface shape said REAL, receiver type said safe.

## Standing hazard finding (no current instance; recorded, not fixed here)

`ProcessContext::alloc_bigint` (context/alloc.rs:205) and `alloc_binary`
(alloc.rs:138) both call `alloc_words` → `ensure_heap_space` →
`gc::ensure_space` FIRST and copy their slice argument SECOND, without
rooting it. A heap-minted borrow passed to either is a silent UAF — the
signature `&[u64]`/`&[u8]` cannot express "must not be process-heap
provenance". Every current native/ caller passes owned or non-heap data, so
this is a latent API footgun, not a defect. It is the byte-level sibling of
the AR-1 sink problem (whose remedy Candidate B seals `&[Term]` sinks by
trait); noted for the AR-1 ruling's context, not acted on unilaterally.
Contrast: `alloc_list_with_tail` already roots its Term inputs — the API
family knows the hazard for terms and is blind to it for bytes.

## Provenance-vs-type refinement (recorded against walker rows)

Four string_bifs sites (:65, :125, :157, :218) read `str` receivers whose
strings are HEAP-PROVENANCE (`utf8_str` launders heap bytes to
`&'static str`). The walker labeled them TYPE-SAFE by receiver; this record
reclassifies them SAFE-BY-CONSTRUCTION (copy precedes allocation — verified),
because "receiver type is str" is the old spelling mistake in mirror form:
provenance decides, not surface type. Verdict unchanged; label corrected.

## Bearing on RUSTSEC

No new REAL sites ⇒ nothing to add to the pending advisory texts (PR #3122
and the split drafts routed 4d0bb293). The task's motivating question — "if
the exclusion applies anywhere it applies inside native/ too" — is answered:
it applied (the raw-pointer mints exist) but their reach ends outside native/,
and the full type-derived walk of native/ found zero read-after-allocation
sites.

Docs/evidence-only landing; no production bytes changed; no battery owed.
