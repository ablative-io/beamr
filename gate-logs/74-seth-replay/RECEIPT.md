# #74 SETH-REPLAY — battery receipt

## ⚠️ READ THIS BEFORE ANY RESULT BELOW

**The wrapper's exit status is not a verdict and must not be read.**
`battery-74.sh` ends on an `echo`, so it exits 0 regardless of what any leg
did; the only non-zero exits it can produce are the disk-threshold halt (9)
and a failed `cd` (20).

**This script's verdict channel is the per-leg `leg-N.rc` artifacts** — not
the wrapper's exit code, and not its stdout. That is the question to ask
*before* running any harness control (Waffles, 2026-08-06): what IS the
verdict channel? The answer decides which controls can bite. Here it means
controls 2, 3 and 4 are the ones that bite, and all three fired; control 1 is
**not applicable, not failed**. A wrapper exit code that no reader consults
cannot mislead a reader.

**This is a narrowing of my own earlier finding, and it makes my first
reading less alarming than I stated it.** The estate-wide pattern found on
2026-08-06 across four seats is real — a script exits with the status of its
last command, `set -e` does not catch it because the `echo` succeeds, and the
same trap eats `cmd | tee log`. It is the shell's default, not a habit. But
it is only a *defect* where the exit code is the verdict channel. I counted
this harness inside the pattern before asking that question; the receipt as
first committed overstated it, and this amendment is the correction. Nothing
about any measured result changes.

**Read only the per-leg `leg-N.rc` artifacts.** They are what this receipt
quotes. The question was raised mid-run and is recorded here as an
annotation; the script was NOT edited while it was running (zsh reads scripts
incrementally, so an edit under a live interpreter is its own corruption
class).

Related, and checked at the bytes rather than assumed: in zsh `status` is a
read-only alias for `$?`, so a wrapper-level `status=0` / `exit "$status"`
idiom does not do what it says. **It fails LOUDLY, not silently** — Apollo
measured it: the script dies at the assignment with `read-only variable` on
stderr and visibly stops. It does not run on to emit a wrong verdict. (That
is Waffles' correction to his own correction, 2026-08-06; his first telling
said "silently fails", which is wrong in the direction that matters, and this
receipt repeated it before the measurement arrived.)

Neither script uses it. Both capture into `rc`; the only occurrence of the
string `status` in either file is a comment about pipelines.

