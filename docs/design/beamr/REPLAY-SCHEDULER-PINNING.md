# Replay is pinned to one scheduler, and record must be pinned to match

Ruled 2026-07-29 by Artemis Peach (beamr owner seat), on evidence found
by Diana Plum during an unrelated CLI investigation and independently
confirmed and sharpened by Seth Crackers. Every cite below was verified
at three seats' own bytes at `9989828`, not carried on anyone's word.

This note exists because the decision it records would otherwise be made
silently by whoever wires the recorder, and would look like success.

## The constraint, as the code stands

`scheduler/mod.rs:1267-1271` — when a replay driver exists, the thread
count is forced to `1` and the configured value is discarded:

```rust
let replay_enabled = replay_driver.is_some();
let thread_count = if replay_enabled { 1 } else { configured_thread_count(config.thread_count) };
```

`scheduler/execution.rs:830-841` — `next_replay_pid` will only consume an
event belonging to the calling scheduler:

```rust
let event = guard.peek_schedule()?;
if event.scheduler_index != my_index || shared.process_table.get(event.pid).is_none() {
    return None;
}
```

`scheduler_index` is real recorded data, not an artefact: `RecordedSchedule`
carries it (`replay/driver.rs:200-209`) and `record_schedule`
(`replay/recorder.rs:53-68`) sets it from a caller-supplied argument.

**Together: replay runs on exactly one scheduler, whose `my_index` is
always `0`, so only index-`0` events are ever consumable. A recording made
on N > 1 schedulers is unreplayable by construction.**

## The failure mode is a silent hang, not an error

This is the part that decides how the constraint must be enforced.
`execution.rs:383` takes the `None` arm and parks:

```rust
match next_replay_pid(shared, my_index) {
    Some(pid) => pid,
    None => { park_thread(shared); continue; }
}
```

No error, no refusal, no diagnostic. Under a forced single scheduler there
is no other worker to make progress and `my_index` can never change, so the
event never becomes consumable. **A multi-scheduler log does not fail
replay — it hangs the VM forever, producing a process that looks like it is
working.** Anyone hitting it will hunt their own program, because nothing
points at the harness.

That is the same class the estate named elsewhere the same day: presence
does not distinguish working from deadlocked. Here it arrives inside the
replay loop.

## Ruled

1. **Pin record to a single scheduler**, enforced at the same construction
   site that already forces replay to `1` — symmetric enforcement in the
   library, *not* a CLI-level override, which would also pin ordinary `run`
   and repeat the mistake this project has just finished removing from the
   CLI.
2. **Refuse at ingest, not only at construction.** If a log carrying a
   non-zero `scheduler_index` is ever loaded, reject it with a named error
   before it can reach the scheduler loop. This costs one check and converts
   an invisible infinite hang into a loud refusal. A constraint enforced at
   construction but unenforced at ingest is still a trap for anyone holding
   an older log.
3. **State the constraint and its consequence in a comment at the pin**, and
   record the unbuilt alternative below. The choice is what must survive,
   not merely its effect.

## The alternative that was not built, and why it is not foreclosed

**(b) Stop forcing replay to one scheduler and consume events per scheduler
index across N threads.** This is the only design under which record/replay
can reproduce a *concurrency* bug — which, for a BEAM, is a substantial part
of what such a feature is for. It is strictly more work and it is not
required for a first working round trip.

Taking (1) does not foreclose (b). **Shipping (1) undocumented would** — a
year of code written against an unexamined single-scheduler assumption is
what makes the alternative unreachable, not the pin itself. Hence rule 3.

## Why this is the cheapest possible moment

`ReplayRecorder` is never constructed anywhere: its only references are the
re-export at `replay/mod.rs:27`, the struct at `recorder.rs:13`, and its own
`impl` at `recorder.rs:18`. `record_schedule` has no callers. Verified with a
positive control (`ReplayDriver::new` does have real call sites, so the
search finds constructions where they exist).

**So there are no multi-scheduler recordings in existence to be broken.**
This is not a restriction on behaviour anybody has; it is a decision about
behaviour that does not exist yet, taken before anything depends on it. That
argument for pinning does not rely on pinning being the easier option.
