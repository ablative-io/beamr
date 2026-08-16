# The `nostd-ratchet` leg — design, and why it is strict in both directions

Ruled by Waffles at seq=38: `(b)` the ratchet and `(c)` the doc correction ride
as one small lane; ratchet at rustc's own 1039, direction DOWN only, with the
counting rule written into the leg so the next instrument joins correctly.

## What it holds

`cargo check -p beamr --no-default-features` is red and its redness is a
deliberate, recorded waiver. This leg does not un-waive it. It holds the
**number**, which nothing was holding: 1019 at `ec5d7f8` → 1039 at `d4a82e5`,
+20 across five minor versions, unremarked.

## The counting rule, and the off-by-one that has now caught two seats

Use rustc's own `due to N previous errors` line. Do **not** count `^error`
lines: the trailing `error: could not compile …` summary line is itself an
`error` line, so a naive grep reads exactly one too many.

- the historical record says **1020**; rustc at that same pin says **1019**
- my own first reading of HEAD said **1040**; rustc says **1039**

Both off by one, in the same direction, for the same reason. **Neither seat
miscounted anything about the world.** Whenever a figure from this leg appears
beside a historical one, state that reconciliation — otherwise the next reader
concludes someone was sloppy, and the accusation outlives the correction.

## Strict in BOTH directions — my call, stated so it can be overruled

An **improvement also fails** (rc 2), demanding `CEILING` be lowered in the same
commit. This is deliberate: a ratchet that tolerates slack is not a ratchet.
Leave the ceiling above the true count and the number is free to drift back up
to it silently — which is precisely the failure this leg exists to prevent.

It **REFUSES** (rc 3) rather than passing when the tally line is absent. A green
from an instrument that could not measure is worth nothing: same value whether
the tree is sound or the parse is broken.

If no-std ever compiles clean, the leg **passes loudly** and asks for the
ceiling to drop to 0 and the leg to be reconsidered — a changed world should not
slide through as an ordinary green.

## Self-test — 7/7, `✅ every arm fired`

Arms exercise a pure `verdict()` on **minted** inputs, so no arm depends on the
tree currently being broken — the #115 lesson, where three self-test arms died
the moment the defect they fed on was fixed. A control confirms the parser reads
rustc's real wording (→1039) and, in the other direction, that it **invents
nothing** when the anchor is absent.

## Falsifiers on the LIVE instrument (`falsifiers.log`)

Synthetic arms prove the logic; only a real perturbation proves the gate is
sensitive to the actual measurement. Both run a real `cargo check`:

| arm | ceiling | expected | got |
|---|---|---|---|
| M1 ceiling below truth | 1038 | FAIL rc 1 (breach) | **KILLED** |
| M2 ceiling above truth | 1040 | FAIL rc 2 (improvement) | **KILLED** |
| control, unperturbed | 1039 | PASS rc 0 | **rc 0** |

## What this leg does NOT fix

`--no-default-features` remains **inert under `cargo test`** — the dev
self-dependency at `crates/beamr/Cargo.toml` carries default features and cargo
unifies across graph edges. That is a separate lane, ruled: it changes what
every `cargo test` builds, and it interacts with the finding that legs 5/8
already compile `cooperative` and `threads` together. Not a rider.
