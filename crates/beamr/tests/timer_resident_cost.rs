//! Isolated-process pin: PENDING TIMERS ADD ZERO RESIDENT COST.
//!
//! Doctrine conversion (pair-routed, 2026-07-12): "zero resident cost added
//! by pending timers" is a permanent negative resource assertion, and those
//! don't get to live as signatures — they get a bound and a test that
//! outlives the signer. The mechanism under pin: the timer wheel is data
//! (`Arc<Mutex<TimerWheel>>`) fired by workers inside their existing loop
//! iteration; `park_thread` parks on the FIXED signed `IDLE_PARK_TIMEOUT`
//! and never derives its wait from the wheel's next expiry.
//!
//! Three walls, each falsifiable (fail-first verified at commit time):
//!  1. THREADS — the process thread multiset is identical before and after
//!     arming a thousand pending entries: no timer thread, ever.
//!  2. PARK TIMEOUT — `observed_park_timeout_millis()` still reads the
//!     signed constant with a full wheel: an implementation that re-derives
//!     the park wait from next-expiry (tickless-done-wrong) fails here
//!     deterministically, with zero sensitivity to host load.
//!  3. WAKE RATE — the aggregate idle wake rate with a thousand pending
//!     entries stays under the SAME signed 2× ceiling the empty-wheel floor
//!     test uses: any per-entry wake mechanism multiplies the rate a
//!     thousandfold and lands far beyond the ceiling.
//!
//! What this pin deliberately does NOT claim: the scheduler's own idle floor
//! (separately signed, `thread_inventory.rs`) and timer FIRING cost — both
//! out of scope; pending is the word doing the work. Expiry granularity
//! rides the same floor (~one `IDLE_PARK_TIMEOUT` of resolution).
//!
//! T1-grade methodology (comment-as-contract):
//!  - sampling source: `idle_park_count()` (one increment per `park_thread`
//!    entry) over wall-clock windows divided by ACTUAL elapsed;
//!    `process_thread_names` for the thread multiset (macOS exact,
//!    count-containment on Linux — `comm` truncates to 15 bytes).
//!  - host state: this quiet isolated integration-test binary.
//!  - bounds are ceilings, never exact matches (lower bounds on wake counts
//!    are load-sensitive; the deterministic seam is wall 2).
//!  - timers armed through PUBLIC paths only (`spawn_native` +
//!    `NativeContext::send_after`) — the first-external-consumer gate.

#![cfg(feature = "threads")]

use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use beamr::module::ModuleRegistry;
use beamr::native::native_process::{NativeContext, NativeHandler, NativeOutcome};
use beamr::scheduler::thread_probe::{process_thread_names, thread_name_multiset};
use beamr::scheduler::{
    IDLE_PARK_TIMEOUT, IDLE_WAKES_PER_SEC_PER_WORKER, Scheduler, SchedulerConfig,
};
use beamr::term::Term;

const PENDING_TIMERS: usize = 1_000;
/// Far enough that nothing fires inside the test; pending is the claim.
const FAR_FUTURE: Duration = Duration::from_secs(3_600);
const SOAK: Duration = Duration::from_millis(400);

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

fn idle_wake_rate(scheduler: &Scheduler) -> f64 {
    let start = scheduler.idle_park_count();
    let started_at = Instant::now();
    thread::sleep(SOAK);
    let parks = scheduler.idle_park_count().saturating_sub(start);
    parks as f64 / started_at.elapsed().as_secs_f64()
}

#[test]
fn a_thousand_pending_timers_add_no_thread_no_wake_and_no_park_shortening() {
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(2),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let workers = scheduler.worker_names().len();
    let ceiling = 2.0 * IDLE_WAKES_PER_SEC_PER_WORKER as f64 * workers as f64;

    // Settle, then take the empty-wheel baseline (positive control: the
    // scheduler is genuinely parking, and at the signed timeout).
    thread::sleep(Duration::from_millis(50));
    let empty_rate = idle_wake_rate(&scheduler);
    assert!(
        empty_rate > 0.0,
        "baseline soak must observe parks, or the pin is vacuous"
    );
    assert_eq!(
        scheduler.observed_park_timeout_millis(),
        Some(IDLE_PARK_TIMEOUT.as_millis() as u64),
        "empty-wheel park timeout must read the signed constant"
    );
    let threads_before = thread_name_multiset(&process_thread_names());

    // Arm the wheel through public paths and hold the entries pending.
    let (armed_sender, armed_receiver) = mpsc::channel();
    let _pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(ArmTimers {
                armed: armed_sender.clone(),
            })
        }))
        .unwrap_or_else(|error| panic!("spawn timer-arming native: {error}"));
    let armed = armed_receiver
        .recv_timeout(Duration::from_secs(10))
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

    // Wall 2: the park wait is still the signed constant — the wheel's
    // next expiry (one hour out) must not leak into the park duration in
    // EITHER direction.
    let full_rate = idle_wake_rate(&scheduler);
    assert_eq!(
        scheduler.observed_park_timeout_millis(),
        Some(IDLE_PARK_TIMEOUT.as_millis() as u64),
        "park timeout must not be re-derived from the wheel's next expiry"
    );

    // Wall 3: the wake rate with a full wheel obeys the SAME signed ceiling
    // as the empty wheel — a per-entry wake mechanism lands ~1000× beyond it.
    assert!(
        full_rate <= ceiling,
        "idle wake rate {full_rate:.1}/s with {PENDING_TIMERS} pending \
         timers exceeds the signed ceiling {ceiling:.1}/s ({workers} \
         workers at the {}ms floor) — pending entries are adding wakes",
        IDLE_PARK_TIMEOUT.as_millis(),
    );

    scheduler.shutdown();
}
