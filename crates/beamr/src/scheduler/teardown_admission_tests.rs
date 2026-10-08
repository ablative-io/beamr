use super::process_slot::{ProcessMetadata, ProcessSlot};
use super::*;
use crate::atom::Atom;
use crate::ets::{EtsError, EtsTableMetadata, EtsTableType, Protection};
use crate::module::ModuleRegistry;
use crate::namespace::NamespaceId;
use crate::native::ProcessContext;
use crate::native::group_leader::GroupLeaderError;
use crate::native::supervision::SupervisionError;
use crate::process::heap::DEFAULT_HEAP_SIZE;
use crate::process::{ExitReason, Process};
use crate::term::Term;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, mpsc};
use crate::native::native_process::{NativeContext, NativeHandler, NativeOutcome};
use crate::ets::OwnedTerm;
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
}

impl NativeHandler for TerminalConsumer {
    fn handle(&mut self, context: &mut NativeContext<'_>) -> NativeOutcome {
        while context.recv().is_some() {}
        loop {
            let command = lock_or_recover(&self.commands).pop_front();
            let Some(command) = command else {
                break;
            };
            self.executed.fetch_add(1, Ordering::SeqCst);
            command.reply.send(command.id)
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
}

impl TerminalFixture {
    fn new() -> Self {
        let scheduler = Arc::new(Scheduler::with_services(
            SchedulerConfig {
                thread_count: Some(1),
                ..SchedulerConfig::default()
            },
            SchedulerServices::minimal(),
            Arc::new(ModuleRegistry::new()),
            NativeBifs::none(),
        ).unwrap_or_else(|error| panic!("terminal scheduler: {error}")));
        let (ready, registered) = mpsc::channel();
        let published = AtomicBool::new(false);
        *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |_, gap, pid| {
            if gap == ParkGap::WaitRegistered && !published.swap(true, Ordering::SeqCst) {
                ready.send(pid).unwrap_or_else(|error| panic!("park registration: {error}"));
            }
        }));
        let commands = Arc::new(Mutex::new(VecDeque::new()));
        let executed = Arc::new(AtomicUsize::new(0));
        let factory_commands = Arc::clone(&commands);
        let factory_executed = Arc::clone(&executed);
        let pid = scheduler.spawn_native(Box::new(move || Box::new(TerminalConsumer {
            commands: Arc::clone(&factory_commands),
            executed: Arc::clone(&factory_executed),
        }))).unwrap_or_else(|error| panic!("terminal consumer: {error}"));
        let parked = registered.recv().unwrap_or_else(|error| panic!("ready signal: {error}"));
        *lock_or_recover(&scheduler.shared.park_gap_hook) = None;
        assert_eq!(parked, pid);
        Self { scheduler, pid, commands, executed }
    }

    fn command(&self, id: u64) -> mpsc::Receiver<u64> {
        let (reply, result) = mpsc::channel();
        lock_or_recover(&self.commands).push_back(TerminalCommand { id, reply });
        result
    }

    fn drop_command(&self, id: u64) {
        let command = lock_or_recover(&self.commands).pop_front()
            .unwrap_or_else(|| panic!("queued command missing"));
        assert_eq!(command.id, id);
        drop(command);
    }
}

#[test]
fn joined_shutdown_refuses_atom_admission_with_live_handles() {
    let fixture = TerminalFixture::new();
    fixture.scheduler.shutdown();
    let admitted = fixture.scheduler.enqueue_atom_message(fixture.pid, Atom::OK);
    assert!(!admitted, "joined scheduler admitted an atom into its retained body");
}

#[test]
fn refused_shutdown_command_drops_its_last_embedder_reply_sender() {
    let fixture = TerminalFixture::new();
    fixture.scheduler.shutdown();
    let reply = fixture.command(7);
    let admitted = fixture.scheduler.enqueue_atom_message(fixture.pid, Atom::OK);
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
    assert!(fixture.scheduler.enqueue_atom_message(fixture.pid, Atom::OK));
    let first_reply = first.recv().unwrap_or_else(|error| panic!("executed reply: {error}"));
    let shadow_pid = 501;
    add_executing_process(&fixture.scheduler, shadow_pid);
    let (admission, admitted) = mpsc::channel();
    fixture.scheduler.shared.mailbox_admission_hook.set(Box::new(move |pid, point| {
        if pid == shadow_pid {
            admission.send(point).unwrap_or_else(|error| panic!("admission boundary: {error}"));
        }
    })).unwrap_or_else(|_| panic!("admission hook already installed"));
    let (closing, at_close) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let close_release = Arc::clone(&release);
    let close_observed = AtomicBool::new(false);
    fixture.scheduler.shared.terminal_admission_hook.set(Box::new(move || {
        if !close_observed.swap(true, Ordering::SeqCst) {
            closing.send(()).unwrap_or_else(|error| panic!("terminal boundary: {error}"));
            close_release.wait();
        }
    })).unwrap_or_else(|_| panic!("terminal hook already installed"));
    let entry = fixture.scheduler.shared.process_bodies.get(&shadow_pid)
        .unwrap_or_else(|| panic!("executing slot missing"));
    let slot = lock_or_recover(&entry);
    let sender_scheduler = Arc::clone(&fixture.scheduler);
    let sender = std::thread::spawn(move || {
        sender_scheduler.send_to_mailbox(shadow_pid, OwnedTerm::immediate(Term::small_int(2)))
    });
    let before_slot = admitted.recv().unwrap_or_else(|error| panic!("contending sender: {error}"));
    let close_scheduler = Arc::clone(&fixture.scheduler);
    let closer = std::thread::spawn(move || close_scheduler.shutdown());
    at_close.recv().unwrap_or_else(|error| panic!("terminal close: {error}"));
    drop(slot);
    drop(entry);
    let queued = admitted.recv().unwrap_or_else(|error| panic!("queued completion: {error}"));
    release.wait();
    closer.join().unwrap_or_else(|_| panic!("shutdown worker panicked"));
    let retained = fixture.scheduler.shared.process_bodies.get(&shadow_pid).map_or(0, |entry| {
        let slot = lock_or_recover(&entry);
        match &*slot {
            ProcessSlot::Executing(metadata) => metadata.pending_io_messages.len(),
            _ => 0,
        }
    });
    // Release a stranded completion before asserting terminal ownership.
    execution::cleanup_exited_process(&fixture.scheduler.shared, shadow_pid, ExitReason::Normal);
    let abandoned = sender.join().unwrap_or_else(|_| panic!("mailbox sender panicked"));
    let last = fixture.command(3);
    let accepted_after = fixture.scheduler.enqueue_atom_message(fixture.pid, Atom::OK);
    fixture.drop_command(3);
    let last_reply = last.try_recv();
    let executed = fixture.executed.load(Ordering::SeqCst);
    assert_eq!(before_slot, MailboxAdmissionPoint::BeforeSlot);
    assert_eq!(queued, MailboxAdmissionPoint::Queued);
    assert_eq!(first_reply, 1);
    assert_eq!(retained, 0, "terminal stop retained a scheduler-owned completion");
    assert_eq!(abandoned, Err(MailboxSendError::ProcessTerminated));
    assert!(!accepted_after);
    assert_eq!(last_reply, Err(mpsc::TryRecvError::Disconnected));
    assert_eq!(executed, 1);
    assert_eq!(executed + 2, 3);
}
