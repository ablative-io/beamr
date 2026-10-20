//! Isolated-process pin: PENDING TIMERS ADD ZERO RESIDENT COST.
//!
//! Doctrine conversion (pair-routed, 2026-07-12): "zero resident cost added
//! by pending timers" is a permanent negative resource assertion, and those
//! don't get to live as signatures — they get a bound and a test that
//! outlives the signer. The mechanism under pin, since B-181 (tickless idle,
//! EMBEDDER-COMPOSITION-SPEC §3.8 amendment): the timer wheel is data
//! (`Arc<Mutex<TimerWheel>>`) fired by worker 0; an idle worker 0 parks
//! until the wheel's EARLIEST deadline and every other worker parks with no
//! timeout. The 5 ms tick this pin used to read is retired.
//!
//! Three walls, each falsifiable:
//!  1. THREADS — the process thread multiset is identical before and after
//!     arming a thousand pending entries: no timer thread, ever.
//!  2. PARK TIMEOUT — with a thousand entries pending an hour out, worker 0
//!     parks until that hour and every other worker parks unbounded: a
//!     per-entry wake, a tick, or a shortened park reads here
//!     deterministically, with zero sensitivity to host load.
//!  3. WAKE RATE — wall 2 is the wake rate: a park with no timeout ends
//!     only on a wake edge, and worker 0's ends at the hour, so a thousand
//!     pending entries add zero wakes before they are due (the signed
//!     bound: one wake per deadline worker 0 reaches).
//!
//! What this pin deliberately does NOT claim: the scheduler's own idle floor
//! (`thread_inventory.rs`) and timer FIRING cost — both out of scope;
//! pending is the word doing the work.
//!
//! Methodology (comment-as-contract):
//!  - sampling source: the park seams (`worker_park_counts`,
//!    `await_worker_parked`), which record the timeout each park actually
//!    waited with; `process_thread_names` for the thread multiset (macOS
//!    exact, count-containment on Linux — `comm` truncates to 15 bytes).
//!  - every wait ends on an event (the arming reply, a worker starting to
//!    wait), never on a clock.
//!  - timers armed through PUBLIC paths only (`spawn_native` +
//!    `NativeContext::send_after`) — the first-external-consumer gate.

#![cfg(feature = "threads")]

use std::sync::{Arc, mpsc};
use std::time::Duration;

use beamr::module::ModuleRegistry;
use beamr::native::native_process::{NativeContext, NativeHandler, NativeOutcome};
use beamr::scheduler::thread_probe::{process_thread_names, thread_name_multiset};
use beamr::scheduler::{NativeBifs, ObservedParkTimeout, Scheduler, SchedulerConfig};
use beamr::term::Term;

const PENDING_TIMERS: usize = 1_000;
/// Far enough that nothing fires inside the test; pending is the claim.
const FAR_FUTURE: Duration = Duration::from_secs(3_600);

struct ArmTimers {
    armed: mpsc::Sender<usize>,
}

impl NativeHandler for ArmTimers {
    fn handle(&mut self, context: &mut NativeContext<'_>) -> NativeOutcome {
        let pid = context.self_pid();
        let mut armed = 0;
        for _ in 0..PENDING_TIMERS {
            if context
                .send_after(FAR_FUTURE, pid, Term::atom(beamr::atom::Atom::OK))
                .is_some()
            {
                armed += 1;
            }
        }
        let _sent = self.armed.send(armed);
        // Stay alive so the entries stay pending for the whole soak.
        NativeOutcome::Wait
    }
}

#[test]
fn a_thousand_pending_timers_add_no_thread_no_wake_and_no_park_shortening() {
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(2),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let workers = scheduler.worker_names().len();
    let threads_before = thread_name_multiset(&process_thread_names());
    let before: Vec<_> = (0..workers)
        .map(|worker| scheduler.worker_park_counts(worker))
        .collect();

    // Arm the wheel through public paths and hold the entries pending. The
    // arming native runs on worker 0 and then waits, so worker 0's next park
    // is the first with the full wheel.
    let (armed_sender, armed_receiver) = mpsc::channel();
    let _pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(ArmTimers {
                armed: armed_sender.clone(),
            })
        }))
        .unwrap_or_else(|error| panic!("spawn timer-arming native: {error}"));
    let armed = armed_receiver
        .recv()
        .unwrap_or_else(|error| panic!("timer arming did not report: {error}"));
    assert_eq!(
        armed, PENDING_TIMERS,
        "every entry must actually be pending"
    );

    // Wall 1: no thread appeared. Exact multiset on macOS; Linux gets the
    // count-level form (comm truncation, same posture as thread_inventory).
    let threads_after = thread_name_multiset(&process_thread_names());
    #[cfg(target_os = "macos")]
    assert_eq!(
        threads_after, threads_before,
        "a pending timer must never be a thread"
    );
    #[cfg(not(target_os = "macos"))]
    assert!(
        threads_after.values().sum::<usize>() <= threads_before.values().sum::<usize>(),
        "a pending timer must never be a thread"
    );

    // Walls 2 and 3: worker 0 parks until the earliest entry (an hour out),
    // never on a tick; every other worker parks with no timeout. A park with
    // no timeout ends only on a wake edge, so the pending entries add no
    // wake until the first is due.
    match scheduler.await_worker_parked(0, before[0]) {
        ObservedParkTimeout::Bounded(timeout) => assert!(
            timeout > FAR_FUTURE - Duration::from_secs(60),
            "worker 0 must park until the earliest pending entry, not on a \
             tick: parked for {timeout:?}"
        ),
        other => panic!("worker 0 must park until the earliest entry: {other:?}"),
    }
    for (worker, since) in before.iter().enumerate().skip(1) {
        assert_eq!(
            scheduler.await_worker_parked(worker, *since),
            ObservedParkTimeout::Unbounded,
            "worker {worker} must park with no timeout while timers are pending"
        );
    }

    scheduler.shutdown();
}
