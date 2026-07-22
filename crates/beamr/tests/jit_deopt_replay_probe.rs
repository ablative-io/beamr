//! ADMISSION ARC LEG 1a — replay-reachability probe.
//!
//! Settles, empirically and end-to-end through the WIRED demand path, whether
//! the deopt-restart "double-send" / "message-loss" soundness class described in
//! REAL-ERLC-ADMISSION-SCOPING.md §2 is LIVE on `main` in the jit_badarith proof
//! pattern. Verdict is recorded in the leg-1 handoff either way.
//!
//! Ground (byte-verified on 1a1ca1e, restated so a future reader can re-derive).
//! A native deopt (`JIT_STATUS_DEOPT`) returns `Ok(None)` from `call_native`
//! (interpreter/opcodes/core.rs:850); the call site (core.rs:125-130) then falls
//! through to `jump_with_reduction` to the callee's entry label — the callee
//! re-interprets FROM ITS START, replaying any side effect the native body
//! already committed. `RecvMarkerReserve` is `Coverage::Supported`
//! (ir_control.rs:71) but its lowering UNCONDITIONALLY emits `return DEOPT`
//! (dispatch_core.rs:411-417); it is thus a "runtime-deopt-capable instruction"
//! that a real erlc receive prelude can place AFTER an observable side effect in
//! one admitted slice. It is the cleanest end-to-end instance of the class; the
//! typed-overflow shape the scoping names is unreachable after a side effect
//! (see the handoff for why).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use beamr::atom::{Atom, AtomTable};
use beamr::loader::Instruction;
use beamr::loader::decode::compact::Operand;
use beamr::module::{Module, ModuleOrigin, ModuleRegistry, ResolvedImport, ResolvedImportTarget};
use beamr::process::ExitReason;
use beamr::scheduler::{Scheduler, SchedulerConfig, SchedulerServices};
use beamr::term::Term;

const WAIT_BUDGET: Duration = Duration::from_secs(10);

fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + WAIT_BUDGET;
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn config(threshold: u32) -> SchedulerConfig {
    SchedulerConfig {
        thread_count: Some(1),
        dirty_cpu_threads: Some(1),
        dirty_io_threads: Some(1),
        jit_threshold: Some(threshold),
        ..SchedulerConfig::default()
    }
}

fn finish_module(
    name: Atom,
    code: Vec<Instruction>,
    exports: HashMap<(Atom, u8), u32>,
    resolved_imports: Vec<ResolvedImport>,
) -> Module {
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            _ => None,
        })
        .collect();
    let function_table = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::FuncInfo {
                function: Operand::Atom(Some(function)),
                arity: Operand::Unsigned(arity),
                ..
            } => Some((ip, *function, u8::try_from(*arity).ok()?)),
            _ => None,
        })
        .collect();
    Module {
        name,
        generation: 0,
        origin: ModuleOrigin::Preloaded,
        exports,
        label_index,
        code,
        function_table,
        line_table: Vec::new(),
        literals: Vec::new(),
        constant_pool: Default::default(),
        resolved_imports,
        lambdas: Vec::new(),
        string_table: Vec::new(),
        line_info: Vec::new(),
    }
}

/// An infinite mailbox sink: `loop/0` drains its mailbox forever and never
/// exits. It is a live, stable `Send` target for the whole test, so the probe's
/// heating drives all deliver successfully. Built with a `Jump`-based receive
/// loop so it needs no import table.
fn sink_module(name: Atom, function: Atom) -> Module {
    let mut exports = HashMap::new();
    exports.insert((function, 0), 1);
    let code = vec![
        Instruction::FuncInfo {
            module: Operand::Atom(Some(name)),
            function: Operand::Atom(Some(function)),
            arity: Operand::Unsigned(0),
        },
        Instruction::Label { label: 1 },
        Instruction::Label { label: 10 },
        Instruction::LoopRec {
            fail: Operand::Label(20),
            destination: Operand::X(0),
        },
        Instruction::RemoveMessage,
        Instruction::Jump {
            target: Operand::Label(10),
        },
        Instruction::Label { label: 20 },
        Instruction::Wait {
            fail: Operand::Label(10),
        },
    ];
    finish_module(name, code, exports, Vec::new())
}

