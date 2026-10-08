use super::process_slot::{ProcessMetadata, ProcessSlot};
use super::*;
use crate::atom::Atom;
use crate::ets::OwnedTerm;
use crate::ets::{EtsError, EtsTableMetadata, EtsTableType, Protection};
use crate::module::ModuleRegistry;
use crate::namespace::NamespaceId;
use crate::native::ProcessContext;
use crate::native::group_leader::GroupLeaderError;
use crate::native::native_process::{NativeContext, NativeHandler, NativeOutcome};
use crate::native::supervision::SupervisionError;
use crate::process::heap::DEFAULT_HEAP_SIZE;
use crate::process::{ExitReason, Process};
use crate::term::Term;
use std::collections::VecDeque;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::time::Duration;

fn add_executing_process(scheduler: &Scheduler, pid: u64) {
    scheduler.shared.process_table.spawn_with_pid(pid);
    let process = Process::new(pid, DEFAULT_HEAP_SIZE);
    scheduler.shared.process_bodies.insert(
        pid,
        Mutex::new(ProcessSlot::Executing(ProcessMetadata {
            namespace_id: NamespaceId::DEFAULT,
            capabilities: process.capabilities().clone(),
            links: Vec::new(),
            remote_links: Vec::new(),
            monitors: Vec::new(),
            trap_exit: false,
            priority: process.priority(),
            current_mfa: None,
            heap_size: 0,
            binary_heap_size: 0,
            message_queue_len: 0,
            group_leader: process.group_leader(),
            logical_clock: process.logical_clock(),
            pending_exit_messages: Vec::new(),
            pending_down_messages: Vec::new(),
            pending_io_messages: Vec::new(),
            pending_distribution_payloads: Vec::new(),
            pending_local_messages: Vec::new(),
            pending_ets_transfer_messages: Vec::new(),
            pending_udp_messages: Vec::new(),
            pending_tcp_messages: Vec::new(),
        })),
    );
}

#[test]
fn all_five_commit6_riders_admit_before_drain_and_refuse_after() {
    let scheduler = Scheduler::with_services(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        SchedulerServices::minimal().owned_readiness(),
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("rider scheduler starts: {error}"));
    for pid in [101, 102, 103] {
        add_executing_process(&scheduler, pid);
    }
    let services =
        supervision_integration::build_native_services(&scheduler.shared, NamespaceId::DEFAULT);
    let group = services
        .group_leader_facility
        .as_ref()
        .unwrap_or_else(|| panic!("group-leader facility"));
    let supervision = services
        .supervision_facility
        .as_ref()
        .unwrap_or_else(|| panic!("supervision facility"));
    let ets = services
        .ets_facility
        .as_ref()
        .unwrap_or_else(|| panic!("ETS facility"));

    // Each mutation succeeds while admission is open.
    group
        .set_group_leader(101, Term::pid(101))
        .unwrap_or_else(|error| panic!("pre-drain group leader: {error}"));
    let table = ets
        .create_table(EtsTableMetadata::new(
            None,
            0,
            EtsTableType::Set,
            Protection::Protected,
            101,
        ))
        .unwrap_or_else(|error| panic!("pre-drain ETS create: {error}"));
    let mut timer_context =
        ProcessContext::with_timer_services(101, Arc::clone(&scheduler.shared.timers));
    timer_context.set_teardown_admission_facility(services.teardown_admission_facility.clone());
    assert!(
        timer_context
            .schedule_timer(Duration::from_secs(60), 101, Term::atom(Atom::new(450)))
            .is_some()
    );
    let (reader, _writer) =
        UnixStream::pair().unwrap_or_else(|error| panic!("socket pair: {error}"));
    let token = scheduler
        .shared
        .readiness_register(reader.as_raw_fd(), Interest::READABLE, 101, Atom::new(451))
        .unwrap_or_else(|error| panic!("pre-drain readiness register: {error}"));
    supervision
        .exit_signal(101, 102, ExitReason::Kill)
        .unwrap_or_else(|error| panic!("pre-drain exit signal: {error}"));

    scheduler.shared.drain_dirty_completions();

    // Every named row now refuses through its existing typed surface.
    assert_eq!(
        group.set_group_leader(101, Term::pid(101)),
        Err(GroupLeaderError::NoProc)
    );
    assert_eq!(
        supervision.exit_signal(101, 103, ExitReason::Kill),
        Err(SupervisionError::NoProc)
    );
    assert_eq!(
        ets.create_table(EtsTableMetadata::new(
            None,
            0,
            EtsTableType::Set,
            Protection::Protected,
            101,
        )),
        Err(EtsError::Badarg)
    );
    assert!(!ets.delete_table(table));
    assert!(
        timer_context
            .schedule_timer(Duration::from_secs(60), 101, Term::atom(Atom::new(452)))
            .is_none()
    );
    let (straggler, _peer) =
        UnixStream::pair().unwrap_or_else(|error| panic!("straggler pair: {error}"));
    assert_eq!(
        scheduler.shared.readiness_register(
            straggler.as_raw_fd(),
            Interest::READABLE,
            101,
            Atom::new(453),
        ),
        Err(ReadinessError::TeardownInProgress)
    );

    scheduler.readiness_deregister(token);
    scheduler.shutdown();
}

