# beamr 0.17.1 — release battery receipt

Pin: commit `f6d1fe8`, branch `artemis/backport-0.17.1`, base `v0.17.0`
(`377b6de`). Runner: `runner.sh` beside this file. Six canon legs read from the
committed `gates.json` **at run time**, never transcribed.

## Verdict: 6/6 rc 0. COMPLETE marker DERIVED (legs_declared=6, legs_scored=6).

fmt 0 · clippy `-D warnings` 0 · wasm32-check 0 · wasm-tests 0 · tests 0 ·
blocking-call-in-native-bif 0.

Tree check (three-artifact form, counted with `wc`, never `grep -c`):
**0 pre, 0 post.** Filter declares two exclusions by name: the known untracked
`.claude/skills/` and this battery's own `gate-logs/0171-release/` artifacts.

## Axes — measured, then reconciled by NAME

Measured at `f6d1fe8`: **result-lines 73 / passed 2075 / failed 0 / ignored 0.**

| axis | base `v0.17.0` | delta | expected | measured |
| --- | --- | --- | --- | --- |
| result-lines | 72 | +1 new test binary | 73 | **73** |
| passed | 2067 | +8 named below | 2075 | **2075** |
| failed | 0 | — | 0 | **0** |
| ignored | 0 | — | 0 | **0** |

### The +8, enumerated rather than counted

⭐ **A cardinality match is the weakest form of agreement**, so the delta is
named. Eight `#[test]` functions are added by `4db3ca6` and nothing is removed:

Six helper-level walls (`jit/runtime_message.rs`):
`compiled_self_send_ticks_sender_clock_and_delivers` ·
`compiled_cross_process_send_routes_through_local_send_facility` ·
`replay_mismatch_restores_clock_and_aborts_with_interpreter_error` ·
`non_pid_destination_aborts_with_the_interpreter_badarg` ·
`missing_facility_falls_through_exactly_like_the_interpreter` ·
`remote_destination_without_distribution_aborts_with_noconnection`

Two integration arms (`tests/jit_send_delivery_gate.rs`, the new binary that
accounts for the +1 result-line): `jit_compiled_cross_process_send_delivers` ·
`interpreted_cross_process_send_delivers_control`

### ⚠️ Provenance of the base prior — stated, not glossed

**I did NOT re-run the suite at `377b6de`.** The base `72 / 2067 / 0 / 0` is
**inherited** from the committed `gate-logs/75-ci-gate/RECEIPT.md`
(corroborated by `gate-logs/74-seth-replay/RECEIPT.md`). ⭐ *A prior is a claim
until its applicability to THIS base is checked*, so both checks were run:

* `git merge-base --is-ancestor b96be7a 377b6de` → **true**; the receipt's
  commit is an ancestor of this base.
* `#[test]`-attribute census across `b96be7a..377b6de` → **added 0, removed 0,
  net 0**; nothing moved the test population between the receipt and the base.

⚠️ **Bound on that second check:** it counts the `#[test]` attribute only. It
does not see doc-tests or non-`#[test]` harness attributes, so it bounds the
unit-test population and not the whole denominator. The exact arrival of the
expected 2075 is what makes the inherited prior credible; the census alone
would not.

## The falsifier — the gate is shown to SEE the defect AT THIS BASE

⛔ **A green gate beside a fix proves nothing.** Two-arm run at this tree:

| arm | tree | compiled arm | interpreted control |
| --- | --- | --- | --- |
| **subject** | `f6d1fe8` (fix present) | **ok** | ok |
| **falsifier** | six production files reverted to `377b6de`, gate retained | **FAILED** — *"receiver never got the cross-process message (silent drop under COMPILED execution)"* | **ok** |

The interpreted control staying green in both arms is what makes the compiled
failure attributable to the compiled path rather than to a broken fixture.

## Port fidelity

`git cherry-pick -n 0a5d1ac` applied **all 7 files with ZERO conflicts** (one
auto-merge, `interpreter/opcodes/messaging.rs`). **All six production files
compile against this base unchanged** — the load-bearing fact, because the fix
routes `LocalSendFacility` through the services pointer in `JitRuntimeContext`,
and that seam had to already exist at `0.17.0`. It does.

**One adaptation, and it is meaning-preserving.** `Scheduler::new` takes no
native-BIF argument at this base and constructs an empty `BifRegistryImpl`
internally; on the 0.18.x line the same emptiness is stated explicitly as
`NativeBifs::none()` (a #86 artefact — `NativeBifs` did not exist before
0.18.0). The gate's native surface is therefore unchanged, not weakened.

## An instrument failure worth recording, because it exited 0

The **first** run of this runner scored every leg rc 0 while executing
**nothing**: bare `python3` resolved to a broken shim, so leg names and commands
came back empty and `bash -c ""` returned 0. **The script exited 0.**

⭐ It was caught by the **DERIVED COMPLETE marker**, which read
`INCOMPLETE legs_declared= legs_scored=2` — the denominator, not the exit code.
Runner hardened before the real run: `/usr/bin/python3` absolutely; **ABORT** if
the leg count fails to parse (a zero-trip loop scores nothing and reports all
green); **ABORT** if the tests leg produced no log rather than reporting axes
from a run that never happened.

## What this release does NOT claim

* **The compiled-frame identity fix is NOT backported.** On this line a compiled
  frame still takes its module from the pinned `Arc<Module>`. That is a
  diagnostic defect; `0.18.2` carries the fix.
* **AR-1 remains OPEN and unnarrowed at this base**, exactly as disclosed for
  `0.17.0`.
* **No production incident is explained.** The `json:encode_integer/1`
  non-integer after 99 clean cycles remains **unsolved**.
* **Delivery is the whole point of this release**: a `^0.17` requirement cannot
  resolve `0.18.2`, because each `0.x` minor is a semver major.
