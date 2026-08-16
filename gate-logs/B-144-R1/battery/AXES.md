# B-144 R1 battery — pin `2bddceb`, COMPLETE 9/9

## VERDICT

`COMPLETE (derived: 9/9, pin stable)`, all nine legs rc 0, pin post == pin pre.

## AXES — EXACT TO THE PRE-REGISTRATION, ALL THREE

| leg | name | predicted | measured |
|---|---|---|---|
| 4 | `wasm-tests` | 2 / 86 / 0 / 0 | **2 / 86 / 0 / 0** ✅ |
| 5 | `tests` | 76 / 2150 / 0 / 0 | **76 / 2150 / 0 / 0** ✅ |
| 8 | `tests-all-features` | 76 / 2160 / 0 / 0 | **76 / 2160 / 0 / 0** ✅ |

Predicted exactly unchanged, and unchanged. Every canon leg builds with `threads`
on, where all four edits were already in effect, so the fix changes nothing any
canon leg compiles. Leg 9 `nostd-ratchet` rc 0 at the ruled re-pin: *"1075,
exactly at the ceiling. Debt held."*

## ⚠️ TREE PRE 1, POST 11 — ACCOUNTED EXACTLY, NOT WAVED THROUGH

The runner asserts `tree pre == tree post` within a run, and here it did not hold.
Fully attributed:

```
pre  =  1   gate-logs/B-144-R1/battery/  (created by my output redirect)
new  = 10   <pin>.tsv + <pin>.leg1..9.log
post = 11
```

`OUT="$1"` was a bare relative name and the runner `cd`s to the repo root, so the
artefacts landed there rather than beside `BATTERY.log`. Every one of the 11 is a
battery artefact; **no source file was modified during the run**, which is what
the assertion exists to detect. Artefacts moved into this directory afterwards,
legs 1/2/3/6/7/9 binned per the ruled retention rule (they carry no axes; the
`.tsv` holds their rc, and legs 2/7 are clippy JSON full of absolute operator
paths).

## ⛔ THE FIRST INVOCATION OF THIS BATTERY RAN NOTHING AND LOOKED GREEN

Recorded because it is the sharpest live instance of a banked law I have hit.

I first ran the runner **without its `$1`**. It hit `$1: unbound variable` under
`set -u`, executed **zero legs** — and the surrounding harness reported
**"completed (exit code 0)"**.

**The runner did not exit 0.** I had piped it through `tail -25`, and without
`pipefail` a pipeline's status is the *last* command's. `tail` succeeded, so the
runner's failure was replaced by a 0 before anything could see it. Verified:
`bash -c 'exit 42' | tail -1` ⇒ **0**.

Two banked rules met in one command:

- ⛔ **NEVER PIPE A LONG INSTRUMENT RUN THROUGH `tail`** — already in my shell
  hazards, and I did it anyway.
- ⭐ **THE MARKER IS AUTHORITATIVE, THE EXIT CODE IS NOT.** The only thing that
  contradicted "completed, exit 0" was opening the file and finding no `COMPLETE`
  marker, no pin line, and no legs.

Had I taken the exit code, I would have re-pinned a canon gate's ceiling by +36 on
the strength of a battery that never ran a single leg, and the evidence file would
have contained one error message.

⭐ **A WRAPPER CAN LAUNDER AN INSTRUMENT'S VERDICT WITHOUT TOUCHING THE
INSTRUMENT.** The runner behaved perfectly — `set -u`, error, non-zero. Everything
that went wrong happened *between* the instrument and the reader. When a result
looks clean, check the path the verdict travelled, not just the thing measured.
