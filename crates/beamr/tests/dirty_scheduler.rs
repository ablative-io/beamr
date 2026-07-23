use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use beamr::atom::Atom;
use beamr::error::ExecError;
use beamr::loader::Instruction;
use beamr::loader::decode::compact::Operand;
use beamr::module::{Module, ModuleOrigin, ModuleRegistry, ResolvedImport, ResolvedImportTarget};
use beamr::native::{Capability, NativeEntry, ProcessContext};
use beamr::process::ExitReason;
use beamr::scheduler::dirty::DirtySchedulerKind;
use beamr::scheduler::{Scheduler, SchedulerConfig};
use beamr::term::Term;

static NORMAL_PROGRESS: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
struct DirtyLifecycleState {
    generation: u64,
    started: bool,
    finished: bool,
}

struct DirtyLifecycle {
    state: Mutex<DirtyLifecycleState>,
    condvar: Condvar,
}

static DIRTY_LIFECYCLE: OnceLock<DirtyLifecycle> = OnceLock::new();

fn dirty_lifecycle() -> &'static DirtyLifecycle {
    DIRTY_LIFECYCLE.get_or_init(|| DirtyLifecycle {
        state: Mutex::new(DirtyLifecycleState::default()),
        condvar: Condvar::new(),
    })
}

fn reset_dirty_lifecycle() -> u64 {
    let lifecycle = dirty_lifecycle();
    let mut state = lifecycle.state.lock().expect("dirty lifecycle lock");
    state.generation = state.generation.saturating_add(1);
    state.started = false;
    state.finished = false;
    state.generation
}

fn signal_dirty_started() {
    let lifecycle = dirty_lifecycle();
    let mut state = lifecycle.state.lock().expect("dirty lifecycle lock");
    state.started = true;
    lifecycle.condvar.notify_all();
}

fn signal_dirty_finished() {
    let lifecycle = dirty_lifecycle();
    let mut state = lifecycle.state.lock().expect("dirty lifecycle lock");
    state.finished = true;
    lifecycle.condvar.notify_all();
}

fn wait_for_dirty_started(generation: u64) {
    let lifecycle = dirty_lifecycle();
    let mut state = lifecycle.state.lock().expect("dirty lifecycle lock");
    while state.generation == generation && !state.started {
        state = lifecycle.condvar.wait(state).expect("dirty lifecycle wait");
    }
    assert_eq!(state.generation, generation);
    assert!(state.started);
}

fn dirty_finished_for_generation(generation: u64) -> bool {
    let lifecycle = dirty_lifecycle();
    let state = lifecycle.state.lock().expect("dirty lifecycle lock");
    state.generation == generation && state.finished
}

fn module(name: Atom, code: Vec<Instruction>) -> Module {
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            _ => None,
        })
        .collect();
    Module {
        name,
        generation: 0,
        origin: ModuleOrigin::Preloaded,
        exports: HashMap::new(),
        label_index,
        code,
        literals: Vec::new(),
        constant_pool: Default::default(),
        resolved_imports: Vec::new(),
        lambdas: Vec::new(),
        string_table: Vec::new(),
        function_table: Vec::new(),
        line_table: Vec::new(),
        line_info: Vec::new(),
    }
}

fn dirty_sleep_value(_args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    signal_dirty_started();
    std::thread::sleep(Duration::from_millis(200));
    signal_dirty_finished();
    Ok(Term::small_int(42))
}

fn dirty_badarg(_args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    Err(Term::atom(Atom::BADARG))
}

fn normal_progress(_args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    NORMAL_PROGRESS.fetch_add(1, Ordering::AcqRel);
    Ok(Term::small_int(7))
}

fn native_import(
    function: beamr::native::NativeFn,
    dirty_kind: Option<DirtySchedulerKind>,
) -> ResolvedImport {
    ResolvedImport {
        module: Atom::OK,
        function: Atom::OK,
        arity: 0,
        target: ResolvedImportTarget::Native(NativeEntry {
            function,
            dirty_kind,
            capability: Capability::Pure,
        }),
    }
}

fn call_native_module(name: Atom, import: ResolvedImport) -> Module {
    let mut m = module(
        name,
        vec![
            Instruction::CallExt {
                arity: Operand::Unsigned(0),
                import: Operand::Unsigned(0),
            },
            Instruction::Return,
        ],
    );
    m.resolved_imports.push(import);
    m
}

