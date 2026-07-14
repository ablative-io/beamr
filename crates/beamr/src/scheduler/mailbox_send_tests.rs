use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::time::Duration;

use crate::atom::Atom;
use crate::ets::{OwnedTerm, copy_term_to_ets};
use crate::module::ModuleRegistry;
use crate::native::native_process::{NativeContext, NativeHandler, NativeOutcome};
use crate::process::ExitReason;
use crate::process::heap::Heap;
use crate::term::Term;
use crate::term::boxed::{Tuple, write_tuple};

use super::{MailboxSendError, Scheduler, SchedulerConfig};

const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Eq, PartialEq)]
enum Observed {
    Atom(Atom),
    Integer(i64),
    Tagged(Atom, i64),
}

fn observe(term: Term) -> Observed {
    if let Some(atom) = term.as_atom() {
        return Observed::Atom(atom);
    }
    if let Some(integer) = term.as_small_int() {
        return Observed::Integer(integer);
    }
    let tuple =
        Tuple::new(term).unwrap_or_else(|| panic!("expected observable term, got {term:?}"));
    assert_eq!(tuple.arity(), 2, "tagged command tuple arity");
    Observed::Tagged(
        tuple
            .get(0)
            .and_then(Term::as_atom)
            .unwrap_or_else(|| panic!("tagged command atom")),
        tuple
            .get(1)
            .and_then(Term::as_small_int)
            .unwrap_or_else(|| panic!("tagged command payload")),
    )
}

fn owned_immediate(term: Term) -> OwnedTerm {
    OwnedTerm::immediate(term)
}

fn owned_tagged(tag: Atom, payload: i64) -> OwnedTerm {
    let mut heap = Heap::new(3);
    let words = heap
        .alloc_slice(3)
        .unwrap_or_else(|error| panic!("tuple heap allocation: {error}"));
    let tuple = write_tuple(words, &[Term::atom(tag), Term::small_int(payload)])
        .unwrap_or_else(|| panic!("tagged tuple construction"));
    copy_term_to_ets(tuple).unwrap_or_else(|error| panic!("own tagged tuple: {error}"))
}

fn scheduler() -> Arc<Scheduler> {
    Arc::new(
        Scheduler::new(SchedulerConfig::default(), Arc::new(ModuleRegistry::new()))
            .unwrap_or_else(|error| panic!("scheduler starts: {error}")),
    )
}

struct Recorder {
    expected: usize,
    received: Vec<Observed>,
    ready: mpsc::Sender<()>,
    observed: mpsc::Sender<Vec<Observed>>,
    invocations: Arc<AtomicUsize>,
}

impl NativeHandler for Recorder {
    fn handle(&mut self, context: &mut NativeContext<'_>) -> NativeOutcome {
        let invocation = self.invocations.fetch_add(1, Ordering::SeqCst);
        if invocation == 0 {
            self.ready
                .send(())
                .unwrap_or_else(|error| panic!("publish ready: {error}"));
        }
        while let Some(message) = context.recv() {
            self.received.push(observe(message));
        }
        if self.received.len() == self.expected {
            self.observed
                .send(self.received.clone())
                .unwrap_or_else(|error| panic!("publish observations: {error}"));
            NativeOutcome::Stop(ExitReason::Normal)
        } else {
            NativeOutcome::Wait
        }
    }
}

fn spawn_recorder(
    scheduler: &Scheduler,
    expected: usize,
) -> (
    u64,
    mpsc::Receiver<()>,
    mpsc::Receiver<Vec<Observed>>,
    Arc<AtomicUsize>,
) {
    let (ready_tx, ready_rx) = mpsc::channel();
    let (observed_tx, observed_rx) = mpsc::channel();
    let invocations = Arc::new(AtomicUsize::new(0));
    let factory_invocations = Arc::clone(&invocations);
    let pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(Recorder {
                expected,
                received: Vec::new(),
                ready: ready_tx.clone(),
                observed: observed_tx.clone(),
                invocations: Arc::clone(&factory_invocations),
            })
        }))
        .unwrap_or_else(|error| panic!("spawn recorder: {error}"));
    (pid, ready_rx, observed_rx, invocations)
}

