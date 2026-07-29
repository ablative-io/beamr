# R5(a) — sibling lane, ledgered NOT fixed: `run_until_exit`'s 10 ms poll

EXIT-001 deliberately does not touch this. Recorded forward-only per
Waffles' rider (2026-07-29 05:31Z) so the next planner can size it
without re-reading the code.

## The finding, at the bytes (re-verified at build base `192e4a4`)

`Scheduler::run_until_exit` (`crates/beamr/src/scheduler/execution.rs:142-156`)
is itself a polling wall: it loops on `exit_tombstones.get(&pid)` behind
`wake_condvar.wait_timeout(guard, 10ms)`. That is the same LAW1 shape this
brief retires for liminal's channel waiter — a 10 ms cadence sampling a
store instead of blocking on a notification — inside beamr's own walls.

## Why it is a separate lane

- F7's unblock must not wait on it: liminal's waiter needs `watch_exit`,
  not a reworked `run_until_exit`.
- `run_until_exit` is a long-standing public blocking API; its callers
  (the CLI among them) are outside EXIT-001's ground, and changing its
  wake mechanism changes observable latency characteristics for every
  embedder.

## The one-paragraph repair sketch

`run_until_exit(pid)` is trivially expressible on the primitive this brief
introduces: register `watch_exit(pid)` FIRST, then read the tombstone once
(register-then-check, the same soundness argument as D3); on
`AlreadyExited`/tombstone-hit return immediately, otherwise block on
`ExitWatch::recv()` — no condvar, no timeout loop, no cadence. The
`exit_results` read stays as-is at wake. The one design question the lane
must answer is whether `run_until_exit`'s legacy-tombstone semantics
(bounded, evictable) are part of its contract or an implementation detail
— the watch's durable-record answer is strictly stronger, which is a
behaviour CHANGE for a caller relying on eviction-as-forgetting.

## The cost of NOT fixing it

One wake per 10 ms per outstanding `run_until_exit` caller (100 wakes/s
each), paid whether or not anything exited — the same per-caller idle tax
the walk's finding 2 describes at the scheduler level, here per blocked
API call. A server parking many `run_until_exit` callers concurrently
multiplies it linearly.
