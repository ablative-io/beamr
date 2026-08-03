# RF-003-D3 — BASELINE REGISTRATION (register-then-run, RULING 4-class)

Lane RF-003-D3 (etf.rs framing-helper cap + Display reword + constant-home MOVE).
Dispatched by Artemis Peach under Cally Ray's named GO `c9c71b21`
(2026-08-03T01:12:17Z) quoting brief sha
`a3141d249ea275f4898d0a83b1df1e4c0a29a2bdfe4352d8a7119f75d2f529d1`.

This file is committed BEFORE any lane edit. **Commit ordering IS the
registration** — a baseline that only exists after the fact is not a baseline.

## Base

| item | value |
| --- | --- |
| worktree | `/Users/tom/Developer/ablative/stack/beamr-rf003d3` |
| branch | `artemis/rf003-d3-etf-cap` |
| base sha | `1624f43d1c0e1b604968a59550be5afc7eaf0afc` |
| porcelain at base | CLEAN (`git status --porcelain` produced no output) |
| brief sha, self-verified | `a3141d249ea275f4898d0a83b1df1e4c0a29a2bdfe4352d8a7119f75d2f529d1` — EXACT match, **no frontmatter-`modified:` delta, no delta of any kind** |

## Command and window

| item | value |
| --- | --- |
| command | `cargo test --workspace` |
| start (UTC) | `2026-08-03T01:16:14Z` (artifact `baseline-start.utc`) |
| end (UTC) | `2026-08-03T01:18:56Z` (artifact `baseline-end.utc`) |
| rc | `0` (artifact `baseline.rc`) |
| log | `baseline-test.log` (captured by REDIRECT — no tee, no pipeline) |

Reference form: the log is captured by `> log 2>&1`; the rc is `echo $?` on its
own line into its OWN artifact file (`baseline.rc`). No `2>/dev/null`, no
`|| true`, no `-q` — no producer-side silence redirection anywhere.

## Counts AS PARSED

Parse method (run over the committed `baseline-test.log`, not re-derived from
memory):

```
awk '/^test result:/ {n++; p+=$4; f+=$6; i+=$8} END {print "blocks="n" passed="p" failed="f" ignored="i}' baseline-test.log
```

`blocks=72 passed=2060 failed=0 ignored=0`

| quantity | EXPECTED-PRIOR | AS PARSED | verdict |
| --- | --- | --- | --- |
| harness result blocks | 72 | 72 | MATCH |
| passed | 2060 | 2060 | MATCH |
| failed | 0 | 0 | MATCH |
| ignored | 0 | 0 | MATCH |
| rc | 0 | 0 | MATCH |

EXPECTED-PRIOR provenance: Cally's own counter from lane #64, granted as
registration item (2) of the brief. **No mismatch — no STOP condition fired, and
no quiet new baseline was minted.**

## No-prior caveat

There is no prior RF-003-D3 baseline on this branch: `artemis/rf003-d3-etf-cap`
is created at `1624f43` for this lane and this is its first run of any kind. The
2060/0/72 figure is therefore corroborated, not inherited — it is Cally's #64
counter re-measured here at a different base by a different runner, and it
agrees. That agreement is the registration's whole value; it is not a claim that
the two runs share an instrument.

## Boundary — disk

Threshold instrument and value: `../rf003d3-battery/THRESHOLD.txt`
(**47710208 KiB**, noun = KiB Available on `/System/Volumes/Data`, instrument =
`df -k /System/Volumes/Data`; 45.5 GiB, joiner-inclusive, GUARD-1 live).

Artifact `boundary-baseline.df`, which echoes the value it read:

```
read available KiB: 106343652
verdict: above threshold - proceed
```

106343652 KiB available vs 47710208 KiB threshold — clear by 58633444 KiB.

## Disk consumed by the baseline

| point | du -sk (KiB) | artifact |
| --- | --- | --- |
| worktree before | 22680 | `du-start.txt` |
| worktree after | 4161096 | `du-end.txt` |

Baseline build+test cost: 4138416 KiB ≈ 3.95 GiB. Registered price for the whole
lane is ~9.5 GiB measured-adjacent with a ~10.5 GiB reportable ceiling; this
reading is consistent with that class (the six-leg battery adds the wasm32 and
clippy closures on top of this one).
