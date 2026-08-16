# #219 (b)+(c) — battery prediction, PRE-REGISTERED

Written and committed BEFORE the runner fires. If a number below is wrong, the
finding is the discrepancy, not the number.

## The denominator MOVES this lane, and that is the point

`gates.json` now declares **9** legs, not 8. The runner reads `DECLARED` from
`gates.json` at run time (#75), so `COMPLETE` requires `SCORED == DECLARED == 9`.

⚠️ A battery that prints `COMPLETE (derived: 8/8)` this lane would mean the
runner read a STALE gates.json — that is a RED, not a pass, and it is exactly
the failure mode #75's derived-denominator design exists to catch.

## Per-leg rc: all nine expected 0

Leg 9 `nostd-ratchet` has already been run standalone at these bytes and returns
**rc 0** ("no-std errors 1039, exactly at the ceiling 1039"). ⚠️ Note that the
underlying `cargo check` exits **101** — the leg's rc is the RATCHET's verdict,
not cargo's. A leg rc of 101 in the .tsv would mean the script stopped
translating and is passing cargo's exit through.

## Axes: ALL THREE UNCHANGED AND EXACT

| leg | axis (result-lines/passed/failed/ignored) | prior | predicted |
|---|---|---|---|
| 4 wasm-tests | 2/86/0/0 | #115 | **2/86/0/0** |
| 5 tests | 76/2150/0/0 | #115 | **76/2150/0/0** |
| 8 tests-all-features | 76/2160/0/0 | #115 | **76/2160/0/0** |

**+0 DERIVED AT THE BYTES, NOT ASSUMED**: `git status --porcelain` shows **zero
`.rs` files** in the lane's census, and `git diff -U0 -- '*.rs'` contains **zero**
added or removed `#[test]` / `#[wasm_bindgen_test]` attribute lines. `--numstat`
is 11/1 + 8/1 + 2/1 across three files, none of them Rust.

⛔ `--no-ext-diff` is mandatory here: this repo configures a side-by-side
external diff pager, so a bare `git diff | grep` returns nothing and would have
produced a *false* zero. The zero above is real, not an artifact of the pager.

## Tree

`tree pre` must equal `tree post`. With #116's residue binned, the absolute
number is comparable lane-to-lane again; the five per-leg logs that carry no
axes get binned at lane close (now legs 1/2/3/6/7/**9** — see the runner's
amended note; leg numbers are positions and they moved when leg 9 landed).

## What would falsify the lane

- any axis differing from the table above (nothing Rust changed; a moved count
  means something built differently, not that a test was added);
- `DECLARED` reading 8;
- leg 9 rc 101 (cargo's code leaking through the ratchet's verdict);
- `tree pre != tree post`.
