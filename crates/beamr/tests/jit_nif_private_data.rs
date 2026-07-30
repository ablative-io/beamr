//! Minimal reproduction: does a native BIF called BELOW a JIT-compiled frame
//! still receive the scheduler's `nif_private_data`?
//!
//! Chain: `boot:run/1` (bytecode) -call_ext-> `caller:run/1` (JIT-compiled)
//!        -call_ext_only-> `callee:run/1` (bytecode) -call_ext-> `host:probe/1`
//!        (native).
//!
//! The probe records a BITMASK over several facilities, not just
//! `nif_private_data`. The defect is not "one field goes missing" -- it is
//! "the whole `NativeServices` bundle is constructed default" -- so a wall that
//! asserts a single facility passes the day someone restores that facility and
//! drops another. `replay_driver` is the one that matters most and the one
//! whose loss is SILENT (it makes the ExternalIo/Entropy replay interception
//! in `native_call.rs` skip entirely and take the live path), so the wall must
//! carry the shape of the defect rather than the shape of the symptom that
//! found it.

use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use beamr::atom::{Atom, AtomTable};
use beamr::jit::{JitCacheKey, JitCompiler, JitSettings};
use beamr::loader::Instruction;
use beamr::loader::decode::compact::Operand;
use beamr::module::{Module, ModuleOrigin, ModuleRegistry, ResolvedImport, ResolvedImportTarget};
use beamr::native::{BifRegistryImpl, Capability, ProcessContext};
use beamr::scheduler::{Scheduler, SchedulerConfig};
use beamr::term::Term;

const UNSEEN: u8 = 0;
const PRESENT: u8 = 1;
const ABSENT: u8 = 2;

/// One bit per facility the probe interrogates. The names are the
/// `ProcessContext` accessors, so a failure diff points straight at the getter.
const F_NIF_PRIVATE_DATA: u32 = 1 << 0;
const F_ATOM_TABLE: u32 = 1 << 1;
const F_SPAWN_FACILITY: u32 = 1 << 2;
const F_LINK_FACILITY: u32 = 1 << 3;
const F_LOCAL_SEND_FACILITY: u32 = 1 << 4;
const F_SUPERVISION_FACILITY: u32 = 1 << 5;
const F_ETS_FACILITY: u32 = 1 << 6;
const F_CODE_MANAGEMENT_FACILITY: u32 = 1 << 7;
const F_PROCESS_INFO_FACILITY: u32 = 1 << 8;
const F_SYSTEM_INFO_FACILITY: u32 = 1 << 9;
const F_REPLAY_DRIVER: u32 = 1 << 10;

const FACILITY_NAMES: [(u32, &str); 11] = [
    (F_NIF_PRIVATE_DATA, "nif_private_data"),
    (F_ATOM_TABLE, "atom_table"),
    (F_SPAWN_FACILITY, "spawn_facility"),
    (F_LINK_FACILITY, "link_facility"),
    (F_LOCAL_SEND_FACILITY, "local_send_facility"),
    (F_SUPERVISION_FACILITY, "supervision_facility"),
    (F_ETS_FACILITY, "ets_facility"),
    (F_CODE_MANAGEMENT_FACILITY, "code_management_facility"),
    (F_PROCESS_INFO_FACILITY, "process_info_facility"),
    (F_SYSTEM_INFO_FACILITY, "system_info_facility"),
    (F_REPLAY_DRIVER, "replay_driver"),
];

fn describe(mask: u32) -> String {
    let present: Vec<&str> = FACILITY_NAMES
        .iter()
        .filter(|(bit, _)| mask & bit != 0)
        .map(|(_, name)| *name)
        .collect();
    if present.is_empty() {
        "<none>".to_owned()
    } else {
        present.join(", ")
    }
}

static PROBE: AtomicU8 = AtomicU8::new(UNSEEN);
static PROBE_FACILITIES: AtomicU32 = AtomicU32::new(0);

/// The probe is a process-wide static, because a NIF is a plain `fn` with
/// nowhere to hang per-case state. That makes the two cases below share it, so
/// they must not overlap: `cargo test` runs them on separate threads by
/// default, and without this lock whichever case finishes second reads the
/// other's answer and the verdict inverts. A wall that flips with thread
/// scheduling is worse than no wall.
static CASES_ARE_EXCLUSIVE: Mutex<()> = Mutex::new(());

/// Hold the case lock, tolerating poison.
///
/// A panic in one case (which is exactly what a failing assertion does) must
/// not turn every later case into a second, misleading failure about a
/// poisoned mutex. The guarded data is `()`, so there is no invariant a panic
/// could have left broken.
fn enter_case() -> MutexGuard<'static, ()> {
    match CASES_ARE_EXCLUSIVE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct EngineState;

