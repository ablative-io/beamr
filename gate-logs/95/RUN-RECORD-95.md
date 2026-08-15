# RUN RECORD — #95 Type chunk carried verbatim; typed registers encode

Lane: #95, the encoder half of the #93/#94/#95 loader-fidelity family.
Defect as filed: `load_beam_chunks` dropped the `Type` chunk on the
floor while the compact-term writer refused any typed-register operand —
so an OTP 26+ module with `#tr{}` operands could not round-trip at all,
and a module WITHOUT typed registers silently lost its `Type` chunk on
re-emission. Fix shape ruled by Waffles (option 1): carry the raw chunk
bytes verbatim — no parsing, no rebuilding. Built and measured at my
hands, 2026-08-15; fix commit `7e318c0` on top of origin/main `bfaa85c`.

## What the fix is

1. `ParsedModule` gains `pub type_chunk: Option<Vec<u8>>`;
   `load_beam_chunks` captures `find_chunk(&chunks, b"Type")` verbatim
   (loader/load.rs). The chunk stays opaque to beamr's own loader on
   purpose.
2. `encode_module` emits the chunk byte-identical after `Line`
   (encode/container.rs) — VERBATIM, never rebuilt: typed-register
   operands index into these bytes, and only identical bytes keep those
   indices valid.
3. `AtomEncoder::new` gains `type_chunk_present: bool` (the per-module
   encode context); the `TypedRegister` arm in encode/compact.rs now
   ENCODES when the chunk is present — `push_unsigned(7, 5)` + register
   operand + unsigned type_index, the exact inverse of the decoder's
   extended-tag-5 — and refuses exactly as before when absent.
   `EncodeError::TypedRegisterWithoutTypeChunk` is now reachable only
   for CONSTRUCTED modules (typed registers with `type_chunk: None`), a
   combination no decoded module produces; its doc says so.
4. `first_mismatch` in tests/encode_round_trip.rs gains the type_chunk
   arm, and the committed-fixture ratchet now WALLS
   `assert!(refused.is_empty())` — a future regression that stops
   carrying the chunk goes RED instead of printing.
5. Tests +3: lib `typed_registers_round_trip_when_the_type_chunk_is_present`
   (plain + nested-in-list); integration
   `type_chunk_is_carried_verbatim_and_typed_registers_encode`
   (arbitrary chunk bytes, container-walk Type tag, `reloaded.type_chunk`
   byte-equal, full module equality) and
   `typed_registers_without_a_chunk_still_refuse` (refusal shape pinned:
   `instruction_index: Some(1), type_index: 3`).

`gleam-types`' `ParsedModule` is an unrelated struct, untouched.

## The no-teeth measurement (the ruling's return clause)

The ruling held a return to Waffles pre-landing IF the widening showed
teeth. Measured BEFORE the battery (ratchet-tally.log): **105 committed
fixtures, 0 refused** — the refusal denominator fell 55 → 0 exactly as
pre-registered, and ZERO fixtures flipped to Failed. The widening has no
teeth; per the ruling clause, no pre-landing return owed — the
measurement is stated here instead.

## Battery (canon 8-leg, 2026-08-15 16:14–16:22Z)

Runner: gate-logs/103/battery-RUNNER.sh, stdout to its own file.
Prediction pre-registered in PREDICTION.md and COMMITTED (`aed013b`)
BEFORE launch. Pre-battery check per the #78 law: BOTH clippy FULL legs
re-run AFTER the last edit — rc 0 both.

RESULT: GREEN — all 8 legs rc 0 AT THE PER-LEG TSV (the verdict; the
COMPLETE marker agrees but is not it). Measured axes vs the
registration, exact:

| leg                | predicted         | measured          |
|--------------------|-------------------|-------------------|
| tests              | 75 / 2123 / 0 / 0 | 75 / 2123 / 0 / 0 |
| tests-all-features | 75 / 2133 / 0 / 0 | 75 / 2133 / 0 / 0 |

(+3 each over the #96-corroborated 2120/2130 baselines; all three new
tests confirmed `ok` BY NAME in both leg logs.) Pin
aed013b9aabd3a0088b474c22fa16e121b1939e8 stable pre/post; tree census
0 modified pre and post. Battery logs: BATTERY.log (runner stdout),
legs.tsv, leg5.log + leg8.log (axes witnesses).
