# 0.17.0 release — battery receipt

Evidence for the `v0.17.0` cut. Three battery directories are kept here and
**one of them is void**. It is kept, named, and placed first on purpose: the
defect it carries is the most useful thing in this directory.

## ⚠️ READ THIS BEFORE ANY RESULT BELOW

**A completion marker written unconditionally is not a marker, it is a
constant.** `battery-1-VOID/COMPLETE.marker` reads, in full:

```
DONE
```

`runner-VOID.sh` writes that string as its last line, whatever happened above
it. It is not derived from the work, so it attests exactly one fact — the
script reached its final line. And the directory contains **no `leg-*.rc`
files at all**, so nothing in it distinguishes six passing legs from six legs
that never ran. That run was discarded and the release was held, twice, rather
than published on it.

`runner-repaired.sh` produced the other two. Its marker is derived:

```sh
scored=0; for f in "$EV"/leg-*.rc; do [ -f "$f" ] && scored=$((scored+1)); done
if [ "$scored" -eq "$N" ]; then
  printf 'COMPLETE legs_declared=%s legs_scored=%s commit=%s\n' "$N" "$scored" "$COMMIT"
else
  ... > VOID.marker; exit 89
fi
```

`$N` is read out of `gates.json` — **a different source from the loop that
fills the rc files**, so the count and the thing counted do not share a fault
domain. A runner that dies mid-loop leaves a truncated set of zeros, and a
truncated set of zeros reads exactly like a clean run. **Count first, then
read.**

Four more repairs are in `runner-repaired.sh`, documented in-file:

- **The interpreter is proved by launching it, not by looking it up.**
  `command -v` attests RESOLVABILITY, never EXECUTABILITY. The runner runs
  `"$PY" --version` and keeps the output — `interpreter.out` reads
  `Python 3.9.6 / path=/usr/bin/python3`.
- **The denominator is parsed, not pattern-matched**, and a non-numeric answer
  exits 96 rather than defaulting.
- **A non-empty `denominator.err` exits 98.** A tool that writes to stderr and
  still exits 0 has told you something; discarding it is the same class of
  mistake as the constant marker.
- **No producer-side silence anywhere** — no `2>/dev/null`, no `|| true`, no
  `-q`. Every leg's log is a redirect, never a tee.

Three of those abort paths were driven and observed, not asserted.

## The two scored batteries

| directory | commit | legs | marker |
|---|---|---|---|
| `battery-2-pre-tag/` | `8dcf747` | 6/6 | `COMPLETE legs_declared=6 legs_scored=6 commit=8dcf747…` |
| `battery-3-at-tag/`  | **`377b6de`** | **6/6** | `COMPLETE legs_declared=6 legs_scored=6 commit=377b6de…` |

`377b6de` is the commit `v0.17.0` peels to. **The marker names the tagged
sha**, so the green and the tag are bound by content rather than by a claim
made about them afterwards.

### `battery-3-at-tag` — per-leg, quoted from the artifacts

| leg | name | rc |
|---|---|---|
| 1 | fmt | 0 |
| 2 | clippy | 0 |
| 3 | wasm32-check | 0 |
| 4 | wasm-tests | 0 |
| 5 | tests | 0 |
| 6 | blocking-call-in-native-bif | 0 |

Extras, same run: `extra-cooperative-json.rc` 0, `walls-binary-match.rc` 0,
`walls-gc-rooting.rc` 0. Leg 6's log is `[]`. All three stderr captures are
size 0 — and unlike the constant marker, that zero is load-bearing only
because nothing in this runner redirects stderr away.

- `pin.txt` — commit `377b6de…`, tree `b631a54…`
- `tree-state.txt` — `?? .claude/skills/` only (untracked, not part of the cut)
- `denominator.txt` — `legs_declared=6`, `denominator_rc=0`
- `tally.txt` — `legs_declared=6 legs_scored=6`
- Disk: boundary `df` taken before **every** leg against
  `THRESHOLD.txt` = 25,000,000 KiB; low-water 38,119,816 KiB free.
  `du-final.txt` = 6,869,048 KiB = 6.55 GiB.

**Axes 72 / 2067 / 0 / 0**, stated as a prediction before the run and
satisfied. The delta from the greened `8dcf747` is CHANGELOG + AUDIT + one
runbook paragraph and **zero Rust**, so identity was required here and a
*difference* would have been the finding. Walls non-vacuous by count:
17 passed / 1767 filtered, and 3 passed / 1781 filtered.

## `carriage-markers.out` — the forward-ports, verified by content

`main` is not a descendant of `v0.16.3`. It carries the 0.16.2/0.16.3
memory-safety fixes as **forward-ports**, which are the same change under a
different patch — so **every identity-based carriage test reports them
ABSENT**: commit sha, ancestry, `git cherry`, `git patch-id`. `git cherry`
marks every fix commit `+`, which reads plainly as *the fixes did not make
it*. Only content answers, so `verify-carriage.sh` counts markers.

