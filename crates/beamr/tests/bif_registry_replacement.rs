#![cfg(feature = "threads")]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use beamr::atom::{Atom, AtomTable};
use beamr::constant_pool::ConstantPool;
use beamr::loader::decode::Operand;
use beamr::loader::{Instruction, LambdaEntry, lambda_unique_id};
use beamr::module::{Module, ModuleOrigin, ModuleRegistry, ResolvedImport, ResolvedImportTarget};
use beamr::native::gate3_bifs::register_gate3_bifs;
use beamr::native::{BifRegistryImpl, Capability, NativeEntry, ProcessContext};
use beamr::process::ExitReason;
use beamr::scheduler::{Scheduler, SchedulerConfig, SchedulerServices};
use beamr::term::Term;

static SPAWN_PREVIOUS: OnceLock<NativeEntry> = OnceLock::new();
static SPAWN_LINK_PREVIOUS: OnceLock<NativeEntry> = OnceLock::new();
static SPAWN_WRAPPER_CALLS: AtomicUsize = AtomicUsize::new(0);
static SPAWN_LINK_WRAPPER_CALLS: AtomicUsize = AtomicUsize::new(0);
static SPAWN_CHILD_PID: AtomicU64 = AtomicU64::new(0);
static SPAWN_LINK_CHILD_PID: AtomicU64 = AtomicU64::new(0);

fn wrapped_spawn(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    SPAWN_WRAPPER_CALLS.fetch_add(1, Ordering::Relaxed);
    let previous = SPAWN_PREVIOUS.get().expect("spawn/1 original installed");
    let result = (previous.function)(args, context);
    if let Ok(term) = result {
        SPAWN_CHILD_PID.store(
            term.as_pid().expect("spawn/1 returns a pid"),
            Ordering::Release,
        );
    }
    result
}

fn wrapped_spawn_link(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    SPAWN_LINK_WRAPPER_CALLS.fetch_add(1, Ordering::Relaxed);
    let previous = SPAWN_LINK_PREVIOUS
        .get()
        .expect("spawn_link/1 original installed");
    let result = (previous.function)(args, context);
    if let Ok(term) = result {
        SPAWN_LINK_CHILD_PID.store(
            term.as_pid().expect("spawn_link/1 returns a pid"),
            Ordering::Release,
        );
    }
    result
}

fn label_index(code: &[Instruction]) -> HashMap<u32, usize> {
    code.iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            _ => None,
        })
        .collect()
}

fn spawn_consumer_module(
    atoms: &AtomTable,
    registry: &BifRegistryImpl,
    module_name: Atom,
) -> Module {
    let erlang = atoms.intern("erlang");
    let spawn = atoms.intern("spawn");
    let spawn_link = atoms.intern("spawn_link");
    let first_child = atoms.intern("first_child");
    let second_child = atoms.intern("second_child");
    let code = vec![
        Instruction::MakeFun {
            operands: vec![Operand::Unsigned(0)],
        },
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
        Instruction::MakeFun {
            operands: vec![Operand::Unsigned(1)],
        },
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(1),
        },
        Instruction::Return,
        Instruction::Label { label: 10 },
        Instruction::Move {
            source: Operand::Unsigned(71),
            destination: Operand::X(0),
        },
        Instruction::Return,
        Instruction::Label { label: 20 },
        Instruction::Move {
            source: Operand::Unsigned(72),
            destination: Operand::X(0),
        },
        Instruction::Return,
    ];

    Module {
        name: module_name,
        generation: 0,
        origin: ModuleOrigin::Preloaded,
        exports: HashMap::new(),
        label_index: label_index(&code),
        code,
        function_table: Vec::new(),
        line_table: Vec::new(),
        literals: Vec::new(),
        constant_pool: ConstantPool::default(),
        resolved_imports: vec![
            ResolvedImport {
                module: erlang,
                function: spawn,
                arity: 1,
                target: ResolvedImportTarget::Native(
                    registry.lookup(erlang, spawn, 1).expect("wrapped spawn/1"),
                ),
            },
            ResolvedImport {
                module: erlang,
                function: spawn_link,
                arity: 1,
                target: ResolvedImportTarget::Native(
                    registry
                        .lookup(erlang, spawn_link, 1)
                        .expect("wrapped spawn_link/1"),
                ),
            },
        ],
        lambdas: vec![
            LambdaEntry {
                function: first_child,
                arity: 0,
                label: 10,
                num_free: 0,
                unique_id: lambda_unique_id(atoms, module_name, first_child, 0, 0)
                    .expect("first child lambda id"),
            },
            LambdaEntry {
                function: second_child,
                arity: 0,
                label: 20,
                num_free: 0,
                unique_id: lambda_unique_id(atoms, module_name, second_child, 0, 0)
                    .expect("second child lambda id"),
            },
        ],
        string_table: Vec::new(),
        line_info: Vec::new(),
    }
}

