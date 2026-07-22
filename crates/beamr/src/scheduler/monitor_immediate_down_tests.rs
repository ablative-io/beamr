//! Regression tests for monitors registered after the target has exited.

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::atom::Atom;
use crate::module::ModuleRegistry;
use crate::native::native_process::{NativeContext, NativeHandler, NativeOutcome};
use crate::native::supervision::SupervisionFacility;
use crate::process::{ExitReason, ProcessStatus};
use crate::term::Term;
use crate::term::boxed::{self, Tuple};

use super::execution::{cleanup_exited_process, store_runnable_process};
use super::supervision_integration::SchedulerSupervisionFacility;
use super::supervision_tests::{
    insert_process, make_executing, make_shared_state, read_mailbox_tuple,
};
use super::{ProcessSlot, ScheduledProcess, Scheduler, SchedulerConfig, lock_or_recover};

const CHANNEL_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn tombstoned_target_monitor_while_watcher_executing_survives_store_back() {
    let shared = make_shared_state();
    let watcher = insert_process(&shared, 101);
    let target = insert_process(&shared, 102);
    cleanup_exited_process(&shared, target, ExitReason::Error);
    let checked_out_watcher = make_executing(&shared, watcher);

    let facility = SchedulerSupervisionFacility {
        shared: shared.clone(),
    };
    let result = SupervisionFacility::monitor(&facility, watcher, target)
        .unwrap_or_else(|error| panic!("monitor tombstoned target: {error}"));

    {
        let entry = shared
            .process_bodies
            .get(&watcher)
            .unwrap_or_else(|| panic!("executing watcher body exists"));
        let slot = lock_or_recover(&entry);
        let ProcessSlot::Executing(metadata) = &*slot else {
            panic!("watcher remains executing while monitor is registered");
        };
        assert_eq!(
            metadata.pending_down_messages,
            vec![(result.reference, target, ExitReason::Error)],
            "immediate DOWN is admitted to executing-slot metadata"
        );
    }

    // No explicit wake is needed in the Executing arm: store-back merges the
    // pending DOWN before the Wait arm registers, and its final mailbox recheck
    // self-wakes the watcher. This direct store-back pins the merge half of that
    // contract without any timer or unrelated traffic.
    store_runnable_process(&shared, checked_out_watcher);

    let message = read_mailbox_tuple(&shared, watcher)
        .unwrap_or_else(|| panic!("pending DOWN survives watcher store-back"));
    assert_eq!(message.len(), 5);
    assert_eq!(message[0], Term::atom(Atom::DOWN));
    let reference = boxed::Reference::new(message[1])
        .unwrap_or_else(|| panic!("DOWN contains a monitor reference"));
    assert_eq!(reference.id(), result.reference);
    // Strengthened re-pin: the DOWN reference is term-equal (not merely
    // id-equal) to a canonical boxed reference of the monitor's id — the same
    // term rank monitor/2 now returns, on which the OTP {'DOWN', Ref, ...}
    // selective receive depends.
    let mut expected_ref = [0u64; 2];
    assert_eq!(
        message[1],
        boxed::write_reference(&mut expected_ref, result.reference)
            .unwrap_or_else(|| panic!("canonical DOWN reference fits"))
    );
    assert_eq!(message[2], Term::atom(Atom::PROCESS));
    assert_eq!(message[3].as_pid(), Some(target));
    assert_eq!(message[4], Term::atom(Atom::ERROR));
    assert!(result.immediate_down, "admitted DOWN is reported honestly");
}

