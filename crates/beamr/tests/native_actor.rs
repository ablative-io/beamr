//! End-to-end gate tests for the ergonomic native-actor API (NATIVE-003).
//!
//! Every test boots a REAL multi-threaded scheduler and drives actors through
//! the same machinery as bytecode processes — through the public crate-root
//! surface only (`beamr::Actor`, `beamr::spawn_actor`, `beamr::SenderHandle`),
//! so it also proves the re-exports are importable from the crate root.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use beamr::module::ModuleRegistry;
use beamr::scheduler::{Scheduler, SchedulerConfig};
use beamr::term::Term;
use beamr::term::boxed::Tuple;
use beamr::{Actor, ActorContext, ActorMessage, NativeContext, SenderHandle, spawn_actor};

fn scheduler() -> Arc<Scheduler> {
    Arc::new(
        Scheduler::new(SchedulerConfig::default(), Arc::new(ModuleRegistry::new()))
            .expect("scheduler starts"),
    )
}

// --- A counter actor: call(Add(n)) -> running total; cast(Add(n)) accumulates.

#[derive(Clone)]
struct Add(i64);

impl ActorMessage for Add {
    fn encode(&self, _ctx: &mut NativeContext<'_>) -> Option<Term> {
        Term::try_small_int(self.0)
    }
    fn decode(term: Term) -> Option<Self> {
        Some(Self(term.as_small_int()?))
    }
}

struct Counter {
    total: i64,
    call_seen: Arc<AtomicBool>,
    cast_seen: Arc<AtomicU64>,
}

impl Actor for Counter {
    type Call = Add;
    type Reply = i64;
    type Cast = Add;

    fn handle_call(&mut self, request: Add, _ctx: &mut ActorContext<'_, '_>) -> i64 {
        self.call_seen.store(true, Ordering::SeqCst);
        self.total += request.0;
        self.total
    }

    fn handle_cast(&mut self, request: Add, _ctx: &mut ActorContext<'_, '_>) {
        self.cast_seen.fetch_add(1, Ordering::SeqCst);
        self.total += request.0;
    }
}

#[test]
fn call_returns_handle_call_reply_and_cast_reaches_handle_cast() {
    let scheduler = scheduler();
    let call_seen = Arc::new(AtomicBool::new(false));
    let cast_seen = Arc::new(AtomicU64::new(0));
    let call_for_actor = Arc::clone(&call_seen);
    let cast_for_actor = Arc::clone(&cast_seen);

    let counter = spawn_actor(&scheduler, move || Counter {
        total: 0,
        call_seen: Arc::clone(&call_for_actor),
        cast_seen: Arc::clone(&cast_for_actor),
    })
    .expect("spawn counter");

    // A cast accumulates without a reply. cast and call route through separate
    // transient processes, so their delivery to the actor can race; wait until
    // the cast is observed before issuing the call so the assertion is
    // deterministic (this models a real client that orders its own effects).
    counter.sender.cast(Add(5)).expect("cast");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while cast_seen.load(Ordering::SeqCst) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "cast was never observed"
        );
        std::thread::sleep(Duration::from_millis(2));
    }

    // A later call observes the accumulated state and replies.
    let total = counter.sender.call(Add(3)).expect("call");
    assert_eq!(
        total, 8,
        "call reply reflects the prior cast plus this call"
    );
    assert!(call_seen.load(Ordering::SeqCst), "handle_call ran");
    assert_eq!(cast_seen.load(Ordering::SeqCst), 1, "handle_cast ran once");

    scheduler.shutdown();
}

#[test]
fn two_concurrent_calls_do_not_cross_replies() {
    // A doubling actor: handle_call(n) -> 2n. Two distinct inputs run
    // concurrently; a crossed reply would surface the wrong product.
    #[derive(Clone)]
    struct N(i64);
    impl ActorMessage for N {
        fn encode(&self, _ctx: &mut NativeContext<'_>) -> Option<Term> {
            Term::try_small_int(self.0)
        }
        fn decode(term: Term) -> Option<Self> {
            Some(Self(term.as_small_int()?))
        }
    }
    struct Doubler;
    impl Actor for Doubler {
        type Call = N;
        type Reply = i64;
        type Cast = N;
        fn handle_call(&mut self, request: N, _ctx: &mut ActorContext<'_, '_>) -> i64 {
            // A small spin so both calls are genuinely in flight together.
            std::thread::sleep(Duration::from_millis(20));
            request.0 * 2
        }
        fn handle_cast(&mut self, _request: N, _ctx: &mut ActorContext<'_, '_>) {}
    }

    let scheduler = scheduler();
    let doubler = spawn_actor(&scheduler, || Doubler).expect("spawn doubler");

    let handle_a: SenderHandle<Doubler> = doubler.sender.clone();
    let handle_b: SenderHandle<Doubler> = doubler.sender.clone();
    let thread_a = std::thread::spawn(move || handle_a.call(N(21)).expect("call a"));
    let thread_b = std::thread::spawn(move || handle_b.call(N(50)).expect("call b"));

    let reply_a = thread_a.join().expect("join a");
    let reply_b = thread_b.join().expect("join b");
    assert_eq!(
        reply_a, 42,
        "call a must receive its own (non-crossed) reply"
    );
    assert_eq!(
        reply_b, 100,
        "call b must receive its own (non-crossed) reply"
    );

    scheduler.shutdown();
}