All eight markers carry (`67f89c4` → `377b6de`), and all six files are
byte-identical between `v0.16.3` and `v0.17.0` by blob id.

⚠️ **`string_bifs.rs` is 2 → 7, not 0 → n.** It already contained the idiom
before the fix, so **a presence test passes at the pre-fix state** and reports
carriage that is not there. Compare the columns. That single row is why the
table published in `CHANGELOG.md` has a left-hand column at all — an
instrument that lies in the reassuring direction is worse than none.

Three controls, in-artifact:

- **C1** absent pattern → `0` — and it *returns* 0 rather than aborting. An
  earlier draft of this script asserted the producer by `|| fail` on the whole
  pipeline, but **`grep -c` exits 1 on a zero count**, so a true zero was
  being reported as a failed producer. A declined measurement and a negative
  one are not the same answer.
- **C2** certainly-present pattern → `31`. The instrument can say a number.
- **C3** bad path → aborts rc 9 with `PRODUCER-FAILED`, **message captured in
  the artifact, not sent to `/dev/null`**. Silencing it would have proved
  nothing and left `carriage-markers.err` misleadingly empty.

C3 exists because of a live failure during this work: `git show` was fed a
mangled rev, wrote nothing, and `grep -c` counted its empty stdin as `0`. The
harness then printed `pre=0 post=0` for all eight rows, formatted exactly like
a measurement. **A failed producer feeding a counter yields a confident zero.**

The mangling itself: **in zsh, `$VAR:` begins a modifier**, so `$PRE:crates/…`
became `67f89c4rates/…`. Every rev in the shipped script is braced. This is
the second time this hazard fired in this release — the first ate a push
refspec — because it was banked as a *refspec* hazard when it is a *colon*
hazard.

## `close-declared-ahead.out` — the §6 close, after the publish

For every publishable member, ask the registry whether the version **declared
in the tree** already exists. Not-published means *declared ahead*. The
close must return EMPTY.

```
gleam-types  0.4.3   200      beamr       0.17.0  200
beamr-cli    0.5.0   200      beamr-wasm  0.8.0   200
controls: beamr/0.16.3 -> 200 · beamr/0.99.0 -> 404 · beamr-nonexistent-xyzzy/1.0.0 -> 404
```

**Declared-ahead set: EMPTY. No named residue.** `gleam-types` was declared
*out* of the wave — 0.4.3 is already its published version, zero commits since
it was set — and stating the exclusion is part of the declaration, because a
set that silently omits a member is indistinguishable from one that lost it.

**The controls are the load-bearing part, not the four `200`s.** A run in
which every probe answers `200` closes perfectly whether or not anything
published. Two negatives redden here, one of them on a crate name that has
never existed — the arm that catches an interposing proxy answering `200` to
everything. The probe hits the **metadata** endpoint (`api/v1/crates/<n>/<v>`)
and sends a User-Agent: the download endpoint `302`s blindly for absent
versions, and a request without a UA gets `403` for every crate, which parses
as "nothing is published" and closes perfectly while being entirely false.

## What this release does NOT certify

- **`jit` cannot be disabled in any build that retains `threads`.** The
  manifest declares `jit = ["std", "threads", …]`, but parts of the scheduler
  are gated on `threads` while referencing `crate::jit`, so a build with
  `threads` and without `jit` does not compile. This predates 0.17.0 and is
  present at `v0.16.3`. It is disclosed in `CHANGELOG.md` as a **defect under
  repair, not an intended property**, and the fix is a design ruling that is
  not this lane's to make.
- **The battery does not build the shipped feature set.** These legs test; a
  test build unifies dev-dependency features, so the feature set under test is
  a **superset** of the one that ships. A
  `cargo check --no-default-features --features <shipped set>` leg would close
  this and has no test population to go stale. Not added here.
- **The RF-006 JIT rooting class is still open** — one named site and two
  unenumerated classes, reachable under `jit`, which is on by default.

## Files

```
battery-1-VOID/        the discarded run — constant marker, no per-leg rc
battery-2-pre-tag/     8dcf747, 6/6
battery-3-at-tag/      377b6de, 6/6, the commit v0.17.0 peels to
runner-VOID.sh         the runner that produced the void run
runner-repaired.sh     the runner that produced both scored runs
verify-carriage.sh     forward-port marker check, 3 controls
carriage-markers.out   its output          carriage-markers.err   (0 bytes)
close-declared-ahead.sh  the §6 close, with both control arms
close-declared-ahead.out its output        close-declared-ahead.err (0 bytes)
tag-message-v0.17.0.txt  the annotated tag message as minted
```
