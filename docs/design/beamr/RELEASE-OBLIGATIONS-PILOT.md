# RELEASE-OBLIGATIONS PILOT — beamr, built under the 2026-08-09 ruling

Artemis Peach. Ruling: `ablative/docs` `tracking/ruling-release-obligations-20260809.md`
(commit `1369278`, both halves — Cally Ray + Waffles). Origin: the citable miss —
board #65's "re-open `EtsOrderedSet::new` delete-vs-keep at the next real release"
was not honoured at 0.18.0 because no path existed by which cutting a release
could consult a seat's board. The ruling fixed five properties and left file
name, format, and hook points to the pilot. This is the pilot.

## What was built

| piece | path |
|---|---|
| The obligations file | `RELEASE-OBLIGATIONS.json` (repo root, beside `gates.json` — the same in-repo, read-at-run-time pattern) |
| The gate | `scripts/release-obligations-gate.py` (stdlib-only; shebang pins `/usr/bin/python3` — the env shim on the release box is broken, measured rc 126) |
| Its controls | `scripts/release-obligations-fixtures/{due_open,missing_reason,resolved_ok}.json` — run EVERY invocation before the real file is graded (the `gate-blocking-call.sh` pattern) |
| The hook | `scripts/release.sh` runs the gate FIRST, dry-run included, before announcing intent; `set -e` makes any non-zero abort the run |
| The human mirror | `RELEASE_CHECKLIST.md` "Release constraints" names the gate and forbids bare `cargo publish` around it |

**due grammar:** `next-release` (due at any cut) or `release:X.Y.Z` (due when the
version being cut ≥ X.Y.Z). The version being cut is read from
`crates/beamr/Cargo.toml` — the same source `release.sh` publishes, so the gate
and the act cannot disagree.

## The five ruled properties, each mapped and each demonstrated

1. **In the repo** — `RELEASE-OBLIGATIONS.json` is tracked at the root.
2. **On the path the act takes** — arm E (`gate-logs/65/falsifier-arms.txt`):
   `release.sh --dry-run` aborts at the gate with rc 1 **before printing
   "intends to publish"**. The act cannot run without consulting the file.
3. **The gate REFUSES — it does not list** — there is no warn/list mode in the
   code at all; a due-open obligation is exit 1. Live run
   (`gate-logs/65/live-refusal.txt`): REFUSING release of 0.18.2 on OB-001.
4. **The reason is a required field** — an open obligation with no
   `reason_unresolved` grades MALFORMED (exit 2), demonstrated by arm B on the
   real file and by the `missing_reason` control every run.
5. **Absence ⇒ RAISE, never SKIP** — arm A: file moved aside ⇒ exit 2.
   Unparseable JSON and unknown status/due values also grade malformed.
   Deleting the file cannot disarm the alarm.

Exit codes: 0 pass · 1 refused (due-open) · 2 raised (absent/malformed) ·
3 instrument broken (a control failed — arm C demonstrates, rc 3).

## Instrument discipline

- The three controls run every invocation through the SAME `grade()` code path
  as the real file — a control through a side door proves nothing. Arm C broke
  a control on purpose: the gate exits 3 and refuses.
- Controls are keyed on committed synthetic fixtures, never on live
  obligations — the AR-1 R10-a lesson (a control keyed on a live entry is
  destroyed by the resolution it exists to survive).
- No stderr suppression anywhere; no bypass flag in `release.sh`, on purpose.

## The recorded obligations at pilot creation

- **OB-001** `EtsOrderedSet::new` delete-vs-keep — `next-release`, OPEN. The
  citable miss, now armed: the next cut REFUSES until it is decided or a lead's
  ruling is recorded as its new reason. Subject verified live at
  `ets/ordered_set.rs:52`.
- **OB-002** delete the `spawn_process` raw-ip-0 scaffold — `release:0.19.0`,
  OPEN (task #104's parked delete, moved from "the 0.19.0 cutter's list" prose
  onto the path). Correctly NOT due at 0.18.2 (arm D's pass proves the
  due-arithmetic distinguishes them).
- **OB-003** `spawn_link_dirty` removal — RESOLVED exemplar, recorded
  retroactively: task #38 parked it to "next breaking release" and **it was
  already discharged at the 0.17.0 window** (verified absent from production at
  recording). Recorded as resolved rather than open because verify-before-record
  found the obligation dead — recording it open would have been the class's
  inverse: a phantom obligation blocking a cut.

## Falsifier transcript (`gate-logs/65/falsifier-arms.txt`)

A: absence → rc 2 · B: reason stripped on the real file → rc 2 · C: control
broken → rc 3 · D: OB-001 resolved in a temp copy → rc 0 (OB-002 not due) ·
E: release.sh aborts at the gate before intent → rc 1. All restores
sha256-verified; live tree byte-identical after.

## Disclosures

- **No battery run for this landing, stated loudly:** the delta is
  sh/py/json/md — zero Rust bytes, zero `gates.json` legs touched, no CI
  workflow changed. The gate carries its own executed control suite and five
  falsifier arms, transcribed above. A grader who wants the canon battery
  anyway orders it and I run it.
- **Residual bypass, named:** a hand-typed `cargo publish -p <crate>` does not
  pass through `release.sh`. The checklist now forbids it in words; the ruled
  path is `release.sh`. Closing the bypass mechanically (e.g. a workspace
  publish alias or CI tag-gate) is follow-up work, not claimed here.
- **Scope guard from the ruling, restated:** a green pilot in beamr says
  nothing about any other repo's release-keyed obligations, and no sweep can
  count that class — the recorded half is the half least likely to be
  forgotten.
