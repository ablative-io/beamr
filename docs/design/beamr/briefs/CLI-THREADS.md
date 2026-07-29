BRIEF — CLI-THREADS: beamr's CLI hardcodes a single scheduler thread. Dispatch-ready, not yet dispatched. Author: Artemis Peach (…c9fa), beamr owner seat. Provenance: finding from my own fresh-eyes user walk (task #48), still unaddressed; re-verified at the bytes 2026-07-29 at main 9989828.

═══ THE FACT, VERIFIED ═══

`crates/beamr-cli/src/main.rs:286`:

    SchedulerConfig {
        thread_count: Some(1),
        ..SchedulerConfig::default()
    },

`SchedulerConfig` (`crates/beamr/src/scheduler/mod.rs:284`) derives `Default`, so
`thread_count: Option<usize>` defaults to `None`. The CLI therefore does not
inherit a default — it EXPLICITLY OVERRIDES to one thread.

Two aggravating details, both verified:
- **The field has no doc comment.** Line 285 is bare, while its siblings in the
  same struct are documented (`dirty_cpu_threads` explains None/Some(n)/Some(0)
  precisely, citing spec §3.2). So what `None` means for `thread_count` is not
  stated anywhere in the type.
- **The comment adjacent to the override explains something else.** The prose
  immediately below `thread_count: Some(1)` is about `SchedulerServices::full_runtime()`
  and the distribution profile. A reader naturally attaches it to the line above.
  Nothing anywhere says why one thread.

═══ WHY IT MATTERS ═══

beamr is a BEAM runtime. Concurrency across schedulers is the core of what a BEAM
is for. The CLI is the front door — the first thing a user runs, the thing the
README's quickstart drives — and it runs the VM on ONE scheduler thread, with no
flag to change it and no statement that it is doing so.

This is a credibility gap before it is a performance gap: a user benchmarking or
evaluating beamr through the CLI measures a single-scheduler VM and may reasonably
conclude that is what beamr is.

═══ THE QUESTION THAT MUST BE ANSWERED BEFORE ANY FIX ═══

**IS THE SINGLE THREAD LOAD-BEARING?** Do not change the value until this is settled
at the bytes. Two candidate reasons it might be deliberate:

1. **Replay determinism.** `tick_replay_timers` is called on THREAD 0 only
   (`execution.rs:317-319`), and the replay driver's event ordering assumes a
   single consumer. Multi-threading the CLI could make replay non-deterministic —
   which matters even though the recorder is currently unwired, because wiring it
   is queued work (see the R2 resequencing).
2. **Output ordering.** The CLI is run-and-print; multiple schedulers may interleave
   stdout in ways the current capture sink does not order.

If either holds, THE FIX IS NOT "raise the number" — it is to state the constraint
in the type, in the CLI's help, and in the README, and to make the coupling explicit
so the next person does not silently break replay by "optimising" this line.

═══ DELIVERABLE, whichever way the question resolves ═══

A. **If the single thread is incidental:** a `--threads N` flag defaulting to the
   library default, with the CLI no longer overriding. Walls: a test proving N
   schedulers are actually created; a test proving the default is not 1.
B. **If the single thread is required:** a doc comment on `SchedulerConfig::thread_count`
   stating what `None` means; a comment at the override naming the constraint and
   what breaks without it; a README line; and — the part that makes it a control
   rather than a note — a test that FAILS if the CLI's thread count is changed
   without the coupled constraint being addressed.

Either way: no new `#[allow]`, no unwrap/expect/panic outside cfg(test), red-first,
branch off current main, push the branch never main, battery derived from the current
canon blob with the hash verified at the executor's own bytes, MEMBER_ID echoed.

SIZE: small if (B), medium if (A). The investigation is the majority of the work and
is READ-ONLY — it can start on any box without a battery slot.

═══ ADDENDUM 2026-07-29 21:22Z — SPLIT DISPATCH + IDENTITY CONSTRAINT (Artemis) ═══

This brief is now dispatched in TWO HALVES, because the investigation is
read-only and the fix is not, and because tonight's identity ruling has
made battery venues scarce.

HALF A — INVESTIGATION, dispatched to Diana Plum, Annabel's box.
Read-only. Answers exactly one question: IS THE SINGLE THREAD LOAD-BEARING?
Deliverable is a finding, not a patch: the answer, the evidence at the
bytes, and which of deliverable (A) or (B) below it selects. NO code
change, NO battery, NO claim — so the canon-gated battery block on that
box does not touch it. If the investigation cannot settle it from the
source, that is itself the finding and it should be reported rather than
resolved by judgement.

HALF B — THE FIX, HELD. Not dispatched. It is gated on half A's answer
selecting (A) or (B), and on a battery venue existing. Whoever takes it
inherits this brief unchanged.

IDENTITY CONSTRAINT, applying to half B only (half A takes no claim, so
it has no MEMBER_ID and no runner):
- The dispatching brief MUST state what `MEMBER_ID` was set FROM, and the
  dispatcher compares the evidence `member_id` against the server-stamped
  `author_id` on the operator's own reply. Member against member; the
  server's copy is derived, so it is a real control rather than a second
  assertion. This is MITIGATION, not closure.
- Reattachment remedies DO NOT reach beamr: our runner is r3-derived and
  contains no session identifier anywhere, so it makes no session-to-session
  comparison for reattachment to repair. Measured 2026-07-29.
- Annabel's box is OUT for canon-gated batteries until fork hygiene lands.

WHY THIS SPLIT IS NOT A DELAY: the brief already stated that the
investigation is the majority of the work and can start on any box
without a battery slot. Splitting it lets that majority proceed while the
minority waits on a venue, instead of the whole item waiting.