struct TerminalCommand {
    id: u64,
    reply: mpsc::Sender<u64>,
}

struct TerminalConsumer {
    commands: Arc<Mutex<VecDeque<TerminalCommand>>>,
    executed: Arc<AtomicUsize>,
    received: Arc<Mutex<Vec<(Term, Term)>>>,
}

impl NativeHandler for TerminalConsumer {
    fn handle(&mut self, context: &mut NativeContext<'_>) -> NativeOutcome {
        while let Some(message) = context.recv() {
            if let Some(tuple) = crate::term::boxed::Tuple::new(message) {
                let tag = tuple
                    .get(0)
                    .unwrap_or_else(|| panic!("message tag missing"));
                let id = tuple.get(1).unwrap_or_else(|| panic!("message id missing"));
                lock_or_recover(&self.received).push((tag, id));
            }
        }
        loop {
            let command = lock_or_recover(&self.commands).pop_front();
            let Some(command) = command else {
                break;
            };
            self.executed.fetch_add(1, Ordering::SeqCst);
            command
                .reply
                .send(command.id)
                .unwrap_or_else(|error| panic!("command reply: {error}"));
        }
        NativeOutcome::Wait
    }
}

struct TerminalFixture {
    scheduler: Arc<Scheduler>,
    pid: u64,
    commands: Arc<Mutex<VecDeque<TerminalCommand>>>,
    executed: Arc<AtomicUsize>,
    received: Arc<Mutex<Vec<(Term, Term)>>>,
}

impl TerminalFixture {
    fn new() -> Self {
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
            .unwrap_or_else(|error| panic!("terminal scheduler: {error}")),
        );
        let (ready, registered) = mpsc::channel();
        let published = AtomicBool::new(false);
        *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |_, gap, pid| {
            if gap == ParkGap::WaitRegistered && !published.swap(true, Ordering::SeqCst) {
                ready
                    .send(pid)
                    .unwrap_or_else(|error| panic!("park registration: {error}"));
            }
        }));
        let commands = Arc::new(Mutex::new(VecDeque::new()));
        let executed = Arc::new(AtomicUsize::new(0));
        let received = Arc::new(Mutex::new(Vec::new()));
        let factory_received = Arc::clone(&received);
        let factory_commands = Arc::clone(&commands);
        let factory_executed = Arc::clone(&executed);
        let pid = scheduler
            .spawn_native(Box::new(move || {
                Box::new(TerminalConsumer {
                    commands: Arc::clone(&factory_commands),
                    executed: Arc::clone(&factory_executed),
                    received: Arc::clone(&factory_received),
                })
            }))
            .unwrap_or_else(|error| panic!("terminal consumer: {error}"));
        let parked = registered
            .recv()
            .unwrap_or_else(|error| panic!("ready signal: {error}"));
        *lock_or_recover(&scheduler.shared.park_gap_hook) = None;
        assert_eq!(parked, pid);
        Self {
            scheduler,
            pid,
            commands,
            executed,
            received,
        }
    }

    fn command(&self, id: u64) -> mpsc::Receiver<u64> {
        let (reply, result) = mpsc::channel();
        lock_or_recover(&self.commands).push_back(TerminalCommand { id, reply });
        result
    }

    fn drop_command(&self, id: u64) {
        let command = lock_or_recover(&self.commands)
            .pop_front()
            .unwrap_or_else(|| panic!("queued command missing"));
        assert_eq!(command.id, id);
        drop(command);
    }
}

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
    scheduler
        .shared
        .terminal_admission_hook
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

