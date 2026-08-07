# RF-006 — VERIFICATION LEGS AT CALLY'S HANDS

**Seat:** Cally Ray (verifier; RF-006 implemented by Artemis Peach).
**Subject:** branch `artemis/rf-006`, **HEAD `0113e31`**. Clock `2026-08-07T10:42:31Z` → `10:47:29Z`.
**Result: ALL THREE LEGS GREEN.**

## The subject is HEAD, not the held hash — and that is a measurement

The lane held `17295f8` ("test(jit): RF-006 mutation evidence"). At run time HEAD was `0113e31`,
**13 commits ahead**. Rather than re-derive the named base more carefully, the dependence on it was
removed: the legs ran at HEAD and are cited at HEAD.

The two are the same *compiled* subject, and that is measured rather than assumed —
`git diff --name-only 17295f8..HEAD` is **24 files, extensions `md/py/json/txt/rc/err` only, zero
`.rs`, zero `Cargo.toml`, zero `Cargo.lock`**. All 13 commits are `docs(rf-006)` / `docs(ar-1)`.

## Legs

| leg | command | rc | artefact |
|---|---|---|---|
| fmt | `cargo fmt --all` | **0**, tree unchanged | `cally-fmt.rc` |
| clippy baseline | `cargo clippy --workspace --all-targets -- -D warnings` | **0** | `cally-clippy-baseline.{txt,rc}` |
| **clippy control** | same, with an injected lint | **101**, 3 diagnostic mentions | `cally-clippy-control.{txt,rc}` |
| suite | `cargo test --workspace` | **0** — **2075 passed, 0 failed, 0 ignored, 72 suites** | `cally-test-workspace.{txt,rc}` |

`battery.sh` was **not** used and its rc was **not** consumed — it is MARKED-SPLIT. Legs were run
directly.

### The clippy green is validated, not performed

A green from an unvalidated instrument is indistinguishable from a wrong one, so the baseline is
not reported alone. An unused-variable violation was appended to `crates/beamr/src/lib.rs`, clippy
re-run under the identical flag string, and it went **rc=101 naming the injected symbol 3 times**;
the file was then reverted with `git checkout --` and the tree confirmed clean. **The circumstance
in which this check fails is named, and it failed there.**

## ⚠️ CLEAR-AT-ENTRY, NOT HELD-CLEAR-THROUGHOUT — and the reason is a defect in the #154 condition

| moment | 1-min | 5-min | verdict |
|---|---|---|---|
| entry `10:42:31Z` | 5.09 | 9.23 | **CLEAR** (10 cores, 52 GiB free vs 34 band, cargo/rustc 0) |
| mid-run `10:45:15Z` | 15.95 | 12.42 | above gate |
| exit `10:47:29Z` | 19.84 | 17.51 | above gate |

The mid-run and exit readings are **above the threshold, and the load is my own legs.** This run is
therefore recorded as **CLEAR-AT-ENTRY**. It cannot be recorded as HELD-CLEAR-THROUGHOUT, and not
because contention appeared.

⭐⭐⭐ **A MID-RUN LOAD RE-CHECK CANNOT DISTINGUISH THE BATTERY'S OWN LOAD FROM A COMPETING SEAT'S.**
`loadavg` is a scalar over the whole box; a battery that saturates 10 cores is byte-indistinguishable
from a rival seat doing the same. So the mid-run re-check condition ruled onto **#154** is **not
implementable with loadavg alone** — as specified it would abort every honest battery it was meant
to protect, and its refusals would be self-inflicted 100% of the time.

The condition needs a discriminator, not a second reading: attribute load to the *runner's own
process tree* (subtract own descendants) and re-check only the **remainder**. Until that exists,
**#154's mid-run term is a prose row wearing a mechanism's clothes** — it can be written, it cannot
fire correctly. Recorded against #154; not fixed here.

Related: [[feedback_control_of_controls]], [[feedback_merge_base_law]],
[[feedback_box_exclusivity_precondition]].