/// A module whose `driver/1` tail-calls (`call_ext_only`, the heatable external
/// edge) the local `probe/1`, whose body is `[Send, RecvMarkerReserve, Return]`:
/// an observable side effect immediately followed by an unconditional runtime
/// deopt. `driver/1` receives the sink PID in x0 and threads it into `probe/1`.
fn probe_module(name: Atom, driver: Atom, probe: Atom, message: Atom) -> Module {
    // Imports: slot 0 = self-module probe/1 (the external tail edge that heats).
    let imports = vec![ResolvedImport {
        module: name,
        function: probe,
        arity: 1,
        target: ResolvedImportTarget::Code {
            module: name,
            label: 3,
        },
    }];
    let mut exports = HashMap::new();
    exports.insert((driver, 1), 1);
    exports.insert((probe, 1), 3);
    let code = vec![
        // driver/1 (x0 = sink pid): tail-call probe/1 through the import edge.
        Instruction::FuncInfo {
            module: Operand::Atom(Some(name)),
            function: Operand::Atom(Some(driver)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 1 },
        Instruction::CallExtOnly {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
        // probe/1 (x0 = sink pid): Send hello to the sink, then a deopt-capable op.
        Instruction::FuncInfo {
            module: Operand::Atom(Some(name)),
            function: Operand::Atom(Some(probe)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 3 },
        Instruction::Move {
            source: Operand::Atom(Some(message)),
            destination: Operand::X(1),
        },
        // Send: dest = x0 (sink pid), msg = x1 (hello); result -> x0.
        Instruction::Send,
        // Unconditional runtime deopt AFTER the send (dispatch_core.rs:411-417).
        Instruction::RecvMarkerReserve {
            dest: Operand::X(2),
        },
        Instruction::Return,
    ];
    finish_module(name, code, exports, imports)
}

/// PRIMARY 1a PROBE — deopt-after-side-effect divergence, end-to-end through the
/// wired demand path, in the jit_badarith proof shape.
///
/// `probe/1` performs `[Send hello -> sink, RecvMarkerReserve, Return]`. Once it
/// is heated and cached native, a drive runs the compiled body: the `Send` is
/// committed, then `RecvMarkerReserve` unconditionally deopts, and the callee is
/// re-interpreted FROM ITS ENTRY — replaying the `Send` on a register file the
/// native body already mutated (x0, the send DESTINATION, now holds the send
/// RESULT). The soundness contract is that the JIT-live drive is observably
/// EQUAL to the interpreter-only (minimal) drive. This assertion is the fail-first
/// wall for leg 1b: RED on `main` if the class is live, GREEN once the 1b
/// pre-pass guard rejects the shape (interpreter-only, no divergence).
#[test]
fn deopt_after_send_is_interpreter_equal() {
    let atoms = AtomTable::with_common_atoms();
    let name = atoms.intern("deopt_probe");
    let driver = atoms.intern("driver");
    let probe = atoms.intern("probe");
    let hello = atoms.intern("hello");
    let sink_name = atoms.intern("probe_sink");
    let sink_fn = atoms.intern("loop");

    // JIT-live composition.
    let registry = Arc::new(ModuleRegistry::new());
    registry.insert(sink_module(sink_name, sink_fn));
    let probe_mod = registry.insert(probe_module(name, driver, probe, hello));
    let generation = probe_mod.generation();
    let jit = Scheduler::new(config(2), Arc::clone(&registry)).expect("jit scheduler starts");

    let sink_pid = jit
        .spawn(sink_name, sink_fn, Vec::new())
        .expect("spawn sink");
    let sink_term = Term::try_pid(sink_pid).expect("sink pid fits");

    // Heat probe/1 at the external tail edge with successful (interpreted) drives.
    for _ in 0..2 {
        let pid = jit
            .spawn(name, driver, vec![sink_term])
            .expect("spawn driver (heat)");
        assert_eq!(
            jit.run_until_exit(pid).0,
            ExitReason::Normal,
            "pre-compile drive exits normally (interpreter runs Send + RecvMarkerReserve)"
        );
    }
    // Wait for the tier's VERDICT on probe/1 — success OR unsupported — so the
    // observed drive is deterministic in both worlds: pre-guard probe/1 compiles
    // and the drive dispatches native (Send -> DEOPT -> replay); post-guard the
    // 1b pre-pass marks it unsupported and the drive stays interpreter-only.
    assert!(
        wait_until(|| {
            let counters = jit.jit_profiler().compile_outcome_counters();
            counters.successes == 1 || counters.unsupported == 1
        }),
        "probe/1 must reach a compile verdict on the dirty-CPU service"
    );
    let _ = generation;

    // The observed drive: pre-guard native probe/1 -> Send -> DEOPT -> replay;
    // post-guard interpreter-only.
    let drive_pid = jit
        .spawn(name, driver, vec![sink_term])
        .expect("spawn driver (deopt drive)");
    let jit_reason = jit.run_until_exit(drive_pid).0;
    let jit_error = jit.take_exit_error(drive_pid);
    jit.shutdown();

    // Minimal (JIT-absent) composition: identical drive, interpreter only.
    let minimal_registry = Arc::new(ModuleRegistry::new());
    minimal_registry.insert(sink_module(sink_name, sink_fn));
    minimal_registry.insert(probe_module(name, driver, probe, hello));
    let minimal = Scheduler::with_services(
        config(2),
        SchedulerServices::minimal(),
        Arc::clone(&minimal_registry),
    )
    .expect("minimal scheduler starts");
    let msink = minimal
        .spawn(sink_name, sink_fn, Vec::new())
        .expect("spawn sink (minimal)");
    let msink_term = Term::try_pid(msink).expect("sink pid fits (minimal)");
    let minimal_pid = minimal
        .spawn(name, driver, vec![msink_term])
        .expect("spawn driver (minimal)");
    let minimal_reason = minimal.run_until_exit(minimal_pid).0;
    let minimal_error = minimal.take_exit_error(minimal_pid);
    minimal.shutdown();

    // Soundness: the deopt handed control to the interpreter, which must produce
    // the SAME observable as running interpreter-only from the start.
    assert_eq!(
        minimal_reason,
        ExitReason::Normal,
        "interpreter-only drive is normal (Send delivers, RecvMarkerReserve continues, Return)"
    );
    assert_eq!(
        (jit_reason, jit_error),
        (minimal_reason, minimal_error),
        "JIT-live deopt-restart drive must be observably equal to the interpreter-only drive; \
         a divergence here is the live deopt-after-side-effect soundness defect"
    );
}
