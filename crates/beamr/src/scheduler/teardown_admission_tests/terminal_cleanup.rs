//! Terminal cleanup: admissions contending with it, reserved publishers,
//! and which bodies it visits.

use super::*;

#[test]
fn admission_contending_with_terminal_cleanup_releases_owned_completion() {
    let fixture = TerminalFixture::new();
    let first = fixture.command(1);
    assert!(
        fixture
            .scheduler
            .enqueue_atom_message(fixture.pid, Atom::OK)
    );
    let first_reply = first
        .recv()
        .unwrap_or_else(|error| panic!("executed reply: {error}"));
    let shadow_pid = 501;
    add_executing_process(&fixture.scheduler, shadow_pid);
    let (admission, admitted) = mpsc::channel();
    fixture
        .scheduler
        .shared
        .mailbox_admission_hook
        .set(Box::new(move |pid, point| {
            if pid == shadow_pid {
                admission
                    .send(point)
                    .unwrap_or_else(|error| panic!("admission boundary: {error}"));
            }
        }))
        .unwrap_or_else(|_| panic!("admission hook already installed"));
    let (at_close, release) = terminal_close_gate(&fixture.scheduler);
    let entry = fixture
        .scheduler
        .shared
        .process_bodies
        .get(&shadow_pid)
        .unwrap_or_else(|| panic!("executing slot missing"));
    let slot = lock_or_recover(&entry);
    let sender_scheduler = Arc::clone(&fixture.scheduler);
    let sender = std::thread::spawn(move || {
        sender_scheduler.send_to_mailbox(shadow_pid, OwnedTerm::immediate(Term::small_int(2)))
    });
    let before_slot = admitted
        .recv()
        .unwrap_or_else(|error| panic!("contending sender: {error}"));
    let close_scheduler = Arc::clone(&fixture.scheduler);
    let closer = std::thread::spawn(move || close_scheduler.shutdown());
    at_close
        .recv()
        .unwrap_or_else(|error| panic!("terminal close: {error}"));
    drop(slot);
    drop(entry);
    let queued = admitted
        .recv()
        .unwrap_or_else(|error| panic!("queued completion: {error}"));
    release.wait();
    closer
        .join()
        .unwrap_or_else(|_| panic!("shutdown worker panicked"));
    let retained = fixture
        .scheduler
        .shared
        .process_bodies
        .get(&shadow_pid)
        .map_or(0, |entry| {
            let slot = lock_or_recover(&entry);
            match &*slot {
                ProcessSlot::Executing(metadata) => metadata.pending_io_messages.len(),
                _ => 0,
            }
        });
    // Release a stranded completion before asserting terminal ownership.
    if retained != 0 {
        execution::cleanup_exited_process(
            &fixture.scheduler.shared,
            shadow_pid,
            ExitReason::Normal,
        );
    }
    let abandoned = sender
        .join()
        .unwrap_or_else(|_| panic!("mailbox sender panicked"));
    let last = fixture.command(3);
    let accepted_after = fixture
        .scheduler
        .enqueue_atom_message(fixture.pid, Atom::OK);
    fixture.drop_command(3);
    let last_reply = last.try_recv();
    let executed = fixture.executed.load(Ordering::SeqCst);
    assert_eq!(before_slot, MailboxAdmissionPoint::BeforeSlot);
    assert_eq!(queued, MailboxAdmissionPoint::Queued);
    assert_eq!(first_reply, 1);
    assert_eq!(
        retained, 0,
        "terminal stop retained a scheduler-owned completion"
    );
    assert_eq!(abandoned, Err(MailboxSendError::SchedulerTerminated));
    assert!(!accepted_after);
    assert_eq!(last_reply, Err(mpsc::TryRecvError::Disconnected));
    assert_eq!(executed, 1);
    assert_eq!(executed + 2, 3);
}

