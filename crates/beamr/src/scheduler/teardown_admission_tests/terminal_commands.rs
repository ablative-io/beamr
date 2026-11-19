//! Embedder commands at joined shutdown: owned tuples, atoms and reply
//! senders are refused or released.

use super::live_harness::LiveMeasurement;
use super::*;

fn owned_tagged_message(id: i64) -> OwnedTerm {
    let mut heap = crate::process::heap::Heap::new(DEFAULT_HEAP_SIZE);
    let words = heap
        .alloc_slice(3)
        .unwrap_or_else(|error| panic!("tuple storage: {error}"));
    let root = crate::term::boxed::write_tuple(words, &[Term::atom(Atom::OK), Term::small_int(id)])
        .unwrap_or_else(|| panic!("tuple construction"));
    crate::ets::copy_term_to_ets(root).unwrap_or_else(|error| panic!("owned tuple: {error}"))
}

fn retained_mailbox_length(scheduler: &Scheduler, pid: u64) -> usize {
    scheduler
        .shared
        .process_bodies
        .get(&pid)
        .map_or(0, |entry| match &*lock_or_recover(&entry) {
            ProcessSlot::Present(ScheduledProcess(process)) => process.mailbox().message_count(),
            ProcessSlot::Executing(metadata) => metadata.pending_io_messages.len(),
            ProcessSlot::Absent => 0,
        })
}

#[test]
fn joined_shutdown_refuses_owned_tuple_and_closes_reply_sender() {
    let fixture = TerminalFixture::new();
    fixture.scheduler.shutdown();
    let before = retained_mailbox_length(&fixture.scheduler, fixture.pid);
    let (reply, response) = mpsc::channel();
    let message = owned_tagged_message(17).with_reply_witness(reply);
    let measurement = LiveMeasurement::start();
    let result = fixture.scheduler.send_to_mailbox(fixture.pid, message);
    let vector = measurement.finish();
    let after = retained_mailbox_length(&fixture.scheduler, fixture.pid);
    let closed = response.recv();
    assert_eq!(
        result.map_err(|error| error.to_string()),
        Err("scheduler has shut down".to_owned())
    );
    assert_eq!(after, before);
    assert_eq!(vector[5], 0, "refusal copied into the target heap");
    assert_eq!(vector[6], 0, "refusal copied the owned term");
    assert_eq!(vector[8], 0, "refusal woke the target");
    assert_eq!(closed, Err(mpsc::RecvError));
}

#[test]
fn joined_shutdown_refuses_drained_and_never_spawned_pids() {
    let fixture = TerminalFixture::new();
    fixture.scheduler.shutdown();
    for pid in [fixture.pid, u64::MAX] {
        assert_eq!(
            fixture
                .scheduler
                .send_to_mailbox(pid, owned_tagged_message(19)),
            Err(MailboxSendError::SchedulerTerminated)
        );
        assert!(!fixture.scheduler.enqueue_atom_message(pid, Atom::OK));
    }
}

#[test]
fn owned_tuple_reaches_live_parked_process_and_preserves_missing_pid_errors() {
    let fixture = TerminalFixture::new();
    let replied = fixture.command(23);
    let result = fixture
        .scheduler
        .send_to_mailbox(fixture.pid, owned_tagged_message(23));
    let observed = replied
        .recv()
        .unwrap_or_else(|error| panic!("live reply: {error}"));
    let received = lock_or_recover(&fixture.received).clone();
    let unknown = fixture
        .scheduler
        .send_to_mailbox(u64::MAX, owned_tagged_message(24));
    fixture.scheduler.shared.process_table.spawn_with_pid(701);
    fixture.scheduler.shared.process_bodies.insert(
        701,
        Mutex::new(ProcessSlot::Present(ScheduledProcess(Process::new(
            701,
            DEFAULT_HEAP_SIZE,
        )))),
    );
    execution::cleanup_exited_process(&fixture.scheduler.shared, 701, ExitReason::Killed);
    let removed = fixture
        .scheduler
        .send_to_mailbox(701, owned_tagged_message(25));
    fixture.scheduler.shutdown();
    assert_eq!(result, Ok(()));
    assert_eq!(observed, 23);
    assert_eq!(received, [(Term::atom(Atom::OK), Term::small_int(23))]);
    assert_eq!(unknown, Err(MailboxSendError::NoSuchProcess));
    assert_eq!(removed, Err(MailboxSendError::ProcessTerminated));
}

#[test]
fn joined_shutdown_refuses_atom_admission_with_live_handles() {
    let fixture = TerminalFixture::new();
    fixture.scheduler.shutdown();
    let admitted = fixture
        .scheduler
        .enqueue_atom_message(fixture.pid, Atom::OK);
    assert!(
        !admitted,
        "joined scheduler admitted an atom into its retained body"
    );
}

#[test]
fn refused_shutdown_command_drops_its_last_embedder_reply_sender() {
    let fixture = TerminalFixture::new();
    fixture.scheduler.shutdown();
    let reply = fixture.command(7);
    let admitted = fixture
        .scheduler
        .enqueue_atom_message(fixture.pid, Atom::OK);
    if !admitted {
        fixture.drop_command(7);
    }
    let observation = reply.try_recv();
    let executed = fixture.executed.load(Ordering::SeqCst);
    if admitted {
        fixture.drop_command(7);
    }
    assert_eq!(observation, Err(mpsc::TryRecvError::Disconnected));
    assert_eq!(executed, 0);
}
