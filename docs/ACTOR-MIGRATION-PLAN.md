# Actor Migration Plan — faked/stub actors → real native beamr processes

> **Status: plan, 2026-06-22.** Produced by a read-only scoping pass, reviewed. Migrates the Ablative
> stack's faked actors onto the landed beamr native-process API (NativeHandler / Actor / spawn_actor on
> beamr main). IMPLEMENTATION is gated on Step 0 (beamr 0.8.0 availability). Strict-linear DAG
> (beamr → haematite → liminal → aion) verified — no migration step adds a back-edge.

## The central technical risk: binary across the beamr term boundary
Shard and channel messages carry `Vec<u8>` keys/values. `NativeContext` has no `alloc_binary`, and the
`Actor` facade's encode/decode is limited to immediates/tuples. **Decision needed per target:** (1) a
shared-memory bridge — deposit bytes in a staging map, pass an integer handle as the Term (PROVEN: the
conversation actor already does this via `QueuedCommand`/mpsc); or (2) raw beamr binary heap terms (cleaner,
but needs an `alloc_binary`-style API that may not be public yet). **Lean: option (1), the proven bridge.**
Resolve this before writing the shard handler — the wrong choice breaks correctness under ETF delivery to an
Executing receiver.

## Step 0 — GATE (Tom's decision): make the native API available to dependents
haematite + liminal depend on published `beamr = "0.7.0"` (no native API; it's post-0.7.0 on local main,
unpublished + unpushed). Options: publish beamr **0.8.0** (via beamr/scripts/release.sh; clean, committable)
**or** path/git-dep for co-dev (local-only / needs beamr pushed). Lean: publish 0.8.0. Nothing implements until this clears.

## Target 1 — haematite CORE-007 shard actor — FAKED (migrate FIRST)
Faked via `extern crate self as beamr; pub type Pid = u64` + `std::thread`+`mpsc` + a global `OnceLock<Runtime>`
process registry + `catch_unwind` supervision. Full impl lives ONLY on branch `stacked-dev-CORE-007` (45de590);
main has a minimal stub → **must rebase onto main first** (past PERSIST-003 + BRANCH-002), remove the fake.
Migration: use **`NativeHandler` directly** (not the Actor facade — binary messages); storage hot path
(DiskStore/WalBuffer/DurableWal/Cursor) stays native Rust as `self.` fields; Get/Put/Delete/Commit = call-style
(reply_to pid), Range = streaming multi-reply; supervision → `NativeHandlerFactory` restart (factory captures
store+wal paths, re-runs `initialise` WAL-recovery). **Invariants:** WAL-before-buffer; committed-root marker
after tree commit; history-independence (CN5); per-shard write serialization (free under the scheduler); delete
the global RUNTIME singleton (removes the TEST_LOCK hazard). Why first: deepest layer; resolves the binary
pattern for the channel actor; kills the original fake + global singleton.

## Target 2 — liminal channel actor — SYNC STUB (migrate SECOND)
`Arc<Mutex<ChannelActor>>`, no beamr; `registry.rs`/`supervisor.rs` are empty stubs. Deepest DESIGN work: the
delivery model must change from shared-memory `VecDeque` polling to beamr `ctx.send(subscriber_pid, ...)`, so
`SubscriptionHandle`'s public API changes (subscriber-as-process vs polling-client — decide before coding).
Can use the `Actor` facade; reuse the Step-1 binary bridge for `Publish` payloads + `Schema`. Write
registry.rs (ChannelId→ActorRef) + supervisor.rs from scratch (per the actor_per_shard example). Keep schema
validation synchronous from the publisher (Publish = call, not cast). `ChannelMode::Durable` depends on Target 1.

## Target 3 — liminal conversation actor — ALREADY REAL beamr (cleanup LAST, deferrable)
Already a real beamr process (ProcessContext, Scheduler, spawn_trap_exit, hand-built bytecode module + NIF).
**No correctness migration.** Optional cleanup under the new API: replace the hand-built `actor_module()`
bytecode with `NativeHandler::handle` + `ctx.set_trap_exit(true)`, eliminate the global `ActorRuntime` pid→core
registry, drop `nif_private_data`. KEEP the `QueuedCommand`/`mpsc::SyncSender` bridge (correct external-caller
pattern). Deferrable entirely; fold into the same PR as the 0.8.0 bump if cheap.

## Sequence
Step 0 (publish/dep gate — Tom) → Step 1 (shard: rebase CORE-007 onto main → resolve binary encoding → migrate
→ remove fake+singleton) → Step 2 (channel: design subscription model → migrate → write registry/supervisor) →
Step 3 (conversation cleanup, optional/last). Each delegated + adversarially reviewed; I coordinate + verify.
