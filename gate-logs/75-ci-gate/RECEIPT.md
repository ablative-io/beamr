# #75 CI-GATE — battery receipt

## ⚠️ READ THIS BEFORE ANY RESULT BELOW

**This lane ships a GitHub Actions workflow, and in Actions a step's verdict
channel IS its exit code.** That is *not* inherited from the last harness I
built: `battery-74.sh`'s verdict channel was its per-leg `rc` artifacts and its
wrapper exit code was explicitly unreadable. Adding CI as a consumer is exactly
what makes an exit code load-bearing, so the question has to be re-asked at
every new consumer rather than carried over (Waffles' question zero,
2026-08-06).

**And the exit code alone is not enough.** The per-leg rc files are written,
kept, uploaded as an artifact, and asserted *against a declared count* before
any verdict is taken. A runner that dies mid-loop leaves a truncated set, and a
truncated set of zeros reads exactly like a clean run. **Count first, then
read.**

**Carriers are not a second instrument.** An exit code, a summary line and N
named greps that all read the same driver's PASS determination are three
consumers of one instrument in one fault domain (Cally's correction to me,
2026-08-06). The per-leg rc files here are written by the loop itself, one file
per leg, and the count is taken from `gates.json` — a different source from the
loop that fills them.

## What is actually new, measured rather than asserted

`.github/workflows/ci.yml`, one job, the six `gates.json` legs.

**THE LEGS ARE NOT RESTATED IN THE WORKFLOW.** They are read out of
`gates.json` at run time with `jq`, so CI cannot drift from the canon by
transcription. A workflow that restates its legs is a second copy of the truth,
and the two copies disagree the first time somebody edits one of them.

### Census: what did main's CI already cover?

`leg-census-vs-existing-ci.txt`. Main carries exactly one tracked workflow,
`cooperative-wasm.yml`. Instrument: each leg's `cmd` extracted from
`gates.json` by `jq`, then `grep -qF` against that workflow.

| Leg | Name | Run by pre-existing CI? |
|---|---|---|
| 1 | fmt | **ABSENT** |
| 2 | clippy | **ABSENT** |
| 3 | wasm32-check | **ABSENT** |
| 4 | wasm-tests | **ABSENT** |
| 5 | tests | **ABSENT** |
| 6 | blocking-call-in-native-bif | **ABSENT** |

**0 of 6.** Positive control fired: the same instrument against `gates.json`
itself, where every command must be present by construction, reports 6/6. So
the instrument *can* say PRESENT, and the zero is a finding rather than a
broken grep.

**Near-miss disclosed rather than left for a reader to trip over:** a
*substring* search for `cargo test --manifest-path crates/beamr-wasm` does hit
line 35 of the tracked workflow. That is not leg 4. Leg 4 version-checks the
runner first and pins `--locked`. Similar shape, different command — which is
the whole reason the census matches the leg's command verbatim instead of
eyeballing intent.

## The six verdict controls — ALL FIRED, ALL DISCRIMINATE

**The scripts under control were EXTRACTED FROM THE SHIPPED YAML, not
retyped.** A control that runs a hand-copy of the code proves the hand-copy
works. The extractor itself is in evidence as `extract.rb`, and the extracted
files carry their sha256 in `extracted-scripts.sha`:

- `canon.sh` — `14ec947e4bcf251c…`
- `verdict.sh` — `7d956a5f9dfe61b1…`

| # | Control | Set-up | rc | Reads as |
|---|---|---|---|---|
| C1 | all six legs zero | complete, all-zero | **0** | pass |
| C2 | **one leg rc=7** | complete, one non-zero | **1** | **fail — the load-bearing direction** |
| C3 | five of six recorded | truncated set, all zeros | **1** | fail (count mismatch) |
| C4 | no `gate-rc` at all | step never reached leg 1 | **1** | fail |
| C5 | declared 6, zero recorded | canon step died before any leg | **1** | fail (count mismatch) |
| C6 | **two-arm, on the canon loop** | leg that leaks a `cd` | **1** both arms | see below |

**C2 is the one that matters for the verdict.** A verdict harness that cannot
go red is not a gate, and every green above it is provisional until C2 fires.
It fired.

**Disclosed against myself:** C5 exercises the *mismatch* branch, not the
`recorded -eq 0` branch — the mismatch check catches it first and returns
before the zero check is reached. That guard is therefore reachable by exactly
one population: a `gates.json` that declares **zero** legs, where declared and
recorded agree at 0 and the mismatch check passes. It is still load-bearing —
"an empty canon is not a pass" — but for a narrower population than C5's name
suggests, and I would rather say so than let the table imply a control fired on
a branch it never touched.

## C6 — the control that changed the workflow

Cally relayed a point from Hermes on 2026-08-06: **where a gate sits relative
to its siblings is part of its correctness, not its performance.** I took that
to my own loop and found a real coupling.

`eval "$cmd"` runs in the **current** shell. A future `gates.json` leg shaped
`cd crates/foo && cargo test` would therefore leave every *later* leg running
from the wrong directory. The fix is one character-pair — `( eval "$cmd" )` —
and it makes the legs genuinely independent, so the verdict does not depend on
leg order.

**C1–C5 could not cover this**, because all five exercise `verdict.sh` and the
change is in `canon.sh`. A control exercises the path it travelled, never the
path you meant. So C6 is two-armed, fixture: leg 1 leaks a `cd`, leg 2 asserts
the cwd survived, leg 3 exits 7.

- **Fixed arm (shipped bytes):** cwd intact for later legs, and `rc=7` still
  captured *through* the subshell.
- **Unfixed arm:** worse than a failed assertion. The leaked `cd` broke the
  **loop itself** — every later `jq … gates.json` failed, so legs 2 and 3 got
  an empty name and an empty command, ran nothing, and were **reported
  `rc=0`**. Verbatim: `LEG 2 () rc=0`. Two legs that never executed, reported
  as passing.