fn direct_spawn_module(scheduler: &Scheduler) -> (Atom, Atom) {
    let name = scheduler.shared.atom_table.intern("terminal_spawn");
    let function = scheduler.shared.atom_table.intern("entry");
    let module = crate::module::Module {
        name,
        generation: 0,
        origin: crate::module::ModuleOrigin::Preloaded,
        exports: std::collections::HashMap::from([((function, 0), 1)]),
        label_index: std::collections::HashMap::from([(1, 0)]),
        code: vec![
            crate::loader::Instruction::Label { label: 1 },
            crate::loader::Instruction::Return,
        ],
        literals: Vec::new(),
        constant_pool: crate::constant_pool::ConstantPool::default(),
        resolved_imports: Vec::new(),
        lambdas: Vec::new(),
        string_table: Vec::new(),
        function_table: Vec::new(),
        line_table: Vec::new(),
        line_info: Vec::new(),
    };
    drop(scheduler.shared.module_registry.insert(module));
    (name, function)
}

#[test]
fn joined_shutdown_refuses_direct_spawns_without_allocating_identity() {
    let fixture = TerminalFixture::new();
    let (module, function) = direct_spawn_module(&fixture.scheduler);
    fixture.scheduler.shutdown();
    let shared = &fixture.scheduler.shared;
    let before = (
        shared.next_pid.load(Ordering::Relaxed),
        shared.process_table.len(),
        shared.process_bodies.len(),
        shared.exit_tombstones.len(),
        fixture
            .scheduler
            .inject_queues
            .iter()
            .map(|queue| queue.len())
            .sum::<usize>(),
    );
    let results = [
        fixture.scheduler.spawn(module, function, Vec::new()),
        fixture
            .scheduler
            .spawn_trap_exit(module, function, Vec::new()),
    ];
    let after = (
        shared.next_pid.load(Ordering::Relaxed),
        shared.process_table.len(),
        shared.process_bodies.len(),
        shared.exit_tombstones.len(),
        fixture
            .scheduler
            .inject_queues
            .iter()
            .map(|queue| queue.len())
            .sum::<usize>(),
    );
    // Release baseline-admitted ownership before evaluating the recorded red.
    for queue in &fixture.scheduler.inject_queues {
        while let Some(request) = queue.pop() {
            drop(request);
        }
    }
    for pid in results.iter().flatten() {
        execution::cleanup_exited_process(shared, *pid, ExitReason::Killed);
    }
    for result in &results {
        assert!(result.is_err(), "joined scheduler admitted a direct spawn");
        assert_eq!(
            result.as_ref().err().map(ToString::to_string),
            Some("scheduler spawn admission is closed".to_owned()),
        );
    }
    assert_eq!(
        after, before,
        "refusal allocated process identity or terminal state"
    );
}

struct BlockingSpawnHandler {
    entered: Option<mpsc::Sender<()>>,
    release: Arc<Barrier>,
}

impl NativeHandler for BlockingSpawnHandler {
    fn handle(&mut self, context: &mut NativeContext<'_>) -> NativeOutcome {
        while context.recv().is_some() {}
        if let Some(entered) = self.entered.take() {
            entered
                .send(())
                .unwrap_or_else(|error| panic!("worker entered: {error}"));
            self.release.wait();
        }
        NativeOutcome::Wait
    }
}