#[test]
fn cast_to_absent_pid_is_silently_dropped() {
    #[derive(Clone)]
    struct Noop;
    impl ActorMessage for Noop {
        fn encode(&self, _ctx: &mut NativeContext<'_>) -> Option<Term> {
            Some(Term::small_int(0))
        }
        fn decode(_term: Term) -> Option<Self> {
            Some(Self)
        }
    }
    struct Sink;
    impl Actor for Sink {
        type Call = Noop;
        type Reply = i64;
        type Cast = Noop;
        fn handle_call(&mut self, _r: Noop, _c: &mut ActorContext<'_, '_>) -> i64 {
            0
        }
        fn handle_cast(&mut self, _r: Noop, _c: &mut ActorContext<'_, '_>) {}
    }

    let scheduler = scheduler();
    // A pid that was never spawned: the cast must not error or panic.
    let absent: SenderHandle<Sink> = SenderHandle::attach(&scheduler, 999_999);
    absent
        .cast(Noop)
        .expect("cast to absent pid does not error");
    std::thread::sleep(Duration::from_millis(30));
    scheduler.shutdown();
}

#[test]
fn intra_actor_request_reply_uses_cast_plus_reply() {
    // The gen_server intra-actor pattern: a `Relay` actor receives a cast
    // carrying a reply_to pid and forwards a derived value to that pid as a
    // cast. A `Collector` actor records what it receives so an external call can
    // read it back. No blocking call is used inside any handler.

    #[derive(Clone)]
    struct Ask {
        reply_to: u64,
        value: i64,
    }
    impl ActorMessage for Ask {
        fn encode(&self, ctx: &mut NativeContext<'_>) -> Option<Term> {
            // Carry the pid as an integer scalar so it survives delivery to an
            // Executing receiver (a pid term would decode back as external).
            ctx.alloc_tuple(&[
                Term::try_small_int(i64::try_from(self.reply_to).ok()?)?,
                Term::try_small_int(self.value)?,
            ])
        }
        fn decode(term: Term) -> Option<Self> {
            let tuple = Tuple::new(term)?;
            Some(Self {
                reply_to: u64::try_from(tuple.get(0)?.as_small_int()?).ok()?,
                value: tuple.get(1)?.as_small_int()?,
            })
        }
    }
    #[derive(Clone)]
    struct Scalar(i64);
    impl ActorMessage for Scalar {
        fn encode(&self, _ctx: &mut NativeContext<'_>) -> Option<Term> {
            Term::try_small_int(self.0)
        }
        fn decode(term: Term) -> Option<Self> {
            Some(Self(term.as_small_int()?))
        }
    }

    // Collector: cast(Scalar) stores; call(Scalar) returns last stored value.
    struct Collector {
        last: i64,
    }
    impl Actor for Collector {
        type Call = Scalar;
        type Reply = i64;
        type Cast = Scalar;
        fn handle_call(&mut self, _r: Scalar, _c: &mut ActorContext<'_, '_>) -> i64 {
            self.last
        }
        fn handle_cast(&mut self, r: Scalar, _c: &mut ActorContext<'_, '_>) {
            self.last = r.0;
        }
    }

    // Relay: on a cast, forward value*10 to reply_to via cast (no blocking).
    struct Relay;
    impl Actor for Relay {
        type Call = Scalar;
        type Reply = i64;
        type Cast = Ask;
        fn handle_call(&mut self, _r: Scalar, _c: &mut ActorContext<'_, '_>) -> i64 {
            0
        }
        fn handle_cast(&mut self, r: Ask, ctx: &mut ActorContext<'_, '_>) {
            ctx.cast(r.reply_to, &Scalar(r.value * 10));
        }
    }

    let scheduler = scheduler();
    let collector = spawn_actor(&scheduler, || Collector { last: -1 }).expect("collector");
    let relay = spawn_actor(&scheduler, || Relay).expect("relay");

    relay
        .sender
        .cast(Ask {
            reply_to: collector.pid,
            value: 4,
        })
        .expect("cast ask");

    // The relay's forwarded cast and our read-back call race; poll the
    // (idempotent) read until the forwarded value lands.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut observed = collector.sender.call(Scalar(0)).expect("call collector");
    while observed != 40 {
        assert!(
            std::time::Instant::now() < deadline,
            "the relay's forwarded value never reached the collector"
        );
        std::thread::sleep(Duration::from_millis(2));
        observed = collector.sender.call(Scalar(0)).expect("call collector");
    }
    assert_eq!(
        observed, 40,
        "the relay forwarded value*10 to the collector via cast+reply"
    );

    scheduler.shutdown();
}