fn await_ready(ready: &mpsc::Receiver<()>) {
    ready
        .recv_timeout(WAIT_TIMEOUT)
        .unwrap_or_else(|error| panic!("receiver did not park: {error}"));
}

fn await_observed(observed: &mpsc::Receiver<Vec<Observed>>) -> Vec<Observed> {
    observed
        .recv_timeout(WAIT_TIMEOUT)
        .unwrap_or_else(|error| panic!("receiver did not publish mailbox content: {error}"))
}

#[test]
fn typed_tagged_tuple_round_trips_by_content() {
    let scheduler = scheduler();
    let command = scheduler.atom_table().intern("r_b_1_host_command");
    let (pid, ready, observed, _) = spawn_recorder(&scheduler, 1);
    await_ready(&ready);

    scheduler
        .send_to_mailbox(pid, owned_tagged(command, 41))
        .unwrap_or_else(|error| panic!("typed send succeeds: {error}"));

    assert_eq!(
        await_observed(&observed),
        vec![Observed::Tagged(command, 41)]
    );
    assert_eq!(scheduler.run_until_exit(pid).0, ExitReason::Normal);
    scheduler.shutdown();
}

#[test]
fn waiting_process_is_woken_exactly_once() {
    let scheduler = scheduler();
    let marker = scheduler.atom_table().intern("one_wake");
    let (pid, ready, observed, invocations) = spawn_recorder(&scheduler, 1);
    await_ready(&ready);

    scheduler
        .send_to_mailbox(pid, owned_immediate(Term::atom(marker)))
        .unwrap_or_else(|error| panic!("typed send succeeds: {error}"));

    assert_eq!(await_observed(&observed), vec![Observed::Atom(marker)]);
    assert_eq!(scheduler.run_until_exit(pid).0, ExitReason::Normal);
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        2,
        "initial park plus exactly one message-driven wake"
    );
    scheduler.shutdown();
}

struct LongSliceReceiver {
    entered: mpsc::Sender<()>,
    release: Arc<Barrier>,
    observed: mpsc::Sender<Observed>,
    invocation: usize,
}

impl NativeHandler for LongSliceReceiver {
    fn handle(&mut self, context: &mut NativeContext<'_>) -> NativeOutcome {
        self.invocation += 1;
        if self.invocation == 1 {
            self.entered
                .send(())
                .unwrap_or_else(|error| panic!("publish executing state: {error}"));
            self.release.wait();
            return NativeOutcome::Wait;
        }
        let message = context
            .recv()
            .unwrap_or_else(|| panic!("next receive observes executing-slot delivery"));
        self.observed
            .send(observe(message))
            .unwrap_or_else(|error| panic!("publish executing receipt: {error}"));
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

#[test]
fn delivery_during_long_running_slice_lands_on_next_receive() {
    let scheduler = scheduler();
    let marker = scheduler.atom_table().intern("during_execution");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (observed_tx, observed_rx) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let factory_release = Arc::clone(&release);
    let pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(LongSliceReceiver {
                entered: entered_tx.clone(),
                release: Arc::clone(&factory_release),
                observed: observed_tx.clone(),
                invocation: 0,
            })
        }))
        .unwrap_or_else(|error| panic!("spawn long-slice receiver: {error}"));
    entered_rx
        .recv_timeout(WAIT_TIMEOUT)
        .unwrap_or_else(|error| panic!("receiver did not enter slice: {error}"));

    let send_scheduler = Arc::clone(&scheduler);
    let (send_started_tx, send_started_rx) = mpsc::channel();
    let sender = std::thread::spawn(move || {
        send_started_tx
            .send(())
            .unwrap_or_else(|error| panic!("publish send start: {error}"));
        send_scheduler.send_to_mailbox(pid, owned_immediate(Term::atom(marker)))
    });
    send_started_rx
        .recv_timeout(WAIT_TIMEOUT)
        .unwrap_or_else(|error| panic!("sender did not start: {error}"));
    release.wait();

    sender
        .join()
        .unwrap_or_else(|_| panic!("sender thread panicked"))
        .unwrap_or_else(|error| panic!("executing-slot send succeeds: {error}"));
    assert_eq!(
        observed_rx
            .recv_timeout(WAIT_TIMEOUT)
            .unwrap_or_else(|error| panic!("next receive did not complete: {error}")),
        Observed::Atom(marker)
    );
    assert_eq!(scheduler.run_until_exit(pid).0, ExitReason::Normal);
    scheduler.shutdown();
}

