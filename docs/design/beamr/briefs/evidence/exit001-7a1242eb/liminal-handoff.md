# R5(b) — the liminal-side handoff: what F7's implementer does with this primitive

**beamr does not own this change.** EXIT-001 closes F7's DEPENDENCY — the
missing scheduler-to-waiter notification — not F7 itself. This record
exists so the liminal implementer does not re-derive the symbols, and so
no reader mistakes this brief for the F7 close.

## What liminal deletes

- `LIVENESS_POLL` (10 ms), `stack/liminal/crates/liminal/src/channel/actor/wait.rs:24`
- the `poll_reply` process-table sampling loop, `wait.rs:73-95` — the
  `scheduler.process_table().get(pid)` cadence that existed only because
  the notification primitive did not.

## What liminal composes instead (its own side, its own brief)

Arm ONE `Scheduler::watch_exit(pid)` at command publication and compose
four sources: the reply path, the exit watch, disconnect, and ONE
deadline. Registration's typed answer matters at the arm site:
`AlreadyExited` lets the waiter skip arming a deadline against a dead
target and fail the command fast (the §5.5 requirement); `NoRecord` is
the typed never-ran answer and must not be conflated with still-live.

## The race semantics liminal inherits (RULED, final)

Reply wins, with mandatory post-exit drain (Waffles 2026-07-29 16:16Z,
Hermes assented, lane entry `99f4e608`): an exit-notification wake means
NO FURTHER replies, never that prior replies are void. The waiter's
obligation on an exit wake is to drain the reply path FIRST; only an
empty final check concludes died-without-reply. The beamr-side invariant
this rests on — a value enqueued strictly before the kill is observable,
with its value, by the time the watch fires — is pinned in beamr's own
suite by
`wall1_reply_enqueued_strictly_before_kill_is_observable_with_value_at_watch_wake`
(`crates/beamr/src/scheduler/exit_observation_tests.rs`), with both of
Hermes's riders (value-assert; enqueue-strictly-before-kill).

## Contract sockets this plugs into

- `stack/liminal/docs/design/W4-LAW1-POLLING-RETIREMENT.md:215` — the F7
  blocked-on-external ledger line this dependency closes.
- `stack/liminal/docs/design/LAW1-POLLING-RETIREMENT.md` §5.3–5.6 (:366 ff)
  — the registration/already-dead/no-cadence requirements `watch_exit`'s
  contract answers point-for-point.
- `stack/liminal/docs/design/PARTICIPANT-CONTRACT.md:401, :6277` — the
  ledger's "primitive not selected" framing, now stale: the primitive
  exists (`watch_exit`), additively, without touching the exclusive
  subscription liminal-server's connection supervisor already holds
  (`supervisor.rs:1055`).

## What liminal must NOT do

Do not touch `subscribe_exit_events` — liminal-server's supervisor holds
the one slot and aion holds the one slot in its own process; the watch
surface exists precisely so no third waiter competes for it. And no
polling replacement of any kind: a repair that trades the 10 ms poll for
another cadence has failed, not shipped (EXIT-001 boundary).
