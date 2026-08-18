# Release 0.19.0 — results at the bytes

## Published set (verified at the sparse index, an instrument independent of the publisher)

| crate | version | checksum (sha256) | beamr req |
|---|---|---|---|
| beamr | 0.19.0 | `212fa650c7fb7d09b031c8dedef0a33207c4755e580df2cfd0640f17792973c7` | — |
| beamr-wasm | 0.9.0 | `a338e4003575721c9d6e7fa72ede21fe04935dd081e50bba54e00f28dc7ec3e5` | `^0.19.0` |
| beamr-cli | 0.6.0 | `18386c2541b8cb38030d4a0ffdc4d88399661402bfee02c9ec6c292a7d52186f` | `^0.19.0` |

Downstream pinning is by these checksums, not by version number (the 0.8.2
reuse trap made that the chain's standard).

## Authority chain

Tom's release word (meridian 2bf2782d, 2026-08-18T04:14Z, relayed verbatim by
Waffles) → Waffles' three rulings (4c98c887): 0.19.0 confirmed (0.18.3 refuted
at the bytes — two breaking entries), #104 demote-to-test-support, #95 carry to
0.20.0 with stated trigger. cli riding confirmed inside the declared window
(60d34834). Tags: NONE cut — the tag-namespace question (v0.9.0 collides with
the distribution-era tag at 30c41ae) is with Cally; publish sequenced
tag-after-ruling per Waffles.

## Battery evidence form

- Legs 1–8: measured at `c991622`. COMPLETE, all rc 0, axes EXACT against the
  pre-registered prior (leg4 2/86/0/0 · leg5 76/2150/0/0 · leg8 76/2160/0/0 —
  prediction written in CONTENTION.md BEFORE the run).
- Leg 9 (nostd-ratchet): went red at `c991622` in the IMPROVEMENT direction —
  tally 1072 vs ceiling 1075. The gate is strict two-sided by design. Ceiling
  re-pinned to 1072 at `feb8148` (reason in-instrument, self-test 7/7), leg 9
  re-run there: rc 0, exactly at ceiling.
- The `c991622..feb8148` delta is `scripts/gate-nostd-ratchet.sh` ONLY
  (`git diff --stat` in the re-pin commit): compiled bytes identical, so legs
  1–8 at c991622 are the evidence for the released bytes. No full re-run was
  spent on a loaded box for zero information; this paragraph is the
  declaration of that shortcut.
- Run contended; timings are NOT price points (CONTENTION.md).
- The Δ3 ratchet improvement is attributed at COMMIT level (4e8ccf6) only; the
  per-function split was deliberately not invented.

## Dry-run gate (the extraction instrument)

Every publish was preceded by `cargo publish --dry-run` (logs in this dir).
beamr-wasm's dry-run — the instrument that killed the first cut at a3b87e6
with four E0599s against registry 0.18.2 — compiles registry beamr 0.19.0
clean. That first-cut evidence is preserved unmodified at
`gate-logs/227b-wasm-0.9.0/`.

## Carve-out retirement (for the haematite/aion chain)

Ruling r2's carve-out named "beamr-wasm 0.9.0's publication" as its retirement
trigger. 0.9.0 is now published — but carrying `^0.19.0`, so a haematite
release admitting beamr **0.18.x** still cannot unify its wasm rung. The
trigger fires in letter, not mechanism. Flagged on the aion/builds row: the
clause should re-word to "a beamr-wasm release unifiable with the native
beamr pin", or haematite admits 0.19.x directly (Tom's update-everything word
points there).
