# #219 (a) — dev-edge `default-features = false`: prediction, PRE-REGISTERED

Committed BEFORE the runner fires. A wrong number below is the finding.

## ⚠️ I TOLD WAFFLES THE AXES WOULD MOVE. THAT WAS WRONG, AND HERE IS WHY

At seq=39 I said "I'd expect the axes to MOVE there rather than hold". Then I
measured, before touching anything, and the reasoning does not survive:

- `crates/beamr-cli/Cargo.toml:16` — `beamr = { version = "0.18.0", path = "../beamr" }`,
  **no `default-features = false`**, so beamr-cli's edge carries the defaults;
- beamr is itself a **workspace member** (root `Cargo.toml:3`), so `--workspace`
  enables beamr's own default features directly.

Defaults therefore reach beamr by **two other paths** in every canon leg. Removing
`default` from the dev self-edge cannot subtract anything canon was building.

**MEASURED, not reasoned:** `cargo tree --workspace --features beamr/encode
-e features` gives a byte-identical beamr feature set before and after the edit
(`features-before.txt` vs `features-after.txt`, `diff` clean) — cooperative,
default, embedded, fs, jit, json, net, readiness, std, test-support, threads.

Correcting the expectation up front so it cannot be retro-fitted into a success
after the fact.

## Axes: ALL THREE UNCHANGED AND EXACT

| leg | prior (#219 @ 726607b) | predicted |
|---|---|---|
| 4 wasm-tests | 2/86/0/0 | **2/86/0/0** |
| 5 tests | 76/2150/0/0 | **76/2150/0/0** |
| 8 tests-all-features | 76/2160/0/0 | **76/2160/0/0** |

Leg 9 `nostd-ratchet` predicted rc **0** at **1039**, unchanged: `cargo check`
pulls no dev-dependencies at all, so this edit cannot move that number. If the
ratchet fires this lane, the edit did something I did not predict.

DECLARED stays **9**. `tree pre == tree post`.

## What the edit DOES change — the capability, measured both directions

`cargo tree -p beamr --no-default-features --features cooperative,json -e normal,dev`:

| | before | after |
|---|---|---|
| cranelift-jit, tokio, mio, num_cpus, zstd | **present** | **absent** |

That is the whole point: `--no-default-features` is now actually in force under
`cargo test`, where it was silently overridden before.

## The probe this finally makes possible — PREDICTION: UNKNOWN

`cargo test -p beamr --no-default-features --features cooperative,json` has
never been runnable as a genuine cooperative-only build. Now it is.

**I do not predict a pass.** Leaning RED: beamr's own suite has ~2150 tests that
have only ever been compiled with `threads` on, so integration tests naming
thread-gated APIs plausibly fail to build. A red there is a NEW FINDING about
test-suite portability, **not** a regression from this edit, and it is **not a
canon leg** — it must not gate this lane. Recorded as UNKNOWN in advance so
neither outcome can be dressed up as a confirmed prediction.

## Falsifiers
- any axis differing from the table (nothing Rust changed);
- leg 9 moving off 1039;
- the canon feature-set diff being non-empty;
- cranelift still present in the after-graph.