#[test]
fn shutdown_joins_native_slice_before_recording_or_removing_its_body() {
    let fixture = TerminalFixture::new();
    let (entered, inside) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let worker_release = Arc::clone(&release);
    let pid = fixture
        .scheduler
        .spawn_native(Box::new(move || {
            Box::new(BlockingSpawnHandler {
                entered: Some(entered.clone()),
                release: Arc::clone(&worker_release),
            })
        }))
        .unwrap_or_else(|error| panic!("held native spawn: {error}"));
    inside
        .recv()
        .unwrap_or_else(|error| panic!("native slice entry: {error}"));
    let (dispatch, closing) = mpsc::channel();
    let dispatch_signalled = AtomicBool::new(false);
    fixture
        .scheduler
        .shared
        .shutdown_dispatch_hook
        .set(Box::new(move || {
            if !dispatch_signalled.swap(true, Ordering::SeqCst) {
                dispatch
                    .send(())
                    .unwrap_or_else(|error| panic!("shutdown dispatch: {error}"));
            }
        }))
        .unwrap_or_else(|_| panic!("shutdown dispatch hook already installed"));
    let (stored, store_back) = mpsc::channel();
    let shared = Arc::downgrade(&fixture.scheduler.shared);
    let joined_signalled = AtomicBool::new(false);
    fixture
        .scheduler
        .shared
        .terminal_admission_hook
        .set(Box::new(move || {
            if joined_signalled.swap(true, Ordering::SeqCst) {
                return;
            }
            let shared = shared
                .upgrade()
                .unwrap_or_else(|| panic!("scheduler owner missing"));
            let present = shared
                .process_bodies
                .get(&pid)
                .is_some_and(|entry| matches!(&*lock_or_recover(&entry), ProcessSlot::Present(_)));
            stored
                .send((
                    present,
                    shared.exit_tombstones.contains_key(&pid),
                    shared.process_table.get(pid).is_some(),
                ))
                .unwrap_or_else(|error| panic!("joined store-back: {error}"));
        }))
        .unwrap_or_else(|_| panic!("terminal admission hook already installed"));
    let scheduler = Arc::clone(&fixture.scheduler);
    let closer = std::thread::spawn(move || scheduler.shutdown());
    closing
        .recv()
        .unwrap_or_else(|error| panic!("shutdown admission closed: {error}"));
    let before_release = (
        fixture.scheduler.shared.exit_tombstones.contains_key(&pid),
        fixture.scheduler.shared.process_table.get(pid).is_some(),
    );
    release.wait();
    let joined = store_back
        .recv()
        .unwrap_or_else(|error| panic!("joined body observation: {error}"));
    closer
        .join()
        .unwrap_or_else(|_| panic!("shutdown panicked"));
    assert_eq!(before_release, (false, true));
    assert_eq!(joined, (true, false, true));
    assert!(fixture.scheduler.shared.process_table.get(pid).is_none());
    assert!(fixture.scheduler.shared.exit_tombstones.contains_key(&pid));
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
    let prior_termination = if let Some(reason) = prior_reason {
        if let Ok(pid) = &published {
            scheduler.terminate_process(*pid, reason);
            Some(())
        } else {
            None
        }
    } else {
        None
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

const LIVE_COMPONENTS: [&str; 11] = [
    "map_table_probes",
    "slot_locks",
    "wait_set_locks",
    "lifecycle_admission_atomic_reads",
    "allocator_calls",
    "heap_allocations",
    "deep_term_copies",
    "mailbox_or_pending_pushes",
    "wake_calls",
    "notifications",
    "heap_or_mailbox_clones",
];

std::thread_local! {
    static LIVE_COUNTS: std::cell::Cell<Option<[usize; 11]>> = const {
        std::cell::Cell::new(None)
    };
}

pub(crate) fn record_live_operation(component: usize) {
    LIVE_COUNTS.with(|counts| {
        if let Some(mut vector) = counts.get() {
            vector[component] += 1;
            counts.set(Some(vector));
        }
    });
}

struct LiveAllocator;

#[global_allocator]
static LIVE_ALLOCATOR: LiveAllocator = LiveAllocator;

// SAFETY: allocation and deallocation delegate to the same system allocator.
unsafe impl std::alloc::GlobalAlloc for LiveAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        record_live_operation(4);
        // SAFETY: the caller supplies the allocator contract's valid layout.
        unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        record_live_operation(4);
        // SAFETY: the caller supplies the allocator contract's valid layout.
        unsafe { std::alloc::GlobalAlloc::alloc_zeroed(&std::alloc::System, layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        // SAFETY: the allocation originated in this same system allocator.
        unsafe { std::alloc::GlobalAlloc::dealloc(&std::alloc::System, pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        record_live_operation(4);
        // SAFETY: the original allocation and replacement size satisfy the caller's contract.
        unsafe { std::alloc::GlobalAlloc::realloc(&std::alloc::System, pointer, layout, size) }
    }
}

struct LiveMeasurement {
    counts: &'static std::thread::LocalKey<std::cell::Cell<Option<[usize; 11]>>>,
}

impl LiveMeasurement {
    fn start() -> Self {
        LIVE_COUNTS.with(|counts| {
            assert!(
                counts.replace(Some([0; 11])).is_none(),
                "nested measurement"
            );
        });
        Self {
            counts: &LIVE_COUNTS,
        }
    }

    fn finish(self) -> [usize; 11] {
        self.counts
            .with(std::cell::Cell::take)
            .unwrap_or_else(|| panic!("measurement not armed"))
    }
}

impl Drop for LiveMeasurement {
    fn drop(&mut self) {
        self.counts.with(|counts| counts.set(None));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LiveTargetMode {
    Present,
    Executing,
    ResultGated,
}

impl LiveTargetMode {
    fn baseline_vector(self, count: usize) -> [usize; 11] {
        match (self, count) {
            (Self::Present, 128) => [512, 128, 128, 0, 6, 0, 0, 128, 128, 1, 0],
            (Self::Present, 256) => [1024, 256, 256, 0, 7, 0, 0, 256, 256, 1, 0],
            (Self::Executing, 128) => [512, 128, 128, 0, 6, 0, 0, 128, 128, 0, 0],
            (Self::Executing, 256) => [1024, 256, 256, 0, 7, 0, 0, 256, 256, 0, 0],
            (Self::ResultGated, 128) => [896, 128, 0, 0, 6, 0, 0, 128, 128, 0, 0],
            (Self::ResultGated, 256) => [1792, 256, 0, 0, 7, 0, 0, 256, 256, 0, 0],
            _ => panic!("unsupported message-count fixture"),
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Executing => "executing",
            Self::ResultGated => "result_gated",
        }
    }
}

struct LiveNativeState {
    mode: LiveTargetMode,
    received: usize,
    ready: mpsc::Sender<(u64, Option<u64>)>,
    release: Arc<Barrier>,
}

static LIVE_NATIVE_STATE: Mutex<Option<LiveNativeState>> = Mutex::new(None);

fn live_message_entry(args: &[Term], context: &mut ProcessContext<'_>) -> Result<Term, Term> {
    if !args.is_empty() {
        return Err(Term::atom(Atom::BADARG));
    }
    let pid = context
        .pid()
        .unwrap_or_else(|| panic!("native process pid missing"));
    let (mode, ready, release) = {
        let state = lock_or_recover(&LIVE_NATIVE_STATE);
        let state = state
            .as_ref()
            .unwrap_or_else(|| panic!("native fixture missing"));
        (state.mode, state.ready.clone(), Arc::clone(&state.release))
    };
    match mode {
        LiveTargetMode::Executing => {
            ready
                .send((pid, None))
                .unwrap_or_else(|error| panic!("executing signal: {error}"));
            release.wait();
        }
        LiveTargetMode::ResultGated => {
            assert!(
                context.request_await_suspend(None).is_some(),
                "host await identity missing"
            );
        }
        LiveTargetMode::Present => {}
    }
    Ok(Term::NIL)
}

fn live_message_received(args: &[Term], context: &mut ProcessContext<'_>) -> Result<Term, Term> {
    if args != [Term::atom(Atom::OK)] || context.pid().is_none() {
        return Err(Term::atom(Atom::BADARG));
    }
    let mut state = lock_or_recover(&LIVE_NATIVE_STATE);
    let state = state
        .as_mut()
        .unwrap_or_else(|| panic!("native fixture missing"));
    state.received += 1;
    Ok(Term::NIL)
}

fn live_message_module(scheduler: &Scheduler) -> (Atom, Atom) {
    use crate::loader::Instruction;
    use crate::loader::decode::compact::Operand;
    use crate::module::{Module, ModuleOrigin, ResolvedImport, ResolvedImportTarget};
    use crate::native::{Capability, NativeEntry};
    let name = scheduler.shared.atom_table.intern("live_message_counts");
    let function = scheduler.shared.atom_table.intern("entry");
    let code = vec![
        Instruction::Label { label: 1 },
        Instruction::CallExt {
            arity: Operand::Unsigned(0),
            import: Operand::Unsigned(0),
        },
        Instruction::Label { label: 2 },
        Instruction::LoopRec {
            fail: Operand::Label(3),
            destination: Operand::X(0),
        },
        Instruction::RemoveMessage,
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(1),
        },
        Instruction::Jump {
            target: Operand::Label(2),
        },
        Instruction::Label { label: 3 },
        Instruction::Wait {
            fail: Operand::Label(2),
        },
    ];
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, index)),
            _ => None,
        })
        .collect();
    let resolved_imports = [
        (0, live_message_entry as crate::native::NativeFn),
        (1, live_message_received as crate::native::NativeFn),
    ]
    .into_iter()
    .map(|(arity, native)| ResolvedImport {
        module: name,
        function,
        arity,
        target: ResolvedImportTarget::Native(NativeEntry {
            function: native,
            dirty_kind: None,
            capability: Capability::Pure,
        }),
    })
    .collect();
    drop(scheduler.shared.module_registry.insert(Module {
        name,
        generation: 0,
        origin: ModuleOrigin::Preloaded,
        exports: std::collections::HashMap::from([((function, 0), 1)]),
        label_index,
        code,
        literals: Vec::new(),
        constant_pool: crate::constant_pool::ConstantPool::default(),
        resolved_imports,
        lambdas: Vec::new(),
        string_table: Vec::new(),
        function_table: Vec::new(),
        line_table: Vec::new(),
        line_info: Vec::new(),
    }));
    (name, function)
}

fn count_live_messages(mode: LiveTargetMode, count: usize) -> [usize; 11] {
    count_live_messages_with_setup(mode, count, |_| ()).0
}

struct LiveMessageTarget {
    scheduler: Arc<Scheduler>,
    pid: u64,
    call_id: Option<u64>,
    release: Arc<Barrier>,
    received_all: mpsc::Sender<usize>,
    received: mpsc::Receiver<usize>,
}

fn live_message_target(mode: LiveTargetMode) -> LiveMessageTarget {
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
        .unwrap_or_else(|error| panic!("live scheduler starts: {error}")),
    );
    let (ready, registered) = mpsc::channel();
    let (received_all, received) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    *lock_or_recover(&LIVE_NATIVE_STATE) = Some(LiveNativeState {
        mode,
        received: 0,
        ready: ready.clone(),
        release: Arc::clone(&release),
    });
    if mode != LiveTargetMode::Executing {
        let signalled = AtomicBool::new(false);
        *lock_or_recover(&scheduler.shared.park_gap_hook) =
            Some(Box::new(move |shared, gap, pid| {
                if gap == ParkGap::WaitRegistered && !signalled.swap(true, Ordering::SeqCst) {
                    let call_id = shared.suspensions.get(&pid).map(|mirror| mirror.call_id);
                    ready
                        .send((pid, call_id))
                        .unwrap_or_else(|error| panic!("parked signal: {error}"));
                }
            }));
    }
    let (module, function) = live_message_module(&scheduler);
    let pid = scheduler
        .spawn(module, function, Vec::new())
        .unwrap_or_else(|error| panic!("live process spawn: {error}"));
    let (ready_pid, call_id) = registered
        .recv()
        .unwrap_or_else(|error| panic!("live ready signal: {error}"));
    assert_eq!(ready_pid, pid);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = None;
    if mode == LiveTargetMode::Present {
        let (entered, blocked) = mpsc::channel();
        let worker_release = Arc::clone(&release);
        scheduler
            .spawn_native(Box::new(move || {
                Box::new(BlockingSpawnHandler {
                    entered: Some(entered.clone()),
                    release: Arc::clone(&worker_release),
                })
            }))
            .unwrap_or_else(|error| panic!("blocking process spawn: {error}"));
        blocked
            .recv()
            .unwrap_or_else(|error| panic!("worker blocked signal: {error}"));
    }
    {
        let entry = scheduler
            .shared
            .process_bodies
            .get(&pid)
            .unwrap_or_else(|| panic!("live process slot missing"));
        let slot = lock_or_recover(&entry);
        match (&*slot, mode) {
            (ProcessSlot::Executing(_), LiveTargetMode::Executing)
            | (ProcessSlot::Present(_), LiveTargetMode::Present | LiveTargetMode::ResultGated) => {}
            _ => panic!("target not at measured slot boundary"),
        }
    }
    if mode == LiveTargetMode::ResultGated {
        assert!(call_id.is_some());
        assert!(scheduler.shared.suspension_blocks_wake(pid));
    } else {
        assert!(call_id.is_none());
    }
    LiveMessageTarget {
        scheduler,
        pid,
        call_id,
        release,
        received_all,
        received,
    }
}

