# BRIEF #64 — registered baseline at clean base (RULING 4: register-then-run)

Registered BEFORE any edit to tracked source. This commit is the registration;
commit ordering in history is the instrument.

## Base

- Worktree: `/Users/tom/Developer/ablative/stack/beamr-64`
- Branch: `artemis/dist-byte-bounds`
- `git rev-parse HEAD` at run time: `92ca6c49724a68c47cca47141bd81978271b3178`
- Working tree at run time: porcelain clean (no tracked modifications; the
  untracked `.claude/skills/` directory is never staged and never deleted).

## Command

```
cargo test --workspace
```

Run form: stdout+stderr redirected to `baseline-test.log` and the return code
read by `echo $?` on its own line — NOT through a pipeline. A `tee` pipeline
would have reported `tee`'s status, not the producer's; the redirect keeps the
full log while leaving `$?` the producer's own. No producer-side silence
redirection was used (no `2>/dev/null`, no `|| true`, no `-q`).

- Start (UTC): `2026-08-03T00:22:52Z`
- End (UTC): `2026-08-03T00:25:26Z`
- Return code: **0** (direct)
- Full log: `gate-logs/64-baseline/baseline-test.log`

## Verdict and counts

**GREEN.**

| Quantity | Value |
| --- | --- |
| Tests passed | **2052** |
| Tests failed | **0** |
| Tests ignored | 0 |
| Harness result blocks (`test result:` lines) | **72** |
| Suite launches (`Running` + `Doc-tests` lines) | 71 |
| `^error` lines in log | 0 |

### Reconciliation against EXPECTED-PRIOR (2052 passed / 0 failed / 72 binaries)

Passed and failed match exactly. The "72 binaries" figure is the count of
harness **result blocks**, which is 72 here; the count of suite *launches* is
71. The two differ by one because `Doc-tests beamr` emits TWO harness blocks —
one for ordinary doctests (1 test) and one for `compile fail` doctests (2
tests) — under a single `Doc-tests` line (log lines 2545-2559). That is a
property of the rustdoc harness, not a change in the suite.

No mismatch. No STOP condition triggered.

## Disk boundary at baseline

- Instrument: `df -k /System/Volumes/Data`
- Threshold: 47185920 KiB available (45 GiB)
- Read at `2026-08-03T00:22:52Z`: **112133736 KiB available** — above threshold.
- Raw: `gate-logs/64-baseline/boundary-baseline.df`
- `du -sk` on the worktree at baseline start: **21896 KiB**
  (`gate-logs/64-baseline/du-start.txt`)