#[test]
fn tombstoned_target_monitor_wakes_present_parked_watcher() {
    let shared = make_shared_state();
    let watcher = insert_process(&shared, 201);
    let target = insert_process(&shared, 202);
    cleanup_exited_process(&shared, target, ExitReason::Error);

    let scheduler_index = 3;
    lock_or_recover(&shared.wait_set)
        .waiting
        .insert(watcher, scheduler_index);

    let facility = SchedulerSupervisionFacility {
        shared: Arc::clone(&shared),
    };
    let result = SupervisionFacility::monitor(&facility, watcher, target)
        .unwrap_or_else(|error| panic!("monitor tombstoned target: {error}"));

    let message = read_mailbox_tuple(&shared, watcher)
        .unwrap_or_else(|| panic!("parked watcher receives immediate DOWN"));
    let reference = boxed::Reference::new(message[1])
        .unwrap_or_else(|| panic!("DOWN contains a monitor reference"));
    assert_eq!(reference.id(), result.reference);
    // Strengthened re-pin: term-equality (not merely id-equality) against a
    // canonical boxed reference of the monitor's id.
    let mut expected_ref = [0u64; 2];
    assert_eq!(
        message[1],
        boxed::write_reference(&mut expected_ref, result.reference)
            .unwrap_or_else(|| panic!("canonical DOWN reference fits"))
    );
    assert!(result.immediate_down);

    let wait_set = lock_or_recover(&shared.wait_set);
    assert!(
        !wait_set.waiting.contains_key(&watcher),
        "wake removes the watcher from the parked set"
    );
    assert_eq!(
        wait_set.woken,
        vec![(watcher, scheduler_index)],
        "DOWN alone wakes the parked watcher"
    );
}

#[test]
fn tombstoned_target_monitor_is_not_immediate_when_watcher_cannot_accept_down() {
    let shared = make_shared_state();
    let target = insert_process(&shared, 301);
    cleanup_exited_process(&shared, target, ExitReason::Error);

    let absent_watcher = insert_process(&shared, 302);
    {
        let entry = shared
            .process_bodies
            .get(&absent_watcher)
            .unwrap_or_else(|| panic!("absent watcher retains a slot entry"));
        *lock_or_recover(&entry) = ProcessSlot::Absent;
    }

    let exited_watcher = insert_process(&shared, 303);
    {
        let entry = shared
            .process_bodies
            .get(&exited_watcher)
            .unwrap_or_else(|| panic!("exited watcher body exists"));
        let mut slot = lock_or_recover(&entry);
        let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
            panic!("exited watcher starts present");
        };
        process
            .transition_to(ProcessStatus::Exited(ExitReason::Normal))
            .unwrap_or_else(|error| panic!("watcher exits: {error}"));
    }

    let facility = SchedulerSupervisionFacility {
        shared: Arc::clone(&shared),
    };
    let absent_result = SupervisionFacility::monitor(&facility, absent_watcher, target)
        .unwrap_or_else(|error| panic!("monitor still returns a reference: {error}"));
    let exited_result = SupervisionFacility::monitor(&facility, exited_watcher, target)
        .unwrap_or_else(|error| panic!("monitor still returns a reference: {error}"));

    assert_ne!(absent_result.reference, exited_result.reference);
    assert!(
        !absent_result.immediate_down,
        "an Absent slot did not admit DOWN"
    );
    assert!(
        !exited_result.immediate_down,
        "an exited watcher did not admit DOWN"
    );
    assert!(
        read_mailbox_tuple(&shared, exited_watcher).is_none(),
        "exited watcher mailbox remains untouched"
    );
}

struct StopImmediately;

impl NativeHandler for StopImmediately {
    fn handle(&mut self, _context: &mut NativeContext<'_>) -> NativeOutcome {
        NativeOutcome::Stop(ExitReason::Error)
    }
}

struct WorkerBlocker {
    entered: Option<mpsc::Sender<()>>,
    release: Arc<Mutex<mpsc::Receiver<()>>>,
}

