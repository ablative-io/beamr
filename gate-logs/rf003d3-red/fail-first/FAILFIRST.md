# RF-003-D3 — FAIL-FIRST REVERT LEG, at the final head

Window: `failfirst-start.utc` → `failfirst-end.utc`. Every step's rc is in its
own artifact file; every log is a redirect.

## Why the revert is SCOPED (declared, not slipped in)

The dispatch offers `git revert --no-commit` **or** a stash-style revert. The
whole-commit revert is not usable here, and step 1 exists to prove that rather
than assert it: the implementation commit `8d31abc` carries the WALLS as well as
the FIX, so reverting it deletes the very tests that are supposed to be observed
going red. **A test that vanished is not a test that failed.**

`step1-git-revert.log` / `.rc` (rc 0) then `step1-revert-stat.log`:

```
 crates/beamr/src/distribution/etf.rs               | 142 +-----------------
 ...
 18 files changed, 77 insertions(+), 495 deletions(-)
```

142 lines off `etf.rs` — the cap arm, the Display reword, the moved constant AND
all four tests. `step1-revert-etf.diff` holds the full text. The revert was then
abandoned (`git revert --quit`, rc 0) and the tree restored
(`git reset --hard 8d31abc`, rc 0).

So the evidence run reverts **the fix** and keeps **the walls**.

## Step 2 — the scoped revert, exactly

`step2-scoped-revert.diff`, whole content: nine lines removed from
`read_dist_message`, the D-a cap arm and nothing else.

```
-    if length > MAX_DIST_FRAME_BYTES {
-        return Err(Error::LengthTooLarge);
-    }
```

## Step 3 — both legs RED again

| leg | test | rc | artifact |
| --- | --- | --- | --- |
| R1 Leg A | `deframe_refuses_length_above_max_dist_frame_bytes` | `101` | `step3-lega.rc` |
| R3 boundary wall | `deframe_boundary_accepts_exactly_max_and_refuses_one_over` | `101` | `step3-boundary.rc` |

Leg A, quoted from `step3-lega.log`:

```
panicked at crates/beamr/src/distribution/etf.rs:1393:9:
assertion `left == right` failed
  left: Err(TruncatedBody { expected: 67108865, actual: 0 })
 right: Err(LengthTooLarge)
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 1783 filtered out
```

Boundary wall, quoted from `step3-boundary.log`:

```
panicked at crates/beamr/src/distribution/etf.rs:1421:9:
assertion `left == right` failed
  left: Err(TruncatedBody { expected: 67108865, actual: 0 })
 right: Err(LengthTooLarge)
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 1783 filtered out
```

Read the boundary failure closely: it panics at `:1421`, which is the
**MAX+1 half**. The exactly-MAX half above it PASSED without the cap — as it
must, since an uncapped helper accepts everything. That is the boundary being
genuinely two-directional: the cap supplies the refusal, and the acceptance is
what the cap must not break.

## BOUNDEDNESS of the reverted run, from the artifacts

Both failures print `expected: 67108865` — 64 MiB + 1, the exact size the
reverted code reserved. Peak is ~64 MiB per leg, briefly held. **Never
gigabytes**, no `handle_alloc_error`, no runner abort.

**The `0xFFFFFFFF` extreme leg was DELIBERATELY NOT RUN.** Each leg above is its
own `cargo test … -- --exact <one name>` invocation precisely so that
`deframe_refuses_extreme_header_without_allocating` could be excluded. Against
reverted bytes that test reserves and commits ~4 GiB — the bounded-red
constraint forbids it absolutely, and it is a GREEN-ONLY leg by construction.
Running the whole `distribution::etf::tests` module under the revert would have
fired it; the two single-test invocations are the mechanism that prevents that,
not an accident of filtering.

## Step 4 — restored exactly, NO residue

| check | artifact | value |
| --- | --- | --- |
| `git checkout -- etf.rs` | `step4-restore.rc` | `0` |
| `git status --porcelain` | `step4-porcelain.log` | `?? gate-logs/rf003d3-red/fail-first/` only — the untracked evidence of this very leg, nothing else |
| `git rev-parse HEAD` | `step4-head.log` | `8d31abcf539c93604136d93962645bce04070c58` — unmoved |

No revert residue: no `.git/sequencer`, no staged reversal, no modified source.
The only working-tree delta is this directory.