pub(super) fn count_live_messages_with_setup<T>(
    mode: LiveTargetMode,
    count: usize,
    setup: impl FnOnce(&Scheduler) -> T,
) -> ([usize; 11], T) {
    let LiveMessageTarget {
        scheduler,
        pid,
        call_id,
        release,
        received_all,
        received,
    } = live_message_target(mode);
    let observer_guard = setup(&scheduler);
    let measurement = LiveMeasurement::start();
    let mut accepted = 0;
    for _ in 0..count {
        accepted += usize::from(scheduler.enqueue_atom_message(pid, Atom::OK));
    }
    let vector = measurement.finish();
    let before_release = received.try_recv();
    let finished = AtomicBool::new(false);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |_, gap, target| {
        if target != pid || gap != ParkGap::WaitRegistered {
            return;
        }
        let observed = lock_or_recover(&LIVE_NATIVE_STATE)
            .as_ref()
            .unwrap_or_else(|| panic!("native fixture missing"))
            .received;
        println!(
            "B179_RECEIVE_PARK mode={} n={} observed={}",
            mode.label(),
            count,
            observed
        );
        if observed >= count && !finished.swap(true, Ordering::SeqCst) {
            received_all
                .send(observed)
                .unwrap_or_else(|error| panic!("receive park signal: {error}"));
        }
    }));
    let resumed = if mode == LiveTargetMode::ResultGated {
        scheduler.wake_with_result_for(
            pid,
            call_id.unwrap_or_else(|| panic!("await identity missing")),
            Term::NIL,
        )
    } else {
        if mode == LiveTargetMode::Present {
            execution::wake_process(&scheduler.shared, pid);
        }
        release.wait();
        true
    };
    let delivered = if resumed {
        received
            .recv()
            .unwrap_or_else(|error| panic!("receive park signal: {error}"))
    } else {
        0
    };
    *lock_or_recover(&scheduler.shared.park_gap_hook) = None;
    scheduler.shutdown();
    let received_count = lock_or_recover(&LIVE_NATIVE_STATE)
        .take()
        .unwrap_or_else(|| panic!("native fixture missing"))
        .received;
    assert_eq!(before_release, Err(mpsc::TryRecvError::Empty));
    assert!(resumed, "owned await completion refused");
    assert_eq!(accepted, count, "live admission refused");
    assert_eq!(delivered, count);
    assert_eq!(received_count, count);
    for (component, (actual, ceiling)) in vector
        .into_iter()
        .zip(mode.baseline_vector(count))
        .enumerate()
    {
        assert!(
            actual <= ceiling,
            "{} increased: {actual} > {ceiling}",
            LIVE_COMPONENTS[component]
        );
    }
    println!(
        "B179_ADMISSION_VECTOR mode={} n={count} delivered={delivered} vector={vector:?}",
        mode.label()
    );
    (vector, observer_guard)
}