impl NativeHandler for WorkerBlocker {
    fn handle(&mut self, _context: &mut NativeContext<'_>) -> NativeOutcome {
        if let Some(entered) = self.entered.take() {
            entered
                .send(())
                .unwrap_or_else(|error| panic!("publish occupied worker: {error}"));
        }
        // A disconnected channel also releases the worker if the test panics.
        let _released_or_disconnected = lock_or_recover(&self.release).recv();
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

struct DownObserver {
    observed: mpsc::Sender<(u64, u64, Atom)>,
}

impl NativeHandler for DownObserver {
    fn handle(&mut self, context: &mut NativeContext<'_>) -> NativeOutcome {
        let message = context
            .recv()
            .unwrap_or_else(|| panic!("queued watcher starts with immediate DOWN"));
        let tuple = Tuple::new(message).unwrap_or_else(|| panic!("DOWN is a tuple"));
        assert_eq!(tuple.arity(), 5);
        assert_eq!(tuple.get(0), Some(Term::atom(Atom::DOWN)));
        assert_eq!(tuple.get(2), Some(Term::atom(Atom::PROCESS)));
        let reference = boxed::Reference::new(
            tuple
                .get(1)
                .unwrap_or_else(|| panic!("DOWN reference element")),
        )
        .unwrap_or_else(|| panic!("DOWN contains a monitor reference"));
        let target = tuple
            .get(3)
            .and_then(Term::as_pid)
            .unwrap_or_else(|| panic!("DOWN target pid"));
        let reason = tuple
            .get(4)
            .and_then(Term::as_atom)
            .unwrap_or_else(|| panic!("DOWN reason atom"));
        self.observed
            .send((reference.id(), target, reason))
            .unwrap_or_else(|error| panic!("publish observed DOWN: {error}"));
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

#[test]
fn monitor_with_result_reports_immediate_down_for_present_queued_watcher() {
    let config = SchedulerConfig {
        thread_count: Some(1),
        dirty_cpu_threads: Some(1),
        dirty_io_threads: Some(1),
        dirty_queue_depth: Some(8),
        ..SchedulerConfig::default()
    };
    let scheduler = Arc::new(
        Scheduler::new(config, Arc::new(ModuleRegistry::new()))
            .unwrap_or_else(|error| panic!("scheduler starts: {error}")),
    );

    let target = scheduler
        .spawn_native(Box::new(|| Box::new(StopImmediately)))
        .unwrap_or_else(|error| panic!("spawn target: {error}"));
    assert_eq!(scheduler.run_until_exit(target).0, ExitReason::Error);

    // Occupy the only worker so the watcher is provably Present and queued,
    // rather than racing into its first Executing slot before monitor returns.
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = Arc::new(Mutex::new(release_rx));
    let factory_release = Arc::clone(&release);
    let blocker = scheduler
        .spawn_native(Box::new(move || {
            Box::new(WorkerBlocker {
                entered: Some(entered_tx.clone()),
                release: Arc::clone(&factory_release),
            })
        }))
        .unwrap_or_else(|error| panic!("spawn worker blocker: {error}"));
    entered_rx
        .recv_timeout(CHANNEL_TIMEOUT)
        .unwrap_or_else(|error| panic!("worker was not occupied: {error}"));

    let (observed_tx, observed_rx) = mpsc::channel();
    let watcher = scheduler
        .spawn_native(Box::new(move || {
            Box::new(DownObserver {
                observed: observed_tx.clone(),
            })
        }))
        .unwrap_or_else(|error| panic!("spawn watcher: {error}"));
    let watcher_is_present = scheduler
        .shared
        .process_bodies
        .get(&watcher)
        .is_some_and(|entry| matches!(&*lock_or_recover(&entry), ProcessSlot::Present(_)));

    let result = scheduler
        .monitor_with_result(watcher, target)
        .unwrap_or_else(|error| panic!("monitor tombstoned target: {error}"));
    release_tx
        .send(())
        .unwrap_or_else(|error| panic!("release worker: {error}"));

    assert!(watcher_is_present, "watcher was queued in a Present slot");
    assert!(result.immediate_down, "public API exposes immediate DOWN");
    assert_eq!(
        observed_rx
            .recv_timeout(CHANNEL_TIMEOUT)
            .unwrap_or_else(|error| panic!("watcher did not observe DOWN: {error}")),
        (result.reference, target, Atom::ERROR)
    );
    assert_eq!(scheduler.run_until_exit(watcher).0, ExitReason::Normal);
    assert_eq!(scheduler.run_until_exit(blocker).0, ExitReason::Normal);
    scheduler.shutdown();
}