**The interpreter is established by the launch command, not by the shebang.**
A shebang is a claim about intent — the file mode and the launch command are
the facts, and a `#!` line selects nothing when the script is handed to an
interpreter by name. Both were launched explicitly as `zsh <path>` (recorded
verbatim in this session's transcript), and both are mode 0755. So zsh is the
interpreter by the fact, not by the claim, and the read-only-`status`
behaviour above is the applicable one. This harness is unaffected.

**Every green below was provisional until control 2 fired.** It fired, and it
passed in both directions. The four controls come first for that reason.

## The four controls (2, 3, 4 FIRED — both directions where applicable; 1 N/A)

| # | Control | Question | Result |
|---|---|---|---|
| 1 | Wrapper exit | Can the wrapper's status go non-zero if a leg fails? | **NO — and NOT APPLICABLE, not failed.** The wrapper's exit code is not this script's verdict channel; the per-leg rc artifacts are. No reader consults it. |
| 2 | Leg rc capture | Can a leg's RECORDED rc go non-zero when capture sits behind a function and an `eval`? | **YES.** `leg-c1.rc = 7`, `leg-c2.rc = 0`, both pre-chosen. |
| 3 | Leg count | Does the recorded leg count match the declared one, asserted before any rc is read? | **6 declared = 6 recorded.** No stage silently absent. |
| 4 | Verdict parse | Does the matcher match the runner's REAL output, including a failing run? | **YES.** Run against `red.log`: reads 26 passed / 3 failed, the independently known answer. Negative arm returns 0/0 and does not read as a pass. |

Control 2 is run by `control-74.sh`, built by **copying** `battery-74.sh` —
not reimplementing it — with only the leg list replaced. `run_leg` is proven
byte-identical between the two files: 20 lines, sha256 prefix
`020ee16a87262891`.

**A control exercises the path it travelled, never the path you meant.** Red
here did NOT go through the harness (it was a direct command), so red is
*not* a control on `run_leg`; that is exactly why control 2 exists.

**First attempt at control 2 failed and is disclosed rather than hidden:** the
command was `exit 7`, which `eval` runs in the *current* shell, killing the
script before any rc was written. That models nothing — real legs are external
commands that fork. Corrected to `sh -c "exit 7"`, and `run_leg` was re-proven
byte-identical after the edit. The failure was in my control command, not in
the harness.

## Red — measured at my own hands, BEFORE green

Tree `49ab8cf` (walls only, fix absent). `cargo test -p beamr-cli`.
`red.rc = 101` — **26 passed / 3 failed / 0 ignored.**

The three failures are exactly the three walls, by name:

- `replay_of_a_log_recorded_against_different_behaviour_fails`
- `replay_of_a_log_with_no_drivable_events_fails_loudly`
- `replay_refuses_dir_flag`

**The defect states itself in the red log, verbatim:**

```
replay must not succeed with nothing to drive;
got WithExitCode { stdout: "STORED TRANSCRIPT THAT MUST NEVER BE PRINTED AS THE ANSWER\n", exit_code: 0 }
```

The runtime handed back the stored transcript and exited 0, on a log carrying
zero drivable events.

**And in that same run:**

```
test tests::record_then_replay_fixture_preserves_stdout_and_exit_code ... ok
```

The always-green test passing beside the three walls that caught the defect
immediately. That pairing is the single most useful artifact in this lane: a
test that cannot fail is not weak coverage, it is an active endorsement of the
defect it covers.

## Green — 6-leg canon at `493f5b8`

Legs verbatim from `gates.json`. Per-leg rc, quoted from the artifacts:

| Leg | Name | rc | Boundary df (GiB) |
|---|---|---|---|
| 1 | fmt | 0 | 99.76 |
| 2 | clippy | 0 | 99.76 |
| 3 | wasm32-check | 0 | 99.18 |
| 4 | wasm-tests | 0 | 98.89 |
| 5 | tests | 0 | 98.35 |
| 6 | blocking-call-in-native-bif | 0 | (final 94.5x) |

Threshold 40 GiB available (`df -k /System/Volumes/Data`, avail/1048576);
every boundary echoed what it read; hard halt if under. Leg 6 is never piped
(gates.json's own note: no pipefail here, so a pipe would report the
downstream tool's status instead of ast-grep's).

### Axes — pre-registered BEFORE green ran, matched EXACTLY

`EXPECTATION.txt` was written before the battery started.

| Axis | Expected | Measured |
|---|---|---|
| result-lines | 72 | **72** |
| passed | 2067 (2064 + exactly three) | **2067** |
| failed | 0 | **0** |
| ignored | 0 | **0** |

The +3 was **derived, not guessed**: the walls commit adds three tests; the
fix commit *renames* one (net zero); CHANGELOG and string commits add none.
Corroborated independently at my own hands — beamr-cli's own binary went
26 → 29.

Failure/panic greps, unpiped, rc reported: `^failures:` 0 hits (rc=1),
`panicked at` 0 hits (rc=1).

The three walls pass by name in the green run, the replacement test
(`record_writes_a_loadable_log_and_replay_refuses_to_reprint_it`) passes, and
`record_then_replay_fixture_preserves_stdout_and_exit_code` is **absent** —
the always-green test is gone from the tree.

## Blast radius — measured, with a denominator and a positive control

`BLAST-74.txt`. Population `/Users/tom/Developer/ablative`,
**denominator 157,534 files.** Exclusions named there rather than left implicit.
Positive control (`"replay".to_owned()` → beamr-cli tests) fired, and was
re-fired under every narrowing of the window.

**Result: the caller set is EMPTY.** No script, CI job, runbook or crate
invokes `beamr replay`. Five patterns including indirect shapes
(`-- replay`, `$VAR replay`, `bin/beamr replay`). Every non-beamr hit is a
*different* "replay" — a UI palette entry, a WebSocket message type, an
unrelated `replay_events` helper, a crate keyword, a config comment. In-beamr
hits are the USAGE string, a brief describing the command, and the CLI tests.

**Nothing goes red anywhere when this lands.**

## Teardown

du before 6,282,040 KiB → after 4,980,512 KiB; reclaimed **1,301,528 KiB**.
`target/debug/incremental` was *attributed* 2,050,524 KiB but only 1,301,528
came back — 748,996 KiB of blocks were shared with dependency objects and
moved rather than freed. **du is attribution, not reclamation.** df
corroborates: 94.51 → 95.75 GiB, +1.24 GiB, agreeing with the du delta.
Peak target footprint 5.99 GiB.
