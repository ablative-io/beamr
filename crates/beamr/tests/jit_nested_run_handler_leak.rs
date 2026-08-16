//! Gate test for the nested-run exception-handler leak (crash-2 / #193).
//!
//! The defect: `jit_call_interpreted` (jit/runtime.rs) services a compiled
//! trampoline's body call by running a FULL nested `run_with_native_services`
//! on the SHARED process. The exception-handler stack was process-level with no
//! nesting barrier, so a raise inside the nested run whose nearest handler was
//! registered OUTSIDE the nesting popped that outer handler, truncated, and
//! resumed the OUTER code at its catch label *while still inside the nested
//! run*. The compiled invocation and its Rust frames (invoke_jit -> native ->
//! jit_call_interpreted -> run) were never returned through: one leaked native
//! nesting per caught exception.
//!
//! Two consequences followed, and this file gates the observable one. The
//! eventual fatal unwind exited every leaked run LIFO, and each leaked
//! trampoline's exception arm fired `jit_add_compiled_frame`
//! (jit/ir_exceptions.rs) once — so a death record that should carry ONE
//! compiled frame carried K+1, where K is the number of previously-caught
//! exceptions. That is the 0.18.2 soak specimen's exact shape (seven identical
//! line-less `gleam@json:int/1` frames = 6 caught + 1 fatal). The unobservable
//! consequence is unbounded Rust-stack growth, which has no in-process
//! assertion; the frame count is its proxy.
//!
//! The fix has two halves and this file gates BOTH, because the first half
//! alone is a worse defect than the one it repairs:
//! 1. `Process::nested_handler_floor` — a nested run may not consume a handler
//!    installed outside it, so the raise leaves through the trampoline instead.
//! 2. `call_native`'s `JIT_STATUS_EXCEPTION` arm re-offers that exception to
//!    the handlers at THIS level (`dispatch_captured_exception`) instead of
//!    exiting unconditionally. Without it, half 1 turns every exception whose
//!    protected region crosses a trampoline into an uncatchable process kill.
//!
//! Five arms:
//! * `compiled_trampoline_records_one_frame_after_caught_exceptions` — K=6
//!   in-VM-caught badargs through the COMPILED trampoline, then one fatal. The
//!   record must name the trampoline ONCE.
//! * `ghost_frame_count_is_independent_of_catch_count` — the same program at
//!   K=2 and K=6; the counts must be EQUAL. This is what a partial fix cannot
//!   satisfy: it pins the count as independent of catch count rather than
//!   merely smaller.
//! * `each_trampoline_records_one_frame_at_nesting_depth_two` — compiled ->
//!   interpreted -> compiled -> raise, catch at the outermost level. One frame
//!   per trampoline, at both catch counts: the count is a property of the
//!   propagation, not of nesting depth.
//! * `caught_exception_value_reaches_the_handler_across_a_compiled_trampoline`
//!   — the liveness half under its own assertion. The exception must still be
//!   CAUGHT, and the handler must receive the value the interpreted path
//!   delivers.
//! * `interpreted_catches_record_no_compiled_frames_control` — the identical
//!   program with the JIT threshold at `u32::MAX` (nothing ever compiles).
//!   Proves the harness and pins the divergence to compiled execution.
//!
//! Measured discrimination (both directions, at my hands):
//! * At the unfixed bytes the three count arms are RED — 7 vs 1 at K=6, 3 vs 7
//!   across K, and 3 per trampoline at depth two — while the value arm passes.
//! * With half 1 only, ALL FOUR count arms pass and the value arm goes RED with
//!   the process killed (`got Error`). The liveness assertion is the sole thing
//!   standing between this fix and a worse bug; it is not decoration.
//!
//! Falsifier rails, so a zero can never be mistaken for a pass: every arm
//! asserts the death record is non-empty AND contains at least one line-bearing
//! interpreted frame (a record the reader cannot parse would otherwise report
//! zero compiled frames and look green), and the compiled arms assert the
//! trampolines they depend on actually reached COMPILED with every compile job
//! settled before the arm ran.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use beamr::atom::{Atom, AtomTable};
use beamr::loader::decode::compact::Operand;
use beamr::loader::{Instruction, LineInfo};
use beamr::module::{Module, ModuleOrigin, ModuleRegistry, ResolvedImport, ResolvedImportTarget};
use beamr::native::BifRegistryImpl;
use beamr::native::bifs::register_gate1_bifs;
use beamr::native::stdlib_stubs::register_stdlib_stubs;
use beamr::process::ExitReason;
use beamr::scheduler::{Scheduler, SchedulerConfig};
use beamr::term::Term;
use beamr::term::boxed::Tuple;