#[test]
fn dead_pid_returns_typed_failure() {
    let scheduler = scheduler();
    let pid = scheduler
        .spawn_native(Box::new(|| {
            struct Stop;
            impl NativeHandler for Stop {
                fn handle(&mut self, _: &mut NativeContext<'_>) -> NativeOutcome {
                    NativeOutcome::Stop(ExitReason::Normal)
                }
            }
            Box::new(Stop)
        }))
        .unwrap_or_else(|error| panic!("spawn stopping process: {error}"));
    assert_eq!(scheduler.run_until_exit(pid).0, ExitReason::Normal);

    assert_eq!(
        scheduler.send_to_mailbox(pid, owned_immediate(Term::NIL)),
        Err(MailboxSendError::ProcessTerminated)
    );
    assert_eq!(
        scheduler.send_to_mailbox(u64::MAX, owned_immediate(Term::NIL)),
        Err(MailboxSendError::NoSuchProcess)
    );
    scheduler.shutdown();
}

#[test]
fn fifo_interleaving_with_atom_send_is_preserved() {
    let scheduler = scheduler();
    let first = scheduler.atom_table().intern("first_atom");
    let tag = scheduler.atom_table().intern("typed_middle");
    let last = scheduler.atom_table().intern("last_atom");
    let (pid, ready, observed, _) = spawn_recorder(&scheduler, 3);
    await_ready(&ready);

    assert!(scheduler.enqueue_atom_message(pid, first));
    scheduler
        .send_to_mailbox(pid, owned_tagged(tag, 2))
        .unwrap_or_else(|error| panic!("typed middle send succeeds: {error}"));
    assert!(scheduler.enqueue_atom_message(pid, last));

    assert_eq!(
        await_observed(&observed),
        vec![
            Observed::Atom(first),
            Observed::Tagged(tag, 2),
            Observed::Atom(last),
        ]
    );
    assert_eq!(scheduler.run_until_exit(pid).0, ExitReason::Normal);
    scheduler.shutdown();
}

#[test]
fn concurrent_senders_all_succeed_without_content_loss() {
    const SENDERS: usize = 16;
    let scheduler = scheduler();
    let (pid, ready, observed, _) = spawn_recorder(&scheduler, SENDERS);
    await_ready(&ready);
    let start = Arc::new(Barrier::new(SENDERS + 1));

    let mut threads = Vec::new();
    for value in 0..SENDERS {
        let thread_scheduler = Arc::clone(&scheduler);
        let thread_start = Arc::clone(&start);
        threads.push(std::thread::spawn(move || {
            thread_start.wait();
            thread_scheduler.send_to_mailbox(
                pid,
                owned_immediate(Term::small_int(
                    i64::try_from(value).expect("sender id fits"),
                )),
            )
        }));
    }
    start.wait();
    for thread in threads {
        thread
            .join()
            .unwrap_or_else(|_| panic!("sender thread panicked"))
            .unwrap_or_else(|error| panic!("concurrent send succeeds: {error}"));
    }

    let mut values = await_observed(&observed)
        .into_iter()
        .map(|observation| match observation {
            Observed::Integer(value) => value,
            other => panic!("expected sender integer, got {other:?}"),
        })
        .collect::<Vec<_>>();
    values.sort_unstable();
    assert_eq!(
        values,
        (0..SENDERS)
            .map(|value| i64::try_from(value).expect("sender id fits"))
            .collect::<Vec<_>>()
    );
    assert_eq!(scheduler.run_until_exit(pid).0, ExitReason::Normal);
    scheduler.shutdown();
}