fn terminal_close_gate(scheduler: &Scheduler) -> (mpsc::Receiver<()>, Arc<Barrier>) {
    let (closing, at_close) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let close_release = Arc::clone(&release);
    let close_observed = AtomicBool::new(false);
    // Release the contended body lock before joining workers, whose telemetry
    // sampling also needs that lock. The terminal drain still follows the joins.
    scheduler
        .shared
        .shutdown_dispatch_hook
        .set(Box::new(move || {
            if !close_observed.swap(true, Ordering::SeqCst) {
                closing
                    .send(())
                    .unwrap_or_else(|error| panic!("terminal boundary: {error}"));
                close_release.wait();
            }
        }))
        .unwrap_or_else(|_| panic!("terminal hook already installed"));
    (at_close, release)
}

fn reserved_publisher_cleanup(prior_reason: Option<ExitReason>) {
    let scheduler = Arc::new(
        Scheduler::with_services(
            SchedulerConfig {
                thread_count: Some(1),
                ..SchedulerConfig::default()
            },
            SchedulerServices::minimal(),
            Arc::new(ModuleRegistry::new()),
            NativeBifs::none(),
        )
        .unwrap_or_else(|error| panic!("publisher scheduler: {error}")),
    );
    let (module, function) = direct_spawn_module(&scheduler);
    let PublisherGates {
        admitted_rx,
        publisher_release,
        waiting_rx,
        stopped_rx,
    } = publisher_gates(&scheduler);
    let (entered_tx, entered_rx) = mpsc::channel();
    let worker_release = Arc::new(Barrier::new(2));
    let release = Arc::clone(&worker_release);
    let native_pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(BlockingSpawnHandler {
                entered: Some(entered_tx.clone()),
                release: Arc::clone(&release),
            })
        }))
        .unwrap_or_else(|error| panic!("blocking worker: {error}"));
    entered_rx
        .recv()
        .unwrap_or_else(|error| panic!("worker signal: {error}"));
    let next_pid = scheduler.shared.next_pid.load(Ordering::Relaxed);
    let publishing = Arc::clone(&scheduler);
    let publishing_thread =
        std::thread::spawn(move || publishing.spawn(module, function, Vec::new()));
    admitted_rx
        .recv()
        .unwrap_or_else(|error| panic!("admission signal: {error}"));
    let held_next_pid = scheduler.shared.next_pid.load(Ordering::Relaxed);
    let closing = Arc::clone(&scheduler);
    let (done_tx, done_rx) = mpsc::channel();
    let closer = std::thread::spawn(move || {
        closing.shutdown();
        done_tx
            .send(())
            .unwrap_or_else(|error| panic!("joined close: {error}"));
    });
    waiting_rx
        .recv()
        .unwrap_or_else(|error| panic!("waiting signal: {error}"));
    let held = {
        let registry = lock_or_recover(&scheduler.shared.dirty_completions);
        (registry.closed, registry.reserved)
    };
    let premature_close = done_rx.try_recv();
    publisher_release.wait();
    let published = publishing_thread
        .join()
        .unwrap_or_else(|_| panic!("publisher panicked"));
    stopped_rx
        .recv()
        .unwrap_or_else(|error| panic!("stop signal: {error}"));
    let prior_termination = match (prior_reason, &published) {
        (Some(reason), Ok(pid)) => {
            scheduler.terminate_process(*pid, reason);
            Some(())
        }
        _ => None,
    };
    worker_release.wait();
    done_rx
        .recv()
        .unwrap_or_else(|error| panic!("close signal: {error}"));
    closer.join().unwrap_or_else(|_| panic!("closer panicked"));
    let pid = match published {
        Ok(pid) => pid,
        Err(error) => panic!("reserved publication refused: {error}"),
    };
    assert_eq!(held_next_pid, next_pid);
    assert_eq!(held, (true, 1));
    assert_eq!(premature_close, Err(mpsc::TryRecvError::Empty));
    assert_eq!(scheduler.shared.process_table.len(), 0);
    assert_eq!(scheduler.shared.process_bodies.len(), 0);
    assert!(scheduler.inject_queues.iter().all(|queue| queue.is_empty()));
    assert_eq!(prior_termination, prior_reason.map(|_| ()));
    assert_eq!(
        scheduler.peek_exit_reason(pid),
        Some(prior_reason.unwrap_or(ExitReason::Killed))
    );
    assert_eq!(
        scheduler.peek_exit_reason(native_pid),
        Some(ExitReason::Killed)
    );
    assert_terminal_visit_counts(&scheduler);
    scheduler.shutdown();
    assert_terminal_visit_counts(&scheduler);
}

