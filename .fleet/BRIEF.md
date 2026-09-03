# BRIEF — beamr: docs-truth follow-up — self-test fixtures off the checkout's parent, and the red-first transcripts recorded whole

Artemis Peach, beamr owner. Ordered by Waffles 2026-09-03T06:02Z after the docs-truth landing
(beamr main `7cfa1771`): "The two owed items are one small brief at your hand … Red-first on
7cfa1771, fire on 205 within the caps, land by your hand on green." Both items were flagged by the
docs-truth judge as "worth the next seat's attention (neither a brief failure)". Grill: Waffles.

## Ground measured at `7cfa17715682dfe72eaacd6c17f9edd87aa1620b` (main, crate `0.20.0`)

Every line read at that sha in a detached checkout; every command run there and its output kept
(`dispatch-rigs/beamr-docs-truth-followup/GROUND-*.out` in docs).

| # | Claim | Where | What the tree does today |
|---|---|---|---|
| G1 | "All fixtures used by --self-test are minted under the run-local TMPDIR" | `scripts/gate-docs-truth.sh:3-4` (header comment) | **False.** `self_test()` mints at `tempfile.mkdtemp(prefix="docs-truth.", dir=str(Path.cwd().parent / "tmp"))` — `:343`. `TMPDIR` is never read: with `TMPDIR=/nonexistent/x` the self-test still exits 0 with `9/9` (measured). The docs-truth gate's own header is a docs-truth defect |
| G2 | The self-test measures from any checkout | `:343` | **False.** From a cwd whose parent has no `tmp/`, `--self-test` raises `FileNotFoundError: [Errno 2] … /noparent/tmp/docs-truth.pl655h30` out of `tempfile.mkdtemp` and exits **1** with a Python traceback — the rc of a *measured red*, not a refusal, and no arm is named (measured) |
| G3 | The fixture parent is cleaned up | `:358-359` (`finally: shutil.rmtree(temp_parent)`) | True for the mkdtemp dir; the `../tmp` it depends on must pre-exist and is never created or removed by the gate |
| G4 | "Record the whole transcript, not the exit code alone" | the flight's leg, step 9.2 (`.fleet/legs/docs-truth.md:528-546` at result branch `575bd7b0`) | The flight's report records T10 as **two lines** (`.fleet/reports/docs-truth.md:195-200`): the arm-(a) line and the count line. The judge re-ran the literal T10 and saw every failing arm named ("all 19 reds printed"); that evidence exists only in the judge's ruling, not in the record |
| G5 | T10's fixture is "the pre-R1 README … plus `<!-- release: 0.17.0 -->`" | leg step 9.2 | The judge read the recorded fixture as "the final README with a stale release line rather than the pre-R1 wording" (VERDICT-DT.md, note 2). The record does not state how the fixture was minted |
| G6 | The T9/T10 transcripts live on main | — | Nothing under `docs/design/beamr/briefs/evidence/docs-truth/` exists on main (0 paths). The only copies are in `.fleet/` on the private mirror and in the judge's ruling on docs main |
| G7 | Gate and self-test at this sha | `scripts/gate-docs-truth.sh` | gate rc **0**; `--self-test` **9/9** rc 0 when run from a cwd whose parent has `tmp/` (both measured at the owner's landing, LANDED-DT.receipt) |

## The one-sentence contract

The self-test mints its fixtures under the run's own `TMPDIR` or a scratch the gate itself creates
inside the repository and removes, refusing by name when it cannot, so it measures from any checkout
and never raises; and the flight's red-first evidence for docs-truth — T9, T10 and the self-test —
is recorded whole, with each fixture's provenance, both in the flight's report and on main.

## Requirements

**R1 — fixture location.** In `self_test()`, the scratch parent is: `os.environ["TMPDIR"]` when set
and non-empty, else `ROOT / ".docs-truth-selftest"`. **A user-supplied `TMPDIR` is never created by the
gate** — it must already exist and be writable, else arm (f) names it (R2); `mkdir(parents=True,
exist_ok=True)` is for `ROOT / ".docs-truth-selftest"` only. The gate mints with
`tempfile.mkdtemp(prefix="docs-truth.", dir=<scratch>)` as today, and in `finally` removes the minted
dir AND, when the scratch is the repo-local one, the scratch dir itself (only if it is then empty —
never `rmtree`, never `mkdir`, a user-supplied `TMPDIR`). *(Amendment, Waffles 2026-09-03T06:08Z: the
first text had the gate create either directory; that would create the operator's tree on any box where
it can, and control (i) reached arm (f) only because `/nonexistent/x` cannot be created there.)* `Path.cwd()` does not appear
in the script afterwards (`grep -c 'Path.cwd()' scripts/gate-docs-truth.sh`: before **1**, after **0**).
The header comment `:3-4` is made true by the code, not rewritten to match the old code.

**R2 — refuse, never raise.** Any `OSError` from creating the scratch or minting under it is reported
through the same `emit` the other arms use, as a new arm **(f)**:
`REFUSE arm (f): self-test scratch cannot be created at <path>: <strerror>` and `self_test()` returns
**3**. No traceback reaches stdout or stderr (`grep -c Traceback` on the transcript = 0). Add
`"f"` wherever the script enumerates its arms, and add the arm to `gates.json`'s note only if that
file already names the arms individually (read it first; if it lists `cannot_measure_rcs: [3]` and
nothing per-arm, leave it untouched — it is outside R6's wall).

**R3 — red-first for R1/R2, recorded whole.** Before the edit, at `7cfa1771`, from a cwd whose parent
holds no `tmp/`: `--self-test` → traceback, rc 1 (G2). After the edit, from the same cwd: `9/9`, rc 0.
Controls, each transcript recorded whole with its rc on its own line:
- (i) `TMPDIR=/nonexistent/x bash scripts/gate-docs-truth.sh --self-test` → before: rc 0, `9/9`
  (the variable is ignored — G1); after: rc **3**, one `REFUSE arm (f)` line naming `/nonexistent/x`,
  no traceback;
- (ii) `TMPDIR` unset → after: `9/9`, then `git status --porcelain` empty and
  `test ! -e .docs-truth-selftest` true;
- (iii) `TMPDIR=<a writable empty dir you mint>` → after: `9/9`, and that dir is empty afterwards
  (`ls -A` prints nothing) — the gate removed its minted dir and left the user's `TMPDIR` in place.
- (iv) **`TMPDIR=<a creatable-but-absent path under a writable parent>`** (e.g. mint an empty dir `P`,
  set `TMPDIR=P/absent`) → after: rc **3**, one `REFUSE arm (f)` line naming `P/absent`, and
  `test ! -e P/absent` true afterwards — the rule refused rather than the filesystem; the gate created
  nothing. Before (at `7cfa1771`): `TMPDIR` is ignored, so rc 0 `9/9` and `P/absent` still absent —
  record that too, it is the same shape as (i)'s before.

**R4 — the red-first transcripts, whole and on main.** Create
`docs/design/beamr/briefs/evidence/docs-truth/red-first.md` carrying, verbatim and complete (every
stdout and stderr line, the rc on its own line, the exact command above each), at the final head:
- **T9** — the shipped gate against a tree whose `README.md` and `CHANGELOG.md` are
  `git show 2d2a9443:<file>` (R1–R5 present, markers absent) → rc **3**, `REFUSE arm (e) … zero markers`;
- **T10** — the shipped gate against a minted copy of the **pre-R1** README: `git show 3a23408d:README.md`
  (the flight's base — the D1 wording) with its release line set to `<!-- release: 0.17.0 -->` and
  **one** valid class marker inserted inside the advisory region so (e) passes; the minting commands
  (the `git show`, the `sed`/`python` edit) recorded above the transcript → rc **1**; the transcript
  holds **every** `RED arm` line the gate prints and a final `docs-truth: N measured red(s)` whose N
  equals the count of `RED arm` lines above it; arm **(a)** is among them and quotes `0.17.0`/`0.20.0`;
- **the self-test** after R1 → `9/9`, rc 0, from the parentless cwd of R3;
- the four R3 controls.
The flight's own report (`.fleet/reports/<leg>.md`) carries the same transcripts whole — the leg's
evidence rule is the sentence in G4. A two-line T10 is a red for this brief.

**R5 — tests are real.** `.fleet/tests/test_docs_truth.py` (the flight's file, at `575bd7b0` on the
private mirror — copy it into this flight's `.fleet/tests/` and extend it) gains at least:
- `test_selftest_refuses_by_name_when_scratch_unwritable`: `TMPDIR=/nonexistent/x` → rc 3, output
  contains `REFUSE arm (f)` and `/nonexistent/x`, and does not contain `Traceback`;
- `test_selftest_leaves_no_scratch`: `TMPDIR` unset → rc 0, `9/9`, `.docs-truth-selftest` absent after;
- `test_selftest_honours_tmpdir`: minted `TMPDIR` → rc 0, dir empty after.
- `test_selftest_never_creates_tmpdir`: `TMPDIR=P/absent` under a minted writable `P` → rc 3, output
  names arm (f) and `P/absent`, and `P/absent` does not exist after.
Every test invokes the shipped `scripts/gate-docs-truth.sh` as a subprocess and asserts on the real rc
and real text. The 5b new-test control counts these (python is one of its six).

**R6 — the wall.** Files this flight may touch, the whole list: `scripts/gate-docs-truth.sh`,
`docs/design/beamr/briefs/evidence/docs-truth/` (new), `.fleet/`. `README.md`, `CHANGELOG.md`,
`gates.json`, `crates/**`, `Cargo.*` are not edited (predicate: `git diff --name-only 7cfa1771...HEAD`
contains none of them). The gate is green at the final head (rc 0) and its self-test is `9/9`.

**R7 — honesty.** Counts carry instrument and population; no `2>/dev/null` anywhere in the gate or the
transcripts; the report's ABSENT section names anything above not delivered.

## Round 2 note (2026-09-03T06:5xZ) — read before anything else

Round 1 (wf `7acaac23`) was judged `judge_pass` and **refused at the owner's landing** on the amended R1: its
`self_test()` runs `scratch.mkdir(parents=True, exist_ok=True)` on the user-supplied `TMPDIR` too (judged script
`:343-350`), so control (iv) created `P/absent` and passed 9/9. Everything else in that tree held at the owner's hands
(gate rc 0; self-test 9/9 from a parentless cwd; controls (i)-(iii); `red-first.md` 416 lines, T10 whole).

**Start from that tree, do not redo it.** The judged tree is `2094e0316cb6879ad28f95638453cdae44c3641f` on the
repo you are fetched from (`refs/heads/fleet/beamr-docs-truth-followup-result`); take exactly its two non-`.fleet` files
(`scripts/gate-docs-truth.sh`, `docs/design/beamr/briefs/evidence/docs-truth/red-first.md`) onto `7cfa1771`. Then:
(1) `mkdir` only when the scratch is `ROOT / ".docs-truth-selftest"`; a supplied `TMPDIR` that does not exist or is not
writable → arm (f) names it, nothing created; (2) add control (iv) — before (the round-1 script: `P/absent` created,
rc 0) and after (rc 3, `REFUSE arm (f)` naming `P/absent`, `test ! -e P/absent`) — to `red-first.md`, whole, with the
minting commands; (3) add `test_selftest_never_creates_tmpdir` (R5); (4) re-record the self-test and the other controls at
the final head. Round 1's flight records are not yours to copy; its `.fleet/tests/test_docs_truth.py` is at the same ref
and IS the file R5 extends.

## Fire

Base and pin `7cfa17715682dfe72eaacd6c17f9edd87aa1620b`; venue 205 under Tom's caps (3 executing build
workloads; one-firer guard); `.fleet/` never reaches the forge. Landed by the owner's hand on
`judge_pass`: five-file-class overlay of the judged tree onto current main minus `.fleet/`, gate + self-test
+ R3(i) re-run at the owner's seat, receipts from post-push re-reads.
