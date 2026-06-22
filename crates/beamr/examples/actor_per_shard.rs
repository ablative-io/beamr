//! Worked actor-per-shard skeleton (NATIVE-003).
//!
//! A sharded key/value store built from native beamr actors — the copy-ready
//! pattern haematite's shard actor (CORE-007) and liminal's channel/conversation
//! actors lift directly to replace faked synchronous structs. It is entirely
//! self-contained within `beamr` (no haematite/liminal dependency).
//!
//! It demonstrates the full NATIVE-003 surface:
//!
//! * [`spawn_actor`] — one restart-capable native actor per shard.
//! * [`SenderHandle::cast`] — fire-and-forget writes (`put`).
//! * [`SenderHandle::call`] — request/reply reads (`get`), correlated by ref.
//! * [`ActorContext::spawn_child`] — a supervisor actor spawning a linked,
//!   restart-capable child shard at run time.
//!
//! # What crosses the boundary
//!
//! Actors exchange immediates/refs/scalars only — here small-integer keys and
//! values, and tuples of them. A raw closure with free variables must never be
//! sent to an actor that may be Executing (the pre-existing ETF
//! closure-encoding limitation); the gen_server pattern this example follows
//! sidesteps that by construction.
//!
//! # The call deadlock hazard
//!
//! [`SenderHandle::call`] BLOCKS until the reply arrives. That is correct for an
//! external driver (this `main`, which owns the scheduler). It would DEADLOCK if
//! called from inside an actor handler — so a handler is given an
//! [`ActorContext`] that offers only `cast`. Intra-actor request/reply uses
//! `cast` + an explicit reply message carrying `ctx.self_pid()`.
//!
//! Run with: `cargo run -p beamr --example actor_per_shard`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use beamr::module::ModuleRegistry;
use beamr::scheduler::{Scheduler, SchedulerConfig};
use beamr::term::Term;
use beamr::term::boxed::Tuple;
use beamr::{
    Actor, ActorContext, ActorMessage, ActorRef, NativeContext, SenderHandle, spawn_actor,
};

/// A `get` request: read the value stored under `key` (reply is the value, or
/// `-1` when the key is absent). Encoded as the bare small-integer key.
#[derive(Clone)]
struct Get {
    key: i64,
}

impl ActorMessage for Get {
    fn encode(&self, _ctx: &mut NativeContext<'_>) -> Option<Term> {
        Term::try_small_int(self.key)
    }

    fn decode(term: Term) -> Option<Self> {
        Some(Self {
            key: term.as_small_int()?,
        })
    }
}

/// A `put` write: store `value` under `key`. Encoded as a `{key, value}` tuple
/// of immediates.
#[derive(Clone)]
struct Put {
    key: i64,
    value: i64,
}

impl ActorMessage for Put {
    fn encode(&self, ctx: &mut NativeContext<'_>) -> Option<Term> {
        ctx.alloc_tuple(&[
            Term::try_small_int(self.key)?,
            Term::try_small_int(self.value)?,
        ])
    }

    fn decode(term: Term) -> Option<Self> {
        let tuple = Tuple::new(term)?;
        Some(Self {
            key: tuple.get(0)?.as_small_int()?,
            value: tuple.get(1)?.as_small_int()?,
        })
    }
}

/// One shard: owns a slice of the key space in private Rust state.
struct Shard {
    store: HashMap<i64, i64>,
}

impl Shard {
    fn new() -> Self {
        Self {
            store: HashMap::new(),
        }
    }
}

impl Actor for Shard {
    type Call = Get;
    type Reply = i64;
    type Cast = Put;

    fn handle_call(&mut self, request: Get, _ctx: &mut ActorContext<'_, '_>) -> i64 {
        self.store.get(&request.key).copied().unwrap_or(-1)
    }

    fn handle_cast(&mut self, request: Put, _ctx: &mut ActorContext<'_, '_>) {
        self.store.insert(request.key, request.value);
    }
}

/// A trivial supervisor actor: on each call it spawns a fresh, linked,
/// restart-capable [`Shard`] child and replies with the child's pid.
struct ShardSupervisor;

impl Actor for ShardSupervisor {
    type Call = i64;
    type Reply = i64;
    type Cast = i64;

    fn handle_call(&mut self, _request: i64, ctx: &mut ActorContext<'_, '_>) -> i64 {
        // The child is built through the NATIVE-002 factory path, so a
        // supervisor can restart it by re-invoking the factory; the link means
        // its exit propagates back here.
        ctx.spawn_child(Shard::new)
            .and_then(|pid| i64::try_from(pid).ok())
            .unwrap_or(-1)
    }

    fn handle_cast(&mut self, _request: i64, _ctx: &mut ActorContext<'_, '_>) {}
}

/// Number of shards the key space is partitioned across.
const SHARD_COUNT: i64 = 4;

fn shard_index(key: i64) -> usize {
    key.rem_euclid(SHARD_COUNT) as usize
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scheduler = Arc::new(
        Scheduler::new(SchedulerConfig::default(), Arc::new(ModuleRegistry::new()))
            .map_err(|error| format!("scheduler failed to start: {error}"))?,
    );

    // One restart-capable native actor per shard.
    let mut shards: Vec<ActorRef<Shard>> = Vec::new();
    for _ in 0..SHARD_COUNT {
        shards.push(spawn_actor(&scheduler, Shard::new)?);
    }

    // Writes are fire-and-forget casts, routed to the owning shard.
    let entries = [(1, 100), (2, 200), (5, 500), (10, 1_000), (7, 700)];
    for (key, value) in entries {
        shards[shard_index(key)].sender.cast(Put { key, value })?;
    }

    // Casts are asynchronous; let them settle before reading back.
    std::thread::sleep(Duration::from_millis(50));

    // Reads are blocking request/reply calls (safe here: main is the external
    // driver, not an actor handler).
    for (key, expected) in entries {
        let got = shards[shard_index(key)].sender.call(Get { key })?;
        assert_eq!(got, expected, "shard must return the stored value");
        println!("get({key}) -> {got} (shard {})", shard_index(key));
    }

    // A missing key reads back as the -1 sentinel.
    let missing = shards[shard_index(3)].sender.call(Get { key: 3 })?;
    assert_eq!(missing, -1);
    println!("get(3) -> {missing} (absent)");

    // Supervisor pattern: spawn a supervisor actor, ask it (by call) to spawn a
    // linked, restart-capable child shard, then talk to that child by pid.
    let supervisor = spawn_actor(&scheduler, || ShardSupervisor)?;
    let child_pid = supervisor.sender.call(0)?;
    assert!(child_pid > 0, "supervisor returns a live child pid");
    let child: SenderHandle<Shard> = SenderHandle::attach(&scheduler, u64::try_from(child_pid)?);
    child.cast(Put {
        key: 42,
        value: 4_242,
    })?;
    std::thread::sleep(Duration::from_millis(50));
    let child_value = child.call(Get { key: 42 })?;
    assert_eq!(child_value, 4_242);
    println!("supervised child {child_pid}: get(42) -> {child_value}");

    scheduler.shutdown();
    println!("actor-per-shard example completed");
    Ok(())
}