/// Transient completion threads spawned to date, read from the public policy
/// lines (§5) — the same surface an embedder would use.
fn completion_spawned_total(scheduler: &Scheduler) -> u64 {
    scheduler
        .service_policies()
        .into_iter()
        .map(|line| line.spawned_total)
        .sum()
}

#[test]
fn dirty_nif_round_trip_does_not_block_normal_scheduler() {
    let generation = reset_dirty_lifecycle();
    NORMAL_PROGRESS.store(0, Ordering::Release);

    let registry = Arc::new(ModuleRegistry::new());
    let dirty_module = registry.insert(call_native_module(
        Atom::OK,
        native_import(dirty_sleep_value, Some(DirtySchedulerKind::Cpu)),
    ));
    let normal_module = registry.insert(call_native_module(
        Atom::ERROR,
        native_import(normal_progress, None),
    ));

    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            dirty_cpu_threads: Some(1),
            dirty_io_threads: Some(1),
            dirty_queue_depth: Some(8),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .expect("scheduler starts");

    // Positive control for the refusal-ordering instruments: a SUCCESSFUL
    // dirty call must move all three counters, or the disabled-pool gates'
    // unchanged-counter assertions could pass vacuously on a deleted or
    // mis-gated increment.
    let allocations_before = scheduler.dirty_suspension_allocation_count();
    let mirrors_before = scheduler.suspension_mirror_registration_count();
    let spawns_before = completion_spawned_total(&scheduler);

    let dirty_pid = scheduler.spawn_process(&dirty_module);
    wait_for_dirty_started(generation);
    assert!(!dirty_finished_for_generation(generation));

    let normal_pid = scheduler.spawn_process(&normal_module);
    let (normal_reason, normal_result) = scheduler.run_until_exit(normal_pid);
    assert_eq!(normal_reason, ExitReason::Normal);
    assert_eq!(normal_result.root(), Term::small_int(7));
    assert_eq!(NORMAL_PROGRESS.load(Ordering::Acquire), 1);
    assert!(!dirty_finished_for_generation(generation));

    let (dirty_reason, dirty_result) = scheduler.run_until_exit(dirty_pid);
    assert_eq!(dirty_reason, ExitReason::Normal);
    assert_eq!(dirty_result.root(), Term::small_int(42));
    assert!(dirty_finished_for_generation(generation));

    // One dirty call = exactly one call-id allocation, one suspension
    // mirror, and one completion thread: the instruments the disabled-pool
    // gates hold at zero really do move when the path runs.
    assert_eq!(
        scheduler.dirty_suspension_allocation_count(),
        allocations_before + 1,
        "one successful dirty call allocates exactly one suspension call id"
    );
    assert_eq!(
        scheduler.suspension_mirror_registration_count(),
        mirrors_before + 1,
        "one successful dirty call registers exactly one suspension mirror"
    );
    assert_eq!(
        completion_spawned_total(&scheduler),
        spawns_before + 1,
        "one successful dirty call spawns exactly one completion thread"
    );

    scheduler.shutdown();
}

#[test]
fn dirty_nif_error_resumes_and_raises_exception() {
    let registry = Arc::new(ModuleRegistry::new());
    let dirty_module = registry.insert(call_native_module(
        Atom::OK,
        native_import(dirty_badarg, Some(DirtySchedulerKind::Cpu)),
    ));

    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            dirty_cpu_threads: Some(1),
            dirty_io_threads: Some(1),
            dirty_queue_depth: Some(8),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .expect("scheduler starts");

    let pid = scheduler.spawn_process(&dirty_module);
    let (reason, _result) = scheduler.run_until_exit(pid);
    assert_eq!(reason, ExitReason::Error);
    let exception = scheduler
        .take_exit_exception(pid)
        .expect("dirty native error captured exception");
    assert_eq!(exception.view().class, Term::atom(Atom::ERROR));
    assert_eq!(exception.view().reason, Term::atom(Atom::BADARG));

    scheduler.shutdown();
}

