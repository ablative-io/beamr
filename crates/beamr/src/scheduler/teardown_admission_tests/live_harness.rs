//! The live-operation harness: counting the operations a message send
//! performs into each kind of target, and the native process it sends to.

use super::*;

pub(super) const LIVE_COMPONENTS: [&str; 11] = [
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

pub(super) struct LiveMeasurement {
    counts: &'static std::thread::LocalKey<std::cell::Cell<Option<[usize; 11]>>>,
}

impl LiveMeasurement {
    pub(super) fn start() -> Self {
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

    pub(super) fn finish(self) -> [usize; 11] {
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
pub(in crate::scheduler) enum LiveTargetMode {
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

pub(in crate::scheduler) fn count_live_messages_with_setup<T>(
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
