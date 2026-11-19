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

mod direct_spawns;
mod live_harness;
mod live_messages;
mod riders;
mod terminal_cleanup;
mod terminal_commands;

pub(crate) use live_harness::record_live_operation;
pub(super) use live_harness::{LiveTargetMode, count_live_messages_with_setup};

fn add_executing_process(scheduler: &Scheduler, pid: u64) {
    scheduler.shared.process_table.spawn_with_pid(pid);
    let process = Process::new(pid, DEFAULT_HEAP_SIZE);
    scheduler.shared.process_bodies.insert(
        pid,
        Mutex::new(ProcessSlot::Executing(ProcessMetadata {
            namespace_id: NamespaceId::DEFAULT,
            capabilities: process.capabilities().clone(),
            relations: crate::process::SharedRelations::default(),
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
