# B-144 R1 — axes pre-registered BEFORE the battery (ruled term 4)

Prediction written before the run, at the fix bytes.

## AXES — PREDICTED EXACTLY UNCHANGED

| leg | name | result-lines / passed / failed / ignored |
|---|---|---|
| 4 | `wasm-tests` | **2 / 86 / 0 / 0** |
| 5 | `tests` | **76 / 2150 / 0 / 0** |
| 8 | `tests-all-features` | **76 / 2160 / 0 / 0** |

**Reasoning, stated so it is falsifiable.** The fix removes `#[cfg(any(threads,
cooperative))]` from two modules and one `From` impl, and makes `crossbeam-queue`
non-optional. Every canon leg builds with `threads` on — directly, via
`beamr-cli`'s defaults, or via `--all-features`. In all of those configurations
`timer`, `replay` and the `From` impl were **already compiled**, and
`crossbeam-queue` was **already present**. The fix therefore changes nothing any
canon leg compiles, adds no test, and removes none.

⇒ **Any axis movement at all is unexplained and blocks the lane.** There is no
"expected small drift" here; the prediction is exact.

## LEG 9 — THE ONE LEG THAT DOES MOVE

`nostd-ratchet` at the ruled re-pin: tally **1075**, ceiling **1075**, rc **0**
("exactly at the ceiling. Debt held."). Verified before the battery, and the
self-test remains **7/7, every arm fired** — including the parser positive
control and the no-anchor REFUSE arm, neither of which depends on the tree being
broken in any particular way.

## THE R1 CRITERION ITSELF

`cargo check --target wasm32-unknown-unknown -p beamr --no-default-features`
→ **rc 0**, and F4 `cargo check -p beamr --no-default-features --features std`
→ **rc 0**, both re-measured at the fix bytes rather than carried from the probe.