**What caught it was not the loop — it was the denominator assertion.** The rc
writes failed too, so recorded=0 against declared=3 and `verdict.sh` refused to
produce a verdict at all. That is the first time in this lane the denominator
check has bitten on a *real* fault rather than a hand-built fixture, and it
shows the two mechanisms are independent: the subshell removes the corruption
at its source, the denominator check catches a corruption whatever its source.
Neither substitutes for the other.

**Honest bound:** in this control the rc writes *happened* to fail, which is
what made the corruption loud. I have not established that every possible cwd
leak fails loudly — only that this one did, and that the subshell prevents the
class regardless.

**Re-verification after the edit:** `verdict.sh` re-extracts to
`7d956a5f9dfe61b1…`, **byte-identical** to the version C1–C5 ran, so those five
still bind. `canon.sh` differs from the controlled version by the subshell hunk
and nothing else (`diff` in evidence).

## Green — the shipped `canon.sh` run against the real tree

Tree `d9f92bc`. This run is **simultaneously** this lane's 6-leg battery and a
live test of the workflow's own canon step: the bytes that ran are the bytes
that ship.

| Leg | Name | rc |
|---|---|---|
| 1 | fmt | **0** |
| 2 | clippy | **0** |
| 3 | wasm32-check | **0** |
| 4 | wasm-tests | **0** |
| 5 | tests | **0** |
| 6 | blocking-call-in-native-bif | **0** |

Declared 6 = recorded 6. **`verdict.sh` rc = 0.** Every rc above is quoted from
its `gate-rc/<name>.rc` artifact, not from the log.

**The battery was run TWICE, and the reason is a claim I refused to weaken.**
Run 1 finished before the subshell edit. That edit made "the bytes that ran are
the bytes that ship" *false*, so rather than downgrade the sentence I re-ran the
whole battery on the shipped post-subshell bytes. Run 1 is retained as an
independent cross-check. **Both runs agree on every axis and every leg rc.**

Boundary `df`: 90.86 GiB at start, 85.79 before the re-run, 85.14 final, against
`THRESHOLD.txt` 40 GiB.

### Axes — a CONFIRMATION, not a pre-registration

**Stated plainly because the distinction is the whole value of the axis
discipline:** there is no `EXPECTATION.txt` for this lane. The prior is #74's
landed measurement at this same tree, and this battery re-measures it.

| Axis | Prior (#74, landed) | Run 2 (shipped) | Run 1 (cross-check) |
|---|---|---|---|
| result-lines | 72 | **72** | 72 |
| passed | 2067 | **2067** | 2067 |
| failed | 0 | **0** | 0 |
| ignored | 0 | **0** | 0 |

The prior holds **exactly**, as it must: this lane adds **zero Rust code** —
`ci.yml` is a new file under `.github/`, compiled by nothing.

**⚠️ WINDOW THE HITS — and this lane is where it bites.** The shipped canon step
writes all six legs to **one** stream (the CI job log) with `::group::` markers,
not to per-leg log files. So a whole-log `^test result:` grep reads **74**, not
72, and would look like a +2 drift that does not exist. The axes are the `tests`
leg's numbers alone; leg 4 `wasm-tests` contributes the other 2 result-lines and
80 passes. **72 + 2 = 74, fully attributed.** Attribution is by parsing the
group markers, not by matching a spelling.

Failure/panic greps, unpiped: `^failures:` 0 hits, `panicked at` 0 hits.

## Stated deviations from reference form

**No per-leg boundary `df`.** Deliberate, not an omission. The reference form's
disk-boundary rail exists because my box is shared and a battery can fill it;
a GitHub-hosted runner is ephemeral, unshared, and destroyed after the job, so
the rail guards nothing there. The rail still applies to the local battery and
`boundary-start.df` records 90.86 GiB against `THRESHOLD.txt` 40.

**No `2>/dev/null`, no `|| true`, no `-q` anywhere in the shipped YAML.**
Producer-side silence redirection is banned, and provisioning is deliberately
fail-loud and version-echoed: a green run says *which* toolchain produced it,
and a leg that cannot run stops the job rather than quietly passing.

## Closing verification — the controlled bytes ARE the committed bytes

Everything above tests scripts extracted from a *working-tree* file. That
leaves one gap worth closing explicitly: the file that got committed could
differ from the file that was controlled.

So the extractor was re-run against the **committed git object**
(`git show <commit>:.github/workflows/ci.yml`), and the results match the pins
exactly:

| Script | From the committed object | Pinned in evidence |
|---|---|---|
| `canon.sh` | `14ec947e4bcf251c…` | `14ec947e4bcf251c…` |
| `verdict.sh` | `7d956a5f9dfe61b1…` | `7d956a5f9dfe61b1…` |

The workflow also parses as YAML, and its structure is what it claims: one job
`gates` on `ubuntu-latest`, seven steps, with **both** terminal steps —
`Verdict` and `Upload per-leg rc artifacts` — carrying `if: always()`, so a
failing canon step cannot skip the verdict or discard the artifacts.

**One parsing note, recorded because it will confuse somebody eventually:**
under YAML 1.1 an unquoted `on:` key loads as the *boolean* `true`, not the
string `"on"`. Every GitHub workflow in existence has this property and
GitHub's own parser is authoritative, so it is not a defect here — but a
home-grown linter reading `y["on"]` will find nothing and may report the
workflow as having no triggers.

## Not shipped, and why — the file-size gate

**The tokei file-size job is deliberately NOT in `ci.yml`.** It needs a ruling
before it can be, and the reason is a measurement that refuted my own
instrument. See the options package delivered alongside this receipt.