/// OS-thread names observed inside `dirty_spawn_path_probe`, proving each
/// invocation ran ON a dirty worker rather than a normal scheduler thread.
static DIRTY_SPAWN_PATH_THREADS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn dirty_spawn_path_probe(_args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    let thread_name = std::thread::current().name().unwrap_or_default().to_owned();
    DIRTY_SPAWN_PATH_THREADS
        .lock()
        .expect("dirty spawn-path thread log lock")
        .push(thread_name);
    Err(Term::atom(Atom::BADARG))
}

/// Exported zero-arity entry whose body makes one dirty-CPU native call that
/// raises `badarg`, so the child dies abnormally and its link fires.
fn exported_dirty_child(name: Atom, entry: Atom) -> Module {
    let mut child = module(
        name,
        vec![
            Instruction::Label { label: 1 },
            Instruction::CallExt {
                arity: Operand::Unsigned(0),
                import: Operand::Unsigned(0),
            },
            Instruction::Return,
        ],
    );
    child.exports.insert((entry, 0), 1);
    child.resolved_imports.push(native_import(
        dirty_spawn_path_probe,
        Some(DirtySchedulerKind::Cpu),
    ));
    child
}

/// Exported zero-arity entry that parks in `receive` forever, staying live
/// until a link exit signal arrives.
fn exported_parked_parent(name: Atom, entry: Atom) -> Module {
    let mut parent = module(
        name,
        vec![
            Instruction::Label { label: 1 },
            Instruction::Label { label: 10 },
            Instruction::LoopRec {
                fail: Operand::Label(20),
                destination: Operand::X(0),
            },
            Instruction::RemoveMessage,
            Instruction::Return,
            Instruction::Label { label: 20 },
            Instruction::Wait {
                fail: Operand::Label(10),
            },
        ],
    );
    parent.exports.insert((entry, 0), 1);
    parent
}

