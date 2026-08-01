# Window-supersession residue — disclosed pre-landing, not absorbed

`battery.sh`'s header cites the window it was authored under: Cally Ray,
03:40:25Z, 56.55 GiB at her hands. That citation was accurate at authoring and
at launch. **Mid-run, the window was withdrawn and superseded**, and one of the
new window's terms could not be executed on a script already in flight. This
file is the record of that gap.

## Timeline (UTC, 2026-08-01)

- **04:06:24Z** — battery STARTED (six legs, monolithic script, no between-leg
  hooks: fmt → clippy → wasm32-check → wasm-tests → tests → ast-grep).
- **04:07:13Z** — Cally withdraws the 03:40:25Z window ("the window's noun is
  DISK and the disk moved") and declares a new one: 40.18 GiB @04:06:31Z,
  falling ~2 GiB/min, three lanes concurrent. Terms: the leg in flight runs to
  completion (no mid-leg kills); **between legs, `df -k /System/Volumes/Data`
  at my hands, HOLD under 40 GiB** and ping Cally + Athena rather than starting
  the next leg. Bands: 35 = Athena escalates to Tom; 25 = hard floor.
- **~04:09Z** — the supersession first reaches my hands (it arrived while my
  session was compacting; first sight was post-compaction).
- **04:09:56Z** — my df reading, mid-tests-leg: **39.36 GiB — under the 40
  boundary**. I moved to arm a boundary watcher to enforce the hold
  mechanically (watch for `tests.exit`, read df at that instant, kill the
  runner by exact PID if under 40 so the next leg would not start).
- **04:10:05–07Z** — before the watcher existed, the tests leg completed and
  the script ran its tail: ast-grep leg, `du -sk target`, COMPLETE.marker
  (04:10:07Z). The runner was already gone when I looked for its PID.
- **04:13:24Z** — post-run df: 39.20 GiB.

## The gap, stated plainly

Every leg boundary that fell after 04:07:13Z — at minimum wasm-tests→tests and
certainly **tests→ast-grep (~04:10:05Z)** — passed **without a df check at my
hands and without a hold**, because a monolithic script auto-starts its next
leg and the run finished before any enforcement mechanism could be built.
The nearest readings bracket the unchecked boundary: 39.36 GiB at 04:09:56Z
(mid-leg) and 39.20 GiB at 04:13:24Z (post-run) — both under 40.

The work that ran past the unchecked boundary was the ast-grep scan
(read-only), one `du -sk`, and the marker write — no builds, no material disk
draw. That is stated as **fact, not authorization**: "read-only" is not
automatically outside a declared class, and a clean outcome never authorizes
retroactively.

## Disposition

**PUSH HELD.** All six legs are green at the bytes (per-leg exits, denominators
proven), but push-on-green is not exercised across an unexecuted window term
without Cally's word. This residue goes to her gate with the package.

Forward fix, proposed: the reference battery form gains a between-leg guard —
each leg preceded by a df read written to the artifact
(`boundary-<leg>.df`), with a threshold file the coordinator can set so a
declared floor holds the script itself, no watcher race. Her call whether that
rides the next battery or a form amendment.
