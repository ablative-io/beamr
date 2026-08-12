# beamr #95 TYPED-REGISTER-GUARD — battery receipt

Base `d5f4f91` (= `origin/main`). Runner beside this file; six canon legs read
from the committed `gates.json` at run time, never transcribed.

## Verdict: 6/6 rc 0. COMPLETE marker DERIVED (legs_declared=6, legs_scored=6).

Tree check (three-artifact form, `wc` not `grep -c`): **6 pre, 6 post**,
unchanged across the run. Interpreter logged: `/usr/bin/python3`, Python 3.9.6.

## Axes — 73 / 2110 / 0 / 0, and the +3 was pre-registered

Base was **73 / 2107 / 0 / 0**. Passed rises by exactly **three**: the three
tests added in `loader::encode::compact` (`typed_registers_are_refused_by_name`,
`typed_registers_nested_in_a_list_are_refused`, and the control
`the_refusal_control_encodes_when_the_typed_register_is_absent`). Result-lines
stay at **73** — no new test binary, so no new result line. Both axes landed on
the prediction.

## The defect

`Operand::TypedRegister` carries a `type_index` into the module's `Type` chunk.
beamr's decoder **keeps the index and drops the table**; `encode_module` emits
no `Type` chunk. So every re-encoded module containing one had `Code` operands
pointing into a chunk that is not in the container.

⚠️ **It produced no error anywhere.** beamr's loader reads the index and ignores
the table, so the module round-tripped structurally identical and the ratchet
went green. Every other BEAM tool got a dangling reference.

**Measured scale: 55 of 105 committed fixtures** carry at least one typed
register — a majority of real modules, silently.

## The guard, and where it lives

The refusal is in `encode_operand`'s `TypedRegister` arm — the **single operand
writer** — not in a separate walk over `Instruction`'s **64** variants. Every
operand of every instruction passes through that match, including nested
`Operand::List` members which recurse into the same function, so coverage is
**by construction**. A hand-written walk would be a thing someone has to
remember to keep in step with the enum; this is the same reasoning that made
#94 a type change rather than an assert.

`encode_code_chunk` enumerates and calls `EncodeError::at_instruction`, which
attaches the position only the `Code` walk knows. The error names both the
instruction index and the `Type` entry.

## Falsifier — TWO arms, and the second one is the point

**Arm 1 — the guard is load-bearing.** Guard reverted in place, refusal arm
restored verbatim:

* `typed_registers_are_refused_by_name` — **FAILED**
* `typed_registers_nested_in_a_list_are_refused` — **FAILED**
* `the_refusal_control_encodes_when_the_typed_register_is_absent` — **passed**

The control staying green while both refusal tests go red is what separates
"the guard fires" from "the encoder is broken".

**⭐ Arm 2 — the pre-guard state was invisible, measured not asserted.** With
the guard still reverted, the committed-fixture ratchet reports:

    === committed-fixture ratchet: 105 fixtures, 0 refused (#95 typed registers) ===
    test result: ok. 1 passed; 0 failed

**55 modules were being written with dangling `Type` references and the suite
said everything round-tripped.** That is the silent fallback wearing a success
exit, demonstrated at the bytes rather than argued. It is why the guard lands
before the fix.

Tree restored after both arms; `loader::encode::compact` re-run green (8 passed)
before the battery was accepted.

## A second finding, and it is mine: the clippy leg had the same blind spot

The canon `clippy` leg ran **without** `--features beamr/encode`, so the entire
encoder was invisible to it — the **same defect** as the `tests` leg I fixed in
#93, one leg over. It was hiding a live `vec_init_then_push` warning that my own
#93 commit introduced.

⚠️ **I found "the encode suite never runs in the gate", fixed the tests leg, and
did not ask whether any other leg had the same blind spot.** The adjacent-leg
question is the one I skipped. Both are corrected here: the leg now passes the
feature, and the warning is gone. The amended leg was run and is rc 0.

## What this is NOT

⛔ **This is a guard, not the fix.** The fix — carry the raw `Type` chunk bytes
through `ParsedModule` and re-emit verbatim — is a separate build, and the
refusal disappears when it lands. **The 55 is the number that must fall to
zero**; `refusal.count` records it so that check is possible later.

Downgrading typed registers to plain registers was **rejected**:
`decode -> encode -> decode` structural equality is the encoder's whole ratchet,
and a lossy rewrite spends the instrument that catches the next regression to
buy a green run.

## Blast radius

`encode` is **not** a default feature and has **no in-tree caller** outside the
round-trip tests (measured: `grep` over `crates/` finds `encode_module` only in
`tests/encode_round_trip.rs`). External callers encoding OTP 26+ modules now get
a named error where they previously got bytes no other tool could read.