/// THE WALL for per-entry dirty dispatch on the linked spawn path: dirty
/// scheduling is a property of the NATIVE ENTRY, not the spawn path — the
/// child's dirty-CPU native executes on a named dirty worker thread through
/// the pool, and the link is real (the child's abnormal death kills the
/// parked parent). The `spawn_link_dirty` alias this wall once compared
/// against was removed at the 0.17.0 breaking window; `spawn_link` is the
/// one linked MFA spawn path.
#[test]
fn spawn_link_dirty_dispatch_is_per_entry_on_the_linked_path() {
    use beamr::atom::AtomTable;
    use beamr::native::BifRegistryImpl;

    DIRTY_SPAWN_PATH_THREADS
        .lock()
        .expect("dirty spawn-path thread log lock")
        .clear();

    let atoms = Arc::new(AtomTable::new());
    let parent_module_name = atoms.intern("dirty_spawn_path_parent");
    let parent_entry = atoms.intern("park");
    let child_module_name = atoms.intern("dirty_spawn_path_child");
    let child_entry = atoms.intern("crash_dirty");

    let registry = Arc::new(ModuleRegistry::new());
    registry.insert(exported_parked_parent(parent_module_name, parent_entry));
    registry.insert(exported_dirty_child(child_module_name, child_entry));

    let scheduler = beamr::scheduler::Scheduler::with_code_server(
        SchedulerConfig {
            thread_count: Some(1),
            dirty_cpu_threads: Some(1),
            dirty_io_threads: Some(1),
            dirty_queue_depth: Some(8),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        Arc::clone(&atoms),
        Arc::new(BifRegistryImpl::new()),
    )
    .expect("scheduler starts");

    let parent_pid = scheduler
        .spawn(parent_module_name, parent_entry, Vec::new())
        .expect("spawn parked parent");
    // Let the parent materialize and reach its receive park.
    std::thread::sleep(Duration::from_millis(100));

    let child_pid = scheduler
        .spawn_link(parent_pid, child_module_name, child_entry, Vec::new())
        .expect("linked spawn succeeds");

    let (child_reason, _child_value) = scheduler.run_until_exit(child_pid);
    assert_eq!(
        child_reason,
        ExitReason::Error,
        "the child's dirty badarg raises and kills it"
    );
    let (parent_reason, _parent_value) = scheduler.run_until_exit(parent_pid);
    assert_eq!(
        parent_reason,
        ExitReason::Error,
        "the link is real: the child's abnormal death kills the parked parent"
    );

    let observed_threads = DIRTY_SPAWN_PATH_THREADS
        .lock()
        .expect("dirty spawn-path thread log lock")
        .clone();
    assert_eq!(
        observed_threads.len(),
        1,
        "the dirty native ran exactly once, dispatched per entry"
    );
    for thread_name in &observed_threads {
        assert!(
            thread_name.starts_with("dirty-cpu"),
            "the dirty native executed ON a dirty worker (got thread {thread_name:?}) — \
             per-entry dispatch, independent of the spawn entrypoint"
        );
    }

    scheduler.shutdown();
}

static DISABLED_PEER_PROGRESS: AtomicUsize = AtomicUsize::new(0);

fn dirty_unreachable(_args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    // A disabled dirty pool refuses before this body could ever run.
    unreachable!("a disabled dirty pool must refuse before executing the native")
}

fn disabled_peer_progress(_args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    // A dedicated counter so this test never races the shared NORMAL_PROGRESS
    // with siblings in the same binary running in parallel.
    DISABLED_PEER_PROGRESS.fetch_add(1, Ordering::AcqRel);
    Ok(Term::small_int(7))
}

/// THE GATE (spec §3.2): on a scheduler with the dirty-CPU pool disabled, a
/// process making a dirty CPU call terminates PROMPTLY with the typed
/// service-unavailable error — it never wedges parked with no worker to
/// complete a gated suspension (readiness contract C2) — while an unrelated
/// process on the SAME scheduler runs to a normal exit.
#[test]
fn disabled_dirty_cpu_pool_refuses_call_and_lets_peers_progress() {
    DISABLED_PEER_PROGRESS.store(0, Ordering::Release);

    let registry = Arc::new(ModuleRegistry::new());
    let dirty_module = registry.insert(call_native_module(
        Atom::OK,
        native_import(dirty_unreachable, Some(DirtySchedulerKind::Cpu)),
    ));
    let normal_module = registry.insert(call_native_module(
        Atom::ERROR,
        native_import(disabled_peer_progress, None),
    ));

    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            dirty_cpu_threads: Some(0),
            dirty_io_threads: Some(1),
            dirty_queue_depth: Some(8),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .expect("scheduler starts");

    // Boundary snapshots for the refusal ORDERING claim (§3.2): refusal must
    // precede the whole gated-suspension sequence (call-id allocation ->
    // set_suspension -> mirror registration) and the dirty submit, so the
    // refused call moves none of these instruments. The allocation counter
    // pins the sequence's first side effect and the mirror counter its last;
    // entering the sequence and cleaning up would leave the same end state
    // but cannot leave the same counters.
    let allocations_before = scheduler.dirty_suspension_allocation_count();
    let mirrors_before = scheduler.suspension_mirror_registration_count();
    let spawns_before = completion_spawned_total(&scheduler);

    // The refused dirty process exits with the explicit error. run_until_exit
    // would hang forever on a park-forever bug, so its return IS the
    // non-wedging assertion.
    let dirty_pid = scheduler.spawn_process(&dirty_module);
    let (dirty_reason, _dirty_result) = scheduler.run_until_exit(dirty_pid);
    assert_eq!(dirty_reason, ExitReason::Error);
    assert_eq!(
        scheduler.take_exit_error(dirty_pid),
        Some(ExecError::ServiceUnavailable {
            service: "dirty-cpu"
        }),
    );

    // The ordering itself: the refused call never entered the suspension
    // sequence — no call id allocated, no mirror registered, no completion
    // thread spawned; not merely cleaned up afterwards.
    assert_eq!(
        scheduler.dirty_suspension_allocation_count(),
        allocations_before,
        "a refused dirty call must never allocate a suspension call id"
    );
    assert_eq!(
        scheduler.suspension_mirror_registration_count(),
        mirrors_before,
        "a refused dirty call must never register a suspension mirror"
    );
    assert_eq!(
        completion_spawned_total(&scheduler),
        spawns_before,
        "a refused dirty call must never spawn a completion thread"
    );

    // A peer on the SAME scheduler makes progress and exits normally.
    let normal_pid = scheduler.spawn_process(&normal_module);
    let (normal_reason, normal_result) = scheduler.run_until_exit(normal_pid);
    assert_eq!(normal_reason, ExitReason::Normal);
    assert_eq!(normal_result.root(), Term::small_int(7));
    assert_eq!(DISABLED_PEER_PROGRESS.load(Ordering::Acquire), 1);

    scheduler.shutdown();
}