#[test]
fn sender_admission_count_vectors_for_present_executing_and_result_gated_targets() {
    println!("B179_COMPONENTS {:?}", LIVE_COMPONENTS);
    for mode in [
        LiveTargetMode::Present,
        LiveTargetMode::Executing,
        LiveTargetMode::ResultGated,
    ] {
        for count in [128, 256] {
            let vector = count_live_messages(mode, count);
            assert_eq!(vector[7], count);
            assert_eq!(vector[8], count);
        }
    }
}

#[test]
fn live_message_allocation_and_clone_observers_have_positive_controls() {
    let mut heap = crate::process::heap::Heap::new(32);
    let mut mailbox = crate::mailbox::Mailbox::new();
    let measurement = LiveMeasurement::start();
    let allocation = Box::new([0_u64; 1024]);
    std::hint::black_box(&allocation);
    assert!(heap.alloc_slice(2).is_ok());
    mailbox.push_owned(Term::atom(Atom::OK));
    drop(std::hint::black_box(heap.clone()));
    drop(std::hint::black_box(mailbox.clone()));
    drop(allocation);
    let vector = measurement.finish();
    assert!(vector[4] > 0);
    assert_eq!(vector[5], 1);
    assert_eq!(vector[7], 1);
    assert_eq!(vector[10], 2);
}