fn probe(_args: &[Term], ctx: &mut ProcessContext) -> Result<Term, Term> {
    let mut facilities = 0_u32;
    let mut note = |present: bool, bit: u32| {
        if present {
            facilities |= bit;
        }
    };
    note(ctx.nif_private_data().is_some(), F_NIF_PRIVATE_DATA);
    note(ctx.atom_table().is_some(), F_ATOM_TABLE);
    note(ctx.spawn_facility().is_some(), F_SPAWN_FACILITY);
    note(ctx.link_facility().is_some(), F_LINK_FACILITY);
    note(ctx.local_send_facility().is_some(), F_LOCAL_SEND_FACILITY);
    note(ctx.supervision_facility().is_some(), F_SUPERVISION_FACILITY);
    note(ctx.ets_facility().is_some(), F_ETS_FACILITY);
    note(
        ctx.code_management_facility().is_some(),
        F_CODE_MANAGEMENT_FACILITY,
    );
    note(
        ctx.process_info_facility().is_some(),
        F_PROCESS_INFO_FACILITY,
    );
    note(ctx.system_info_facility().is_some(), F_SYSTEM_INFO_FACILITY);
    note(ctx.replay_driver().is_some(), F_REPLAY_DRIVER);

    PROBE_FACILITIES.store(facilities, Ordering::Release);
    let observed = if facilities & F_NIF_PRIVATE_DATA != 0 {
        PRESENT
    } else {
        ABSENT
    };
    PROBE.store(observed, Ordering::Release);
    Ok(Term::small_int(0))
}

fn empty_module(name: Atom) -> Module {
    Module {
        name,
        generation: 0,
        origin: ModuleOrigin::Preloaded,
        exports: std::collections::HashMap::new(),
        label_index: std::collections::HashMap::new(),
        code: Vec::new(),
        function_table: Vec::new(),
        line_table: Vec::new(),
        literals: Vec::new(),
        constant_pool: beamr::constant_pool::ConstantPool::new(),
        resolved_imports: Vec::new(),
        lambdas: Vec::new(),
        string_table: Vec::new(),
        line_info: Vec::new(),
    }
}

/// What one case observed: the reached-state and the facility bitmask.
struct Observed {
    reached: u8,
    facilities: u32,
}