const WAIT_BUDGET: Duration = Duration::from_secs(20);
const THRESHOLD: u32 = 5;
const NEVER_COMPILE: u32 = u32::MAX;

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

/// `jit_wireup.rs`'s `finish_module`, extended with a line table so interpreted
/// frames carry line info — the falsifier rail needs the genuine interpreted
/// raiser to be line-bearing while compiled pushes stay line-less.
fn finish_module(
    name: Atom,
    code: Vec<Instruction>,
    exports: HashMap<(Atom, u8), u32>,
    resolved_imports: Vec<ResolvedImport>,
    line_table: Vec<(usize, usize)>,
    line_info: Vec<LineInfo>,
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
        line_table,
        literals: Vec::new(),
        constant_pool: Default::default(),
        resolved_imports,
        lambdas: Vec::new(),
        string_table: Vec::new(),
        line_info,
    }
}

struct Names {
    ffimod: Atom,
    bif_int: Atom,
    wrapmod: Atom,
    int_wrap: Atom,
    wrapmid: Atom,
    int_mid: Atom,
    wrapouter: Atom,
    int_outer: Atom,
    bad: Atom,
}

/// `ffimod:bif_int/1` — CallExtOnly to `erlang:integer_to_binary/1` (Native).
/// A non-integer argument raises badarg: the soak specimen's exact face.
fn ffi_module(atoms: &AtomTable, bifs: &BifRegistryImpl, names: &Names) -> Module {
    let erlang = atoms.intern("erlang");
    let i2b = atoms.intern("integer_to_binary");
    let entry = bifs
        .lookup(erlang, i2b, 1)
        .expect("erlang:integer_to_binary/1 registered");
    let code = vec![
        Instruction::FuncInfo {
            module: Operand::Atom(Some(names.ffimod)),
            function: Operand::Atom(Some(names.bif_int)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 7 },
        Instruction::CallExtOnly {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
    ];
    let mut exports = HashMap::new();
    exports.insert((names.bif_int, 1), 7);
    let imports = vec![ResolvedImport {
        module: erlang,
        function: i2b,
        arity: 1,
        target: ResolvedImportTarget::Native(entry),
    }];
    finish_module(
        names.ffimod,
        code,
        exports,
        imports,
        vec![(1, 0), (2, 1)],
        vec![
            LineInfo { file: 0, line: 101 },
            LineInfo { file: 0, line: 102 },
        ],
    )
}

/// `wrapmod:int_wrap/1` — the trampoline under test: a tail body call
/// (CallExtOnly through a ResolvedImport) to `ffimod:bif_int/1`. This is the
/// shape the tier compiles, and the shape whose body call runs a nested
/// interpreter on the shared process.
fn wrap_module(names: &Names) -> Module {
    let code = vec![
        Instruction::FuncInfo {
            module: Operand::Atom(Some(names.wrapmod)),
            function: Operand::Atom(Some(names.int_wrap)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 2 },
        Instruction::CallExtOnly {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
    ];
    let mut exports = HashMap::new();
    exports.insert((names.int_wrap, 1), 2);
    let imports = vec![ResolvedImport {
        module: names.ffimod,
        function: names.bif_int,
        arity: 1,
        target: ResolvedImportTarget::Code {
            module: names.ffimod,
            label: 7,
        },
    }];
    finish_module(
        names.wrapmod,
        code,
        exports,
        imports,
        vec![(1, 0), (2, 1)],
        vec![
            LineInfo { file: 0, line: 201 },
            LineInfo { file: 0, line: 202 },
        ],
    )
}

fn wrap_import(names: &Names) -> ResolvedImport {
    ResolvedImport {
        module: names.wrapmod,
        function: names.int_wrap,
        arity: 1,
        target: ResolvedImportTarget::Code {
            module: names.wrapmod,
            label: 2,
        },
    }
}

/// One link of the depth-2 chain: a tail body call (CallExtOnly through a
/// ResolvedImport) from `module:function/1` to `target`, at `label`.
fn link_module(
    module: Atom,
    function: Atom,
    label: u32,
    target: ResolvedImport,
    first_line: u32,
) -> Module {
    let code = vec![
        Instruction::FuncInfo {
            module: Operand::Atom(Some(module)),
            function: Operand::Atom(Some(function)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label },
        Instruction::CallExtOnly {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
    ];
    let mut exports = HashMap::new();
    exports.insert((function, 1), label);
    finish_module(
        module,
        code,
        exports,
        vec![target],
        vec![(1, 0), (2, 1)],
        vec![
            LineInfo {
                file: 0,
                line: first_line,
            },
            LineInfo {
                file: 0,
                line: first_line + 1,
            },
        ],
    )
}

/// `wrapmid:int_mid/1` — the INTERPRETED middle of the depth-2 chain, body
/// calling `wrapmod:int_wrap/1`. It is only ever entered by
/// `jit_call_interpreted`, which records no call miss, so it never heats: the
/// depth-2 arm asserts that, because a compiled middle would collapse the two
/// nestings the arm exists to create.
fn mid_module(names: &Names) -> Module {
    link_module(names.wrapmid, names.int_mid, 3, wrap_import(names), 401)
}

/// `wrapouter:int_outer/1` — the OUTER trampoline of the depth-2 chain, body
/// calling the interpreted middle. Compiled, so its body call opens nesting #1
/// and the inner `wrapmod:int_wrap/1` trampoline opens nesting #2 inside it.
fn outer_module(names: &Names) -> Module {
    link_module(
        names.wrapouter,
        names.int_outer,
        4,
        ResolvedImport {
            module: names.wrapmid,
            function: names.int_mid,
            arity: 1,
            target: ResolvedImportTarget::Code {
                module: names.wrapmid,
                label: 3,
            },
        },
        451,
    )
}

fn outer_import(names: &Names) -> ResolvedImport {
    ResolvedImport {
        module: names.wrapouter,
        function: names.int_outer,
        arity: 1,
        target: ResolvedImportTarget::Code {
            module: names.wrapouter,
            label: 4,
        },
    }
}

/// Warm driver: `calls` body calls to `wrapmod:int_wrap(5)` — a VALID argument,
/// so warming never raises — then Return. Heats the wrapper past the threshold.
fn warm_module(atoms: &AtomTable, entry: ResolvedImport, calls: usize) -> Module {
    let name = atoms.intern("warmmod");
    let mut code = vec![Instruction::Label { label: 1 }];
    for _ in 0..calls {
        code.push(Instruction::Move {
            source: Operand::Integer(5),
            destination: Operand::X(0),
        });
        code.push(Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        });
    }
    code.push(Instruction::Return);
    finish_module(
        name,
        code,
        HashMap::new(),
        vec![entry],
        vec![(1, 0)],
        vec![LineInfo { file: 0, line: 301 }],
    )
}

/// Driver: `k` bytecode-`catch`-wrapped calls with a non-integer argument (each
/// raises badarg inside the trampoline's nested run and is caught by a handler
/// registered OUTSIDE it — the leak's precondition), then one uncaught call that
/// kills the process and produces the death record under assertion.
fn leak_driver_module(atoms: &AtomTable, names: &Names, entry: ResolvedImport, k: usize) -> Module {
    let name = atoms.intern("leakdrivermod");
    let mut code = vec![
        Instruction::Label { label: 1 },
        Instruction::Allocate {
            stack_need: Operand::Unsigned(1),
            live: Operand::Unsigned(0),
        },
    ];
    let mut line_table = vec![(1usize, 0usize)];
    let mut line_info = vec![LineInfo { file: 0, line: 501 }];
    for i in 0..k {
        let handler_label = u32::try_from(10 + i).expect("handler label fits");
        code.push(Instruction::Catch {
            destination: Operand::Y(0),
            label: Operand::Label(handler_label),
        });
        line_table.push((code.len(), line_info.len()));
        line_info.push(LineInfo {
            file: 0,
            line: u32::try_from(510 + i).expect("line fits"),
        });
        code.push(Instruction::Move {
            source: Operand::Atom(Some(names.bad)),
            destination: Operand::X(0),
        });
        code.push(Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        });
        code.push(Instruction::Label {
            label: handler_label,
        });
        code.push(Instruction::CatchEnd {
            source: Operand::Y(0),
        });
    }
    // The fatal, uncaught call.
    line_table.push((code.len(), line_info.len()));
    line_info.push(LineInfo { file: 0, line: 599 });
    code.push(Instruction::Move {
        source: Operand::Atom(Some(names.bad)),
        destination: Operand::X(0),
    });
    code.push(Instruction::CallExt {
        arity: Operand::Unsigned(1),
        import: Operand::Unsigned(0),
    });
    code.push(Instruction::Deallocate {
        words: Operand::Unsigned(1),
    });
    code.push(Instruction::Return);
    finish_module(
        name,
        code,
        HashMap::new(),
        vec![entry],
        line_table,
        line_info,
    )
}

/// Value-arrival driver: ONE `catch`-wrapped call with a non-integer argument,
/// then a normal return with x0 untouched since the catch — so the process's
/// exit result IS the value the handler received.
///
/// This is the liveness half of the fix under its own assertion. The nesting
/// floor stops a raise inside a nested run from jumping to a handler installed
/// outside it, which only helps if the exception then propagates OUT of the
/// compiled trampoline and reaches that handler with its value intact. A run
/// that killed the process here, or delivered something else, would still
/// satisfy every frame-count arm in this file.
fn catch_value_driver_module(atoms: &AtomTable, names: &Names, entry: ResolvedImport) -> Module {
    let name = atoms.intern("catchvaluedrivermod");
    let code = vec![
        Instruction::Label { label: 1 },
        Instruction::Allocate {
            stack_need: Operand::Unsigned(1),
            live: Operand::Unsigned(0),
        },
        Instruction::Catch {
            destination: Operand::Y(0),
            label: Operand::Label(10),
        },
        Instruction::Move {
            source: Operand::Atom(Some(names.bad)),
            destination: Operand::X(0),
        },
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
        Instruction::Label { label: 10 },
        Instruction::CatchEnd {
            source: Operand::Y(0),
        },
        Instruction::Deallocate {
            words: Operand::Unsigned(1),
        },
        Instruction::Return,
    ];
    finish_module(
        name,
        code,
        HashMap::new(),
        vec![entry],
        vec![(1, 0), (4, 1)],
        vec![
            LineInfo { file: 0, line: 701 },
            LineInfo { file: 0, line: 702 },
        ],
    )
}

struct Rig {
    scheduler: Scheduler,
    atoms: Arc<AtomTable>,
    names: Names,
    registry: Arc<ModuleRegistry>,
}

fn build_rig(threshold: u32) -> Rig {
    let atoms = Arc::new(AtomTable::with_common_atoms());
    let bifs = Arc::new(BifRegistryImpl::new());
    register_gate1_bifs(&bifs, &atoms).expect("gate1 BIFs register");
    register_stdlib_stubs(&bifs, &atoms).expect("stdlib stubs register");
    let names = Names {
        ffimod: atoms.intern("ffimod"),
        bif_int: atoms.intern("bif_int"),
        wrapmod: atoms.intern("wrapmod"),
        int_wrap: atoms.intern("int_wrap"),
        wrapmid: atoms.intern("wrapmid"),
        int_mid: atoms.intern("int_mid"),
        wrapouter: atoms.intern("wrapouter"),
        int_outer: atoms.intern("int_outer"),
        bad: atoms.intern("not_an_integer"),
    };
    let registry = Arc::new(ModuleRegistry::new());
    registry.insert(ffi_module(&atoms, &bifs, &names));
    registry.insert(wrap_module(&names));
    registry.insert(mid_module(&names));
    registry.insert(outer_module(&names));
    let scheduler = Scheduler::with_code_server(
        config(threshold),
        Arc::clone(&registry),
        Arc::clone(&atoms),
        Arc::clone(&bifs),
    )
    .expect("scheduler starts");
    Rig {
        scheduler,
        atoms,
        names,
        registry,
    }
}

/// Heat the wrapper. With `expect_compile`, assert it actually reached COMPILED
/// and that every submitted job settled — without this the arm would prove
/// nothing about compiled execution.
fn warm(rig: &Rig, expect_compile: bool) {
    warm_through(rig, wrap_import(&rig.names), THRESHOLD as usize, |rig| {
        if expect_compile {
            assert_compiled(rig, rig.names.wrapmod, rig.names.int_wrap);
        } else {
            assert_eq!(
                rig.scheduler
                    .jit_profiler()
                    .compile_outcome_counters()
                    .submissions,
                0,
                "the control arm must never submit a compile job"
            );
        }
    });
}

/// Assert `module:function/1` reached COMPILED within the wait budget, and that
/// every submitted compile job settled — without both, an arm downstream would
/// say nothing about compiled trampolines.
fn assert_compiled(rig: &Rig, module: Atom, function: Atom) {
    assert!(
        wait_until(|| rig
            .scheduler
            .jit_profiler()
            .is_compiled(module, function, 1)),
        "a trampoline this arm depends on never reached COMPILED within the wait budget"
    );
    assert!(
        wait_until(|| {
            let counters = rig.scheduler.jit_profiler().compile_outcome_counters();
            counters.submissions
                == counters.successes + counters.unsupported + counters.transient_failures
        }),
        "every submitted compile job must settle before the arm runs"
    );
}

/// Run `calls` warming calls through `entry` with a VALID argument (so warming
/// never raises), then check the arm's compile expectations.
fn warm_through(rig: &Rig, entry: ResolvedImport, calls: usize, check: impl FnOnce(&Rig)) {
    let module = rig.registry.insert(warm_module(&rig.atoms, entry, calls));
    let pid = rig.scheduler.spawn_process(&module);
    let (reason, _result) = rig.scheduler.run_until_exit(pid);
    assert_eq!(
        reason,
        ExitReason::Normal,
        "the warm loop must exit normally — a raise here means the harness is wrong, \
         not that the defect fired"
    );
    check(rig);
}

/// Run the driver to its fatal raise and tally the death record's frames by
/// `module:function/arity`. Only compiled pushes carry a resolved MFA, so a
/// trampoline's name appearing N times means N compiled-exception-arm
/// executions.
fn frame_tally(rig: &Rig, entry: ResolvedImport, k: usize) -> HashMap<String, usize> {
    let module = rig
        .registry
        .insert(leak_driver_module(&rig.atoms, &rig.names, entry, k));
    let pid = rig.scheduler.spawn_process(&module);
    let (reason, _result) = rig.scheduler.run_until_exit(pid);
    assert_eq!(
        reason,
        ExitReason::Error,
        "the final uncaught badarg must kill the process"
    );

    let exception = rig
        .scheduler
        .take_exit_exception(pid)
        .expect("the death must leave a readable exception record");
    let frames = exception.frames();
    assert!(
        !frames.is_empty(),
        "death record has ZERO frames — the reader is broken and any count from it is void"
    );
    // Falsifier rail: the genuine interpreted raiser carries line info. If no
    // frame does, this reader cannot tell compiled pushes from interpreted
    // frames, and a zero compiled-frame count would be an artefact, not a pass.
    assert!(
        frames.iter().any(|frame| frame.line.is_some()),
        "no line-bearing interpreted frame in the record — the instrument cannot \
         distinguish frame kinds, so its counts prove nothing:\n{}",
        exception.format_with_atoms(&rig.atoms)
    );

    let mut tally = HashMap::new();
    for frame in frames {
        *tally
            .entry(format!(
                "{}:{}/{}",
                frame.module, frame.function, frame.arity
            ))
            .or_insert(0) += 1;
    }
    tally
}

/// Compiled-frame count for one trampoline identity in a death record.
fn compiled_frames_in_death_record(rig: &Rig, k: usize) -> usize {
    frame_tally(rig, wrap_import(&rig.names), k)
        .get("wrapmod:int_wrap/1")
        .copied()
        .unwrap_or(0)
}

/// Arm 1 (RED while the defect is live): six caught badargs through the compiled
/// trampoline, then one fatal — the death record must name the trampoline ONCE.
#[test]
fn compiled_trampoline_records_one_frame_after_caught_exceptions() {
    let rig = build_rig(THRESHOLD);
    warm(&rig, true);
    let compiled_frames = compiled_frames_in_death_record(&rig, 6);
    assert_eq!(
        compiled_frames, 1,
        "one raise through one compiled trampoline must record exactly one compiled \
         frame; {compiled_frames} means each caught exception leaked a nested run \
         whose trampoline also fired its exception arm on the fatal unwind"
    );
    rig.scheduler.shutdown();
}

/// Arm 2 (RED while the defect is live): the count must not track the number of
/// exceptions caught earlier. A partial fix that merely reduces the count still
/// fails this.
#[test]
fn ghost_frame_count_is_independent_of_catch_count() {
    let few = {
        let rig = build_rig(THRESHOLD);
        warm(&rig, true);
        let count = compiled_frames_in_death_record(&rig, 2);
        rig.scheduler.shutdown();
        count
    };
    let many = {
        let rig = build_rig(THRESHOLD);
        warm(&rig, true);
        let count = compiled_frames_in_death_record(&rig, 6);
        rig.scheduler.shutdown();
        count
    };
    assert_eq!(
        few, many,
        "compiled-frame count must be independent of how many exceptions were \
         caught first; {few} at K=2 vs {many} at K=6 means the count tracks K, \
         i.e. one nested run leaks per caught exception"
    );
}

/// Arm 4: two nestings deep — a compiled trampoline whose body call enters an
/// interpreted middle, which body-calls a second compiled trampoline, which
/// raises. The catch is at the OUTERMOST level, so the exception must travel out
/// through BOTH nestings. Each trampoline records exactly one frame, at both
/// catch counts: the per-trampoline count is a property of the propagation, not
/// of nesting depth or of how many exceptions were caught first.
#[test]
fn each_trampoline_records_one_frame_at_nesting_depth_two() {
    let mut tallies = Vec::new();
    for k in [2usize, 6] {
        let rig = build_rig(THRESHOLD);
        // Warming through the outer entry heats the outer trampoline on the
        // driver's call edge and the inner one on the middle's — the middle is
        // only ever entered by `jit_call_interpreted`, which records no miss.
        warm_through(
            &rig,
            outer_import(&rig.names),
            THRESHOLD as usize * 3,
            |rig| {
                assert_compiled(rig, rig.names.wrapouter, rig.names.int_outer);
                assert_compiled(rig, rig.names.wrapmod, rig.names.int_wrap);
            },
        );
        let tally = frame_tally(&rig, outer_import(&rig.names), k);
        rig.scheduler.shutdown();
        tallies.push((k, tally));
    }

    for (k, tally) in &tallies {
        for identity in ["wrapouter:int_outer/1", "wrapmod:int_wrap/1"] {
            assert_eq!(
                tally.get(identity).copied().unwrap_or(0),
                1,
                "at K={k}, {identity} must appear exactly once in the death record; \
                 a higher count means nestings leaked at depth two, a zero means the \
                 propagation never crossed that trampoline: {tally:?}"
            );
        }
        // The middle DOES tier up — it accrues call misses on the driver's
        // pre-tier-up calls, while the outer is still running as bytecode. Its
        // compiled code is nonetheless never entered here: once the outer is
        // compiled, `jit_call_interpreted` runs the middle's BYTECODE in the
        // nested run. Zero frames for it is that fact measured on execution
        // rather than assumed from compile state, and it is what makes the two
        // trampolines above two SEPARATE nestings.
        assert_eq!(
            tally.get("wrapmid:int_mid/1").copied().unwrap_or(0),
            0,
            "at K={k}, the middle's compiled code must never run in this chain — a \
             frame for it means the nesting structure is not the one under test: \
             {tally:?}"
        );
    }
}

/// Arm 5: the liveness half. An exception raised two frames inside a compiled
/// trampoline's nested run must still be CAUGHT by the outer handler, and the
/// value the handler receives must be the same one the interpreted path
/// delivers. Refusing the outer handler inside the nest (arms 1-4) is only
/// correct if the exception then leaves through the trampoline and arrives —
/// a fix that made this exception uncaught would satisfy every frame count in
/// this file and kill a process the catch should have saved.
#[test]
fn caught_exception_value_reaches_the_handler_across_a_compiled_trampoline() {
    fn caught_value_shape(threshold: u32) -> (ExitReason, String) {
        let rig = build_rig(threshold);
        if threshold == NEVER_COMPILE {
            warm(&rig, false);
        } else {
            warm(&rig, true);
        }
        let module = rig.registry.insert(catch_value_driver_module(
            &rig.atoms,
            &rig.names,
            wrap_import(&rig.names),
        ));
        let pid = rig.scheduler.spawn_process(&module);
        let (reason, result) = rig.scheduler.run_until_exit(pid);
        let shape = describe_caught(&rig.atoms, result.root());
        rig.scheduler.shutdown();
        (reason, shape)
    }

    let (compiled_reason, compiled_shape) = caught_value_shape(THRESHOLD);
    let (interpreted_reason, interpreted_shape) = caught_value_shape(NEVER_COMPILE);

    assert_eq!(
        compiled_reason,
        ExitReason::Normal,
        "the badarg raised inside the compiled trampoline's nested run must be CAUGHT \
         and the process must survive to return; got {compiled_reason:?} with \
         {compiled_shape}"
    );
    assert_eq!(
        interpreted_reason,
        ExitReason::Normal,
        "control: the same program with nothing compiled must also survive"
    );
    assert_eq!(
        compiled_shape, "{'EXIT', {badarg, _}}",
        "the handler must receive the badarg the callee raised, shaped as `catch` \
         builds it; got {compiled_shape}"
    );
    assert_eq!(
        compiled_shape, interpreted_shape,
        "the value delivered across a compiled trampoline must be the one the \
         interpreted path delivers"
    );
}

/// Render a caught value as `{'EXIT', {reason, _}}` — the class/reason atoms
/// exactly, the stacktrace tail elided because it legitimately differs between
/// the compiled and interpreted paths (the compiled one carries the trampoline
/// frame this file's other arms count).
fn describe_caught(atoms: &AtomTable, term: Term) -> String {
    let Some(outer) = Tuple::new(term) else {
        return format!("not a tuple: {term:?}");
    };
    if outer.arity() != 2 {
        return format!("tuple of arity {}", outer.arity());
    }
    let tag = outer
        .get(0)
        .and_then(|element| element.as_atom())
        .and_then(|atom| atoms.resolve(atom))
        .unwrap_or("?");
    let Some(inner) = outer.get(1).and_then(Tuple::new) else {
        return format!("{{'{tag}', not-a-tuple}}");
    };
    if inner.arity() != 2 {
        return format!("{{'{tag}', tuple of arity {}}}", inner.arity());
    }
    let reason = inner
        .get(0)
        .and_then(|element| element.as_atom())
        .and_then(|atom| atoms.resolve(atom))
        .unwrap_or("?");
    format!("{{'{tag}', {{{reason}, _}}}}")
}

/// Arm 3 (GREEN control): the identical program with nothing compiled records no
/// compiled frames at all — the harness is sound and the divergence belongs to
/// compiled execution.
#[test]
fn interpreted_catches_record_no_compiled_frames_control() {
    let rig = build_rig(NEVER_COMPILE);
    warm(&rig, false);
    let compiled_frames = compiled_frames_in_death_record(&rig, 6);
    assert_eq!(
        compiled_frames, 0,
        "with nothing compiled there is no trampoline exception arm to push a \
         frame; {compiled_frames} would mean the counter matches something else"
    );
    rig.scheduler.shutdown();
}