fn assert_terminal_visit_counts(scheduler: &Scheduler) {
    assert_eq!(
        scheduler
            .shared
            .terminal_spawn_visits
            .load(Ordering::Relaxed),
        1
    );
    assert_eq!(
        scheduler
            .shared
            .terminal_body_visits
            .load(Ordering::Relaxed),
        1
    );
}

struct PublisherGates {
    admitted_rx: mpsc::Receiver<()>,
    publisher_release: Arc<Barrier>,
    waiting_rx: mpsc::Receiver<()>,
    stopped_rx: mpsc::Receiver<()>,
}

fn publisher_gates(scheduler: &Scheduler) -> PublisherGates {
    let (admitted_tx, admitted_rx) = mpsc::channel();
    let publisher_release = Arc::new(Barrier::new(2));
    let gate = Arc::clone(&publisher_release);
    assert!(
        scheduler
            .shared
            .spawn_admission_hook
            .set(Box::new(move || {
                admitted_tx
                    .send(())
                    .unwrap_or_else(|error| panic!("publisher admitted: {error}"));
                gate.wait();
            }))
            .is_ok(),
        "spawn hook already set"
    );
    let (waiting_tx, waiting_rx) = mpsc::channel();
    let once = AtomicBool::new(false);
    assert!(
        scheduler
            .shared
            .teardown_wait_hook
            .set(Box::new(move || {
                if !once.swap(true, Ordering::SeqCst) {
                    waiting_tx
                        .send(())
                        .unwrap_or_else(|error| panic!("drain waiting: {error}"));
                }
            }))
            .is_ok(),
        "wait hook already set"
    );
    let (stopped_tx, stopped_rx) = mpsc::channel();
    let once = AtomicBool::new(false);
    assert!(
        scheduler
            .shared
            .shutdown_dispatch_hook
            .set(Box::new(move || {
                if !once.swap(true, Ordering::SeqCst) {
                    stopped_tx
                        .send(())
                        .unwrap_or_else(|error| panic!("dispatch stopped: {error}"));
                }
            }))
            .is_ok(),
        "dispatch hook already set"
    );
    PublisherGates {
        admitted_rx,
        publisher_release,
        waiting_rx,
        stopped_rx,
    }
}

#[test]
fn terminal_cleanup_visits_only_current_owned_bodies() {
    for count in [128_usize, 256] {
        let fixture = TerminalFixture::new();
        for pid in 100_000..100_064 {
            fixture
                .scheduler
                .shared
                .insert_exit_tombstone(pid, ExitReason::Killed);
        }
        for offset in 0..count {
            let pid = 10_000
                + u64::try_from(offset).unwrap_or_else(|error| panic!("fixture offset: {error}"));
            add_executing_process(&fixture.scheduler, pid);
        }
        fixture.scheduler.shutdown();
        let visits = fixture
            .scheduler
            .shared
            .terminal_body_visits
            .load(Ordering::Relaxed);
        let spawn_visits = fixture
            .scheduler
            .shared
            .terminal_spawn_visits
            .load(Ordering::Relaxed);
        let remaining = fixture.scheduler.shared.process_bodies.len();
        fixture.scheduler.shutdown();
        assert_eq!(visits, count + 1);
        assert_eq!(spawn_visits, 0);
        assert_eq!(remaining, 0);
        assert_eq!(fixture.scheduler.shared.process_table.len(), 0);
        assert_eq!(
            fixture
                .scheduler
                .shared
                .terminal_body_visits
                .load(Ordering::Relaxed),
            visits
        );
        assert_eq!(
            fixture
                .scheduler
                .shared
                .terminal_spawn_visits
                .load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            fixture.scheduler.peek_exit_reason(100_000),
            Some(ExitReason::Killed)
        );
    }
}

#[test]
fn candidate_reserved_publisher_is_drained_before_terminal_completion() {
    reserved_publisher_cleanup(None);
}

#[test]
fn candidate_terminal_drain_preserves_prior_queued_spawn_reason() {
    reserved_publisher_cleanup(Some(ExitReason::Error));
}