fn run_case(with_jit: bool) -> Observed {
    let _exclusive = enter_case();
    PROBE.store(UNSEEN, Ordering::Release);
    PROBE_FACILITIES.store(0, Ordering::Release);

    let atoms = Arc::new(AtomTable::with_common_atoms());
    let registry = Arc::new(ModuleRegistry::new());
    let natives = Arc::new(BifRegistryImpl::new());

    let host = atoms.intern("host");
    let probe_fn = atoms.intern("probe");
    let run = atoms.intern("run");
    let boot = atoms.intern("boot");
    let caller = atoms.intern("caller");
    let callee = atoms.intern("callee");

    natives
        .register(host, probe_fn, 1, probe, Capability::ExternalIo)
        .expect("probe registration");
    let probe_entry = natives.lookup(host, probe_fn, 1).expect("probe lookup");

    // callee:run/1 — calls the native probe.
    let mut callee_module = empty_module(callee);
    callee_module.code = vec![
        Instruction::Label { label: 1 },
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
        Instruction::Return,
    ];
    callee_module.exports.insert((run, 1), 1);
    callee_module.label_index.insert(1, 0);
    callee_module.resolved_imports.push(ResolvedImport {
        module: host,
        function: probe_fn,
        arity: 1,
        target: ResolvedImportTarget::Native(probe_entry),
    });
    registry.insert(callee_module);

    // caller:run/1 — a JIT-eligible function whose body is one call_ext_only.
    let mut caller_module = empty_module(caller);
    caller_module.code = vec![
        Instruction::Label { label: 1 },
        Instruction::FuncInfo {
            module: Operand::Atom(Some(caller)),
            function: Operand::Atom(Some(run)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 2 },
        Instruction::CallExtOnly {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
    ];
    caller_module.exports.insert((run, 1), 2);
    caller_module.label_index.insert(1, 0);
    caller_module.label_index.insert(2, 2);
    caller_module.function_table.push((1, run, 1));
    caller_module.resolved_imports.push(ResolvedImport {
        module: callee,
        function: run,
        arity: 1,
        target: ResolvedImportTarget::Code {
            module: callee,
            label: 1,
        },
    });
    registry.insert(caller_module);

    // boot:run/1 — the bytecode entrypoint whose call_ext enters the JIT.
    let mut boot_module = empty_module(boot);
    boot_module.code = vec![
        Instruction::Label { label: 1 },
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
        Instruction::Return,
    ];
    boot_module.exports.insert((run, 1), 1);
    boot_module.label_index.insert(1, 0);
    boot_module.resolved_imports.push(ResolvedImport {
        module: caller,
        function: run,
        arity: 1,
        target: ResolvedImportTarget::Code {
            module: caller,
            label: 2,
        },
    });
    registry.insert(boot_module);

    let config = SchedulerConfig {
        thread_count: Some(1),
        nif_private_data: Some(Arc::new(EngineState) as _),
        ..Default::default()
    };
    let scheduler = Arc::new(
        Scheduler::with_code_server(
            config,
            Arc::clone(&registry),
            Arc::clone(&atoms),
            Arc::clone(&natives),
        )
        .expect("scheduler"),
    );

    if with_jit {
        let module = registry.lookup(caller).expect("caller module");
        let entry_ip = module.export_ip(run, 1).expect("caller entry ip");
        let instructions = module
            .function_instructions(entry_ip)
            .expect("caller instruction slice");
        let compiler = JitCompiler::new(JitSettings).expect("jit compiler");
        let code = compiler
            .compile(instructions, caller, run, 1)
            .expect("caller compilation");
        scheduler
            .jit_cache()
            .insert(JitCacheKey::new(caller, run, 1, module.generation()), code);
    }

    scheduler
        .spawn(boot, run, vec![Term::small_int(7)])
        .expect("spawn");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut observed = PROBE.load(Ordering::Acquire);
    while observed == UNSEEN && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
        observed = PROBE.load(Ordering::Acquire);
    }
    scheduler.shutdown();
    Observed {
        reached: observed,
        facilities: PROBE_FACILITIES.load(Ordering::Acquire),
    }
}

/// Fail with a distinct message when the probe never ran at all.
///
/// `UNSEEN` and `ABSENT` are different diagnoses and must not arrive as the
/// same red. A bare `left: 0, right: 1` tells the next reader nothing about
/// whether the chain was broken or the services were empty, and sending
/// someone to fix the wrong half is exactly the failure this file already
/// produced once.
fn reached_the_probe(observed: &Observed, case: &str) {
    assert_ne!(
        observed.reached, UNSEEN,
        "the probe never ran in the {case} case, so this says nothing about \
         native services -- the module chain or the dispatch is broken"
    );
}

/// Baseline: with no compiled code in play the probe sees the installed
/// bundle. This pins that the module graph is sound, so the JIT case isolates
/// the JIT as the only difference.
#[test]
fn bytecode_only_chain_sees_the_installed_native_services() {
    let observed = run_case(false);
    reached_the_probe(&observed, "bytecode-only");
    assert_eq!(
        observed.reached, PRESENT,
        "bytecode-only chain lost nif_private_data"
    );
    assert_ne!(
        observed.facilities, 0,
        "the baseline saw an entirely empty bundle, so it cannot serve as a control"
    );
}

/// The defect. Asserts the WHOLE bundle, not one field of it.
///
/// The JIT path must deliver exactly what the bytecode path delivers. Asserting
/// only `nif_private_data` would go green the day someone restores that field
/// and leaves `replay_driver` empty -- and `replay_driver` is the one whose
/// loss produces no failure at all, just a live path taken during replay.
///
/// WHAT THIS ASSERTION DOES NOT OBSERVE, and do not read it as more than it is:
/// `replay_driver` IS NOT COVERED BY THIS TEST. This harness never installs
/// one -- the scheduler derives it from the replay mode (the `match replay_mode`
/// binding at the head of `Scheduler::construct_with_services`), and
/// `SchedulerConfig` exposes no field for it, so it cannot be installed from a
/// test at all -- so `F_REPLAY_DRIVER` is zero on BOTH sides and a
/// set equality cannot see a bit that is absent from both. Reading a green here
/// as "the replay driver survives the JIT boundary" is wrong. That property
/// holds by CONSTRUCTION, because the fix threads the whole `NativeServices`
/// bundle and there is no per-facility path for it to be dropped from -- but
/// construction is not observation, and this test is observation.
///
/// The probe notes 11 of the bundle's 33 `pub` fields. Unobserved: `timers`,
/// `io_sink`, `bif_registry`, `jit_cache`, `pg_facility`, `readiness_facility`,
/// `suspension_registrar`, the capability sink and violation handler, and the
/// `net`-gated facilities, among others.
///
/// The reason this is a recorded limitation and not a hole: the expectation is
/// DERIVED FROM THE BYTECODE BASELINE rather than hardcoded, so any harness
/// that does install a replay driver -- or any facility added to the probe --
/// gets it covered with no edit to this assertion.
#[test]
fn chain_below_a_jit_frame_sees_the_same_native_services() {
    let baseline = run_case(false);
    reached_the_probe(&baseline, "bytecode-only baseline");

    let jitted = run_case(true);
    reached_the_probe(&jitted, "JIT");

    assert_eq!(
        describe(jitted.facilities),
        describe(baseline.facilities),
        "a call below a JIT-compiled frame received a different native-services \
         bundle than the identical bytecode-only chain; beamr \
         jit/runtime.rs:165 re-enters the interpreter via run_with_registry, \
         which builds NativeServices::default()"
    );
    assert_eq!(
        jitted.reached, PRESENT,
        "a native below a JIT-compiled frame lost nif_private_data"
    );
}
