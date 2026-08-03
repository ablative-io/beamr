# Lane #69 DIST-DOCS — evidence

Base: `0d6676a3501d286b61ad14c3816491d14124141a`
Branch: `artemis/dist-docs-repoint`
Authority: sha-frozen brief `4d7811bc…`, dispatched on Cally Ray's named GO
(message `d9083cdd`, 2026-08-03T02:17:34Z). Chain: RF-003-D3 finding F5 →
Cally's F5 ruling → registration ruled complete (`c68e0166`).

## What this lane is

A **docs-only** amendment. The whole tracked diff is one rustdoc comment block
in `crates/beamr/src/distribution/etf.rs` (the derivation block at lines 25–92,
moved byte-intact from `connection.rs` by RF-003-D3), plus this evidence
directory. No executable line changes. No new tests. No new `#[allow]`.

Three deliverables, wordings ratified at registration:

- **E1** — re-true "No peer count is written anywhere in this file" to name BOTH
  homes. The claim was written about `connection.rs` and its load-bearing
  reference to `connection.rs`'s accept path was lost in the move.
- **E2** — de-self-reference "survived only in `distribution/etf.rs`" (the
  reader is already in `etf.rs`) → "survived only in THIS module".
- **E3** — four rustdoc intra-doc links whose targets are private to
  `connection.rs`, and therefore unresolvable from here, converted to plain
  code font with an explicit `in `connection.rs`` locator on first occurrence.

## NO RED LEG — RULED OUT, NOT SKIPPED

There is no red-at-base leg in this lane, and its absence is **ruled out loud**,
never omitted. Ground (Cally, `c68e0166`): this edit changes no behaviour, so
there is no behaviour to demonstrate failing. A red here would be theatre — it
could only be manufactured by breaking something unrelated.

This lane's evidence class is **E4's per-sentence bytes-verification** instead:
every re-trued sentence carries a grep proving it true at the bytes, each with
its own `.rc` file.

## E1's grep is the sentence's LICENSE TO EXIST

The measurement ran **BEFORE** the sentence was written. The rule stood: if a
hardcoded peer-count literal existed anywhere the sentence denies one, the
SENTENCE would be wrong, not the tree — stop and report.

The bare spelling `64` is not the measurement (it appears in `u64`, `i64`,
`f64`, `AtomicU64`, in `64 * 1024 * 1024` byte quantities, in lane number
"#64", in "64 bits", and in derivation prose stating the DERIVED result — none
of those is a peer-count write). The hits were windowed: standalone-`64` tokens
only, on code lines only, in the **non-test** region only.

| file | non-test region | code-line standalone-`64` | rc |
|---|---|---|---|
| `etf.rs` | lines 1–1061 | 1 hit: `MAX_DIST_FRAME_BYTES = 64 * 1024 * 1024` (a byte quantity) | 0 |
| `connection.rs` | lines 1–1889 | **0 hits** — accept path writes no peer count | 1 (no match) |

Verdict: **no peer-count literal is enforced in code in either home.** The
ratified sentence is true at the bytes. License granted.

Artifacts: `e1-grep-64-raw.*` (unwindowed, for audit),
`e1-grep-peercount-literal.*` (standalone tokens, both files),
`e1-grep-cfgtest-boundaries.*` (the region split),
`e1-stage-*` (intermediate region/code-only extracts),
`e1-final-etf-peercount-literal.*`, `e1-final-connection-peercount-literal.*`
(the two final unpiped measurements, each with its own honest rc).

## Reconciliation-carry — MISMATCH, UNRECONCILED

No baseline run and no baseline commit, per ruling. The test-suite prior is
**2064 passed / 0 failed / 72 ignored**, certified at Cally's own instrument at
this exact code tree (`0d6676a`).

**named-new = ∅ — docs-only, zero new tests.**

Observed at leg 5:

```
EXPECTED-PRIOR   2064 passed / 0 failed / 72 ignored
OBSERVED         2064 passed / 0 failed /  0 ignored
```

`passed` and `failed` match the prior **exactly**. The `ignored` figure does
not. Per the ruling, any mismatch in count or verdict stops the lane and is
reported raw — **it is NOT quietly re-baselined here, and this lane must not be
treated as reconciled until the coordinator rules.**

Raw supporting measurements (all from `leg-5.log`, not from memory):

- result lines carrying a non-zero `ignored`: **none**
- individual tests marked `... ignored`: **0**
- `test result:` lines: **72**
- `running ` headers: **72**
- the string `72` appears nowhere else in the log

Note offered as observation, not as a re-baseline: the prior's third figure
(72) is numerically equal to the number of test binaries in this workspace, not
to any ignored count present in the run. Whether the prior conflated
binary-count with ignored-count is the coordinator's call, not this worker's.
The verdict itself is green (rc 0, zero failures).

## Battery

Six legs, commands verbatim from `gates.json` at `0d6676a`, run from the
worktree root. Each leg's log captured by REDIRECT to its own file (never
`tee` — `tee`'s rc masks the producer's), each rc captured on its own line into
its own artifact, never through a pipeline. No producer-side silence
(no `2>/dev/null`, no `|| true`, no `-q`).

Leg 6 is never piped: no `pipefail`, and its `--json` line numbers are
zero-indexed against the human formatter's one-indexed — no phantom
off-by-one is to be chased. Exit 0 = no findings; it emitted `[]`.

BOUNDARY-DF ran before every leg (`leg-N.df`); see `THRESHOLD.txt`. Available
never approached the 39,321,600 KiB floor — it sat near 105,3xx,xxx KiB
throughout.

## Bar

- new `#[allow]`: **zero, none introduced** (0 added lines match `#[allow`)
- `unwrap`/`expect`/`panic` outside `cfg(test)`: none added — no code added
- `_ =>` arms: none added — no code added
- every `git diff` invoked with `--no-ext-diff` (global `diff.external = difft`
  would otherwise break raw diffs)
- `.claude/skills/` never touched, staged, or deleted
- the coordinator's main checkout never touched
- **PUSHED NOTHING. TAGGED NOTHING.** Commits are local to this branch.