#[test]
fn full_gate3_then_replace_spawn_wrappers_delegate_and_children_complete() {
    let atoms = Arc::new(AtomTable::with_common_atoms());
    let bifs = Arc::new(BifRegistryImpl::new());
    register_gate3_bifs(&bifs, &atoms).expect("install the complete Gate-3 table");

    let erlang = atoms.intern("erlang");
    let spawn = atoms.intern("spawn");
    let spawn_link = atoms.intern("spawn_link");
    let spawn_previous = bifs
        .replace_existing(erlang, spawn, 1, wrapped_spawn, Capability::Spawn)
        .expect("replace Gate-3 spawn/1");
    let spawn_link_previous = bifs
        .replace_existing(erlang, spawn_link, 1, wrapped_spawn_link, Capability::Spawn)
        .expect("replace Gate-3 spawn_link/1");
    SPAWN_PREVIOUS
        .set(spawn_previous)
        .expect("install spawn/1 delegate exactly once");
    SPAWN_LINK_PREVIOUS
        .set(spawn_link_previous)
        .expect("install spawn_link/1 delegate exactly once");

    let modules = Arc::new(ModuleRegistry::new());
    let module_name = atoms.intern("registry_replacement_consumer");
    let module = modules.insert(spawn_consumer_module(&atoms, &bifs, module_name));
    let scheduler = Scheduler::with_services_and_code_server(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        SchedulerServices::minimal(),
        Arc::clone(&modules),
        Arc::clone(&atoms),
        Arc::clone(&bifs),
    )
    .expect("start scheduler with the replaced Gate-3 registry");

    let parent_pid = scheduler.spawn_process(&module);
    let (parent_reason, parent_value) = scheduler.run_until_exit(parent_pid);
    let spawn_child_pid = SPAWN_CHILD_PID.load(Ordering::Acquire);
    let spawn_link_child_pid = SPAWN_LINK_CHILD_PID.load(Ordering::Acquire);

    assert_eq!(parent_reason, ExitReason::Normal);
    assert_eq!(SPAWN_WRAPPER_CALLS.load(Ordering::Relaxed), 1);
    assert_eq!(SPAWN_LINK_WRAPPER_CALLS.load(Ordering::Relaxed), 1);
    assert_ne!(spawn_child_pid, 0, "spawn/1 wrapper recorded its child");
    assert_ne!(
        spawn_link_child_pid, 0,
        "spawn_link/1 wrapper recorded its child"
    );
    assert_eq!(parent_value.root().as_pid(), Some(spawn_link_child_pid));

    let (spawn_reason, spawn_value) = scheduler.run_until_exit(spawn_child_pid);
    let (spawn_link_reason, spawn_link_value) = scheduler.run_until_exit(spawn_link_child_pid);

    assert_eq!(spawn_reason, ExitReason::Normal);
    assert_eq!(spawn_value.root().as_small_int(), Some(71));
    assert_eq!(spawn_link_reason, ExitReason::Normal);
    assert_eq!(spawn_link_value.root().as_small_int(), Some(72));

    scheduler.shutdown();
}
