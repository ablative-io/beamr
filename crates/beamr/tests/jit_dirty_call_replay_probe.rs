//! Does a DIRTY CALL raised inside a JIT-compiled caller's nested run get the
//! caller's prefix replayed — submitting the dirty work twice?
//!
//! WHY THIS ARM EXISTS
//! -------------------
//! `DirtyCall` was fixed at the same moment and by the same code path as
//! `Waiting` (aion#85): both are scheduler-level TRANSFERS, and both now leave
//! the nested-run helpers before the module/position restore. But `Waiting` was
//! calibrated red-then-green at both helpers and **`DirtyCall` never was** — it
//! rode out on an argument from symmetry. Symmetry is an argument, not a
//! measurement, and every cell in this investigation that was assumed rather
//! than measured has cost something.
//!
//! The defect it must reproduce: `jit_call_interpreted` restored the caller's
//! saved code position — which is the compiled function's ENTRY — over the
//! position the nested run had reached, then reported the transfer as a deopt.
//! A deopt means "re-interpret this function from its entry", so the compiled
//! prefix runs again and its tail external call is re-executed. For a dirty
//! call that means **the dirty work is submitted twice**.
//!
//! SHAPE
//! -----
//! ```text
//! driver/1 : mints the closure on the Closure edge, then tail-calls outer/N
//! outer/N  : CallExt -> inner/1  (External)   THE JIT-COMPILED FUNCTION
//!            CallFun -> inner/1  (Closure)    its only edge either way
//! inner/1  : CallExt -> mark/1   (pure native, runs on the scheduler thread)
//!            CallExt -> work/1   (DIRTY native, runs on the dirty CPU pool)
//!            Return
//! ```
//!
//! `inner/1` is POISONED against the tier (`SelectTupleArity` with no
//! candidates), so the nested run really is an interpreted one.
//!
//! `mark/1` returns the TAG so the tag reaches `work/1`; `work/1` returns the
//! shared constant, so both arms exit with the same value and the exits are
//! comparable. (An earlier probe in this family keyed its counter on a tag that
//! the native's own return value had already clobbered in x0, so the replayed
//! call incremented a different bucket and the arm reported a confident false
//! green. The tag is threaded deliberately.)
//!
//! WHAT THE PRE-FIX FACE ACTUALLY IS — measured, and not what the name says
//! ----------------------------------------------------------------------
//! "Double-submit" was the wrong guess. Against the unfixed helpers the
//! transfer is **laundered into a deopt**, so the dirty work is never submitted
//! on the first pass at all; it is submitted once, from the replay. So
//! **`work/1`'s count stays at 1 and `mark/1`'s doubles.**
//!
//! Both counts are therefore asserted on every arm. An arm that watched only
//! the effect it expected to double would read `1 == 1` and pass over a live
//! defect — and an earlier revision of the closure arm here was missing exactly
//! that assertion while its sibling had it, so a calibration run that
//! reproduced the bug on the closure path still reported GREEN.
//!
//! CONTROLS
//! --------
//! * The interpreted arm is the reference: each native runs exactly once.
//! * The JIT arm is witnessed POSITIVELY — `outer/N` must be in the `JitCache`
//!   and `inner/1` must NOT be, or the arm ran interpreted and proves nothing.
//! * **The dirty dispatch is witnessed too.** `native_call.rs` returns
//!   `InstructionOutcome::DirtyCall` the moment a native declares a
//!   `dirty_kind`, and the dirty pool runs it on its own thread. So `work/1`
//!   recording a DIFFERENT thread id from `mark/1` is a direct observation that
//!   the transfer actually happened. Without it, a green would be consistent
//!   with "the dirty path never fired", which is the failure mode this file's
//!   siblings have already been bitten by twice.
//!
//! CALIBRATION
//! -----------
//! Dropping `| ExecutionResult::DirtyCall { .. }` from the transfer arm of BOTH
//! helpers (`jit/runtime.rs`, `jit/runtime_closure.rs`) and leaving everything
//! else alone puts each JIT arm RED and leaves the interpreted control GREEN:
//!
//! ```text
//!   control_interpreted_dirty_call_submits_the_work_once        ok
//!   jit_dirty_call_does_not_resubmit_the_compiled_prefix        FAILED  mark 2 != 1
//!   jit_closure_dirty_call_does_not_resubmit_the_compiled_prefix FAILED mark 2 != 1
//! ```
//!
//! Each edge is therefore calibrated at its OWN site, not inherited from its
//! sibling: the external arm proves `jit_call_interpreted`, the closure arm
//! proves `jit_dispatch_closure`. Restoring the arm returns all three to green
//! with the thread witness live on every one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use beamr::atom::{Atom, AtomTable};
use beamr::loader::decode::compact::Operand;
use beamr::loader::{Instruction, LambdaEntry, lambda_unique_id};
use beamr::module::{Module, ModuleOrigin, ModuleRegistry, ResolvedImport, ResolvedImportTarget};
use beamr::native::{Capability, NativeEntry, ProcessContext};
use beamr::process::ExitReason;
use beamr::scheduler::{DirtySchedulerKind, NativeBifs, Scheduler, SchedulerConfig};
use beamr::term::Term;

const WAIT_BUDGET: Duration = Duration::from_secs(20);
const HEAT_TAG: i64 = 0;
const RESULT: i64 = 4242;

/// Per-tag record of what each native observed.
#[derive(Clone, Copy, Debug, Default)]
struct Observed {
    mark_runs: usize,
    work_runs: usize,
    mark_thread: Option<ThreadId>,
    work_thread: Option<ThreadId>,
}

fn observed() -> &'static Mutex<HashMap<i64, Observed>> {
    static OBSERVED: OnceLock<Mutex<HashMap<i64, Observed>>> = OnceLock::new();
    OBSERVED.get_or_init(|| Mutex::new(HashMap::new()))
}

fn observed_for(tag: i64) -> Observed {
    observed()
        .lock()
        .expect("observed")
        .get(&tag)
        .copied()
        .unwrap_or_default()
}

fn tag_of(args: &[Term]) -> Result<i64, Term> {
    args.first()
        .and_then(|term| term.as_small_int())
        .ok_or_else(|| Term::atom(Atom::BADARG))
}

/// `jit_dirty_probe:mark/1` — a PURE native, so it runs on the scheduler
/// thread. Returns the tag so the burn of the tag through x0 is deliberate.
fn mark(args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    let tag = tag_of(args)?;
    let mut guard = observed().lock().expect("observed");
    let entry = guard.entry(tag).or_default();
    entry.mark_runs += 1;
    entry.mark_thread = Some(std::thread::current().id());
    Ok(Term::small_int(tag))
}

/// `jit_dirty_probe:work/1` — the DIRTY native. Declaring a `dirty_kind` is
/// what makes `native_call.rs` return `InstructionOutcome::DirtyCall` instead
/// of running it inline, which is the transfer this probe is about.
fn work(args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    let tag = tag_of(args)?;
    let mut guard = observed().lock().expect("observed");
    let entry = guard.entry(tag).or_default();
    entry.work_runs += 1;
    entry.work_thread = Some(std::thread::current().id());
    Ok(Term::small_int(RESULT))
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

struct Names {
    module: Atom,
    driver: Atom,
    outer: Atom,
    inner: Atom,
    mark: Atom,
    work: Atom,
}

fn names(atoms: &AtomTable) -> Names {
    Names {
        module: atoms.intern("jit_dirty_probe"),
        driver: atoms.intern("driver"),
        outer: atoms.intern("outer"),
        inner: atoms.intern("inner"),
        mark: atoms.intern("mark"),
        work: atoms.intern("work"),
    }
}

/// Which helper the compiled caller's only edge goes through.
///
/// `CallExt` lowers to `jit_call_interpreted` (`jit/runtime.rs`); `CallFun`
/// lowers to `jit_dispatch_closure` (`jit/runtime_closure.rs`). Both carried
/// the same unconditional restore and both took the same fix, so an arm that
/// drives only one of them reports GREEN over the other.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum OuterEdge {
    External,
    Closure,
}

/// On the closure edge the closure is minted by `driver` and PASSED IN — a
/// function containing `make_fun` can never be JIT-compiled at runtime
/// (beamr#28), so an `outer` that minted its own could never satisfy the
/// compiled-caller witness.
const fn outer_arity(edge: OuterEdge) -> u8 {
    match edge {
        OuterEdge::External => 1,
        OuterEdge::Closure => 2,
    }
}

fn probe_module(names: &Names, atoms: &AtomTable, edge: OuterEdge) -> Module {
    let imports = vec![
        ResolvedImport {
            module: names.module,
            function: names.outer,
            arity: outer_arity(edge),
            target: ResolvedImportTarget::Code {
                module: names.module,
                label: 2,
            },
        },
        ResolvedImport {
            module: names.module,
            function: names.inner,
            arity: 1,
            target: ResolvedImportTarget::Code {
                module: names.module,
                label: 3,
            },
        },
        ResolvedImport {
            module: names.module,
            function: names.mark,
            arity: 1,
            target: ResolvedImportTarget::Native(NativeEntry {
                function: mark,
                dirty_kind: None,
                capability: Capability::Pure,
            }),
        },
        ResolvedImport {
            module: names.module,
            function: names.work,
            arity: 1,
            target: ResolvedImportTarget::Native(NativeEntry {
                function: work,
                dirty_kind: Some(DirtySchedulerKind::Cpu),
                capability: Capability::Pure,
            }),
        },
    ];
    let mut exports = HashMap::new();
    exports.insert((names.driver, 1), 1);
    exports.insert((names.outer, outer_arity(edge)), 2);
    exports.insert((names.inner, 1), 3);

    let mut code = vec![
        Instruction::FuncInfo {
            module: Operand::Atom(Some(names.module)),
            function: Operand::Atom(Some(names.driver)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 1 },
    ];
    if edge == OuterEdge::Closure {
        code.push(Instruction::Move {
            source: Operand::X(0),
            destination: Operand::X(2),
        });
        code.push(Instruction::MakeFun {
            operands: vec![Operand::Unsigned(0)],
        });
        code.push(Instruction::Move {
            source: Operand::X(0),
            destination: Operand::X(1),
        });
        code.push(Instruction::Move {
            source: Operand::X(2),
            destination: Operand::X(0),
        });
    }
    code.push(Instruction::CallExtOnly {
        arity: Operand::Unsigned(u64::from(outer_arity(edge))),
        import: Operand::Unsigned(0),
    });
    code.extend([
        // outer/N — THE COMPILED FUNCTION. Its only edge is a tail call into
        // the interpreted callee, through whichever helper `edge` selects.
        Instruction::FuncInfo {
            module: Operand::Atom(Some(names.module)),
            function: Operand::Atom(Some(names.outer)),
            arity: Operand::Unsigned(u64::from(outer_arity(edge))),
        },
        Instruction::Label { label: 2 },
    ]);
    code.push(match edge {
        OuterEdge::External => Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(1),
        },
        OuterEdge::Closure => Instruction::CallFun {
            arity: Operand::Unsigned(1),
        },
    });
    code.extend([
        Instruction::Return,
        // inner/1 — interpreted (poisoned against the tier).
        Instruction::FuncInfo {
            module: Operand::Atom(Some(names.module)),
            function: Operand::Atom(Some(names.inner)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 3 },
        Instruction::SelectTupleArity {
            value: Operand::X(0),
            fail: Operand::Label(4),
            list: Operand::List(vec![]),
        },
        Instruction::Label { label: 4 },
        // The scheduler-thread marker, so the dirty dispatch has something to
        // be compared against.
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(2),
        },
        // THE DIRTY CALL. Raises the transfer from inside the nested run.
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(3),
        },
        Instruction::Return,
    ]);

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
        name: names.module,
        generation: 0,
        origin: ModuleOrigin::Preloaded,
        exports,
        label_index,
        code,
        function_table,
        line_table: Vec::new(),
        literals: Vec::new(),
        constant_pool: Default::default(),
        resolved_imports: imports,
        lambdas: vec![LambdaEntry {
            function: names.inner,
            arity: 1,
            label: 3,
            num_free: 0,
            unique_id: lambda_unique_id(atoms, names.module, names.inner, 1, 0)
                .expect("inner/1 lambda id"),
        }],
        string_table: Vec::new(),
        line_info: Vec::new(),
    }
}

#[derive(Debug)]
struct ArmOutcome {
    outer_compiled: bool,
    inner_compiled: bool,
    observed: Observed,
    exit: Option<(ExitReason, Term)>,
    exit_error: Option<String>,
}

fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + WAIT_BUDGET;
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn run_arm_with(tag: i64, threshold: u32, heat_drives: usize, edge: OuterEdge) -> ArmOutcome {
    assert_ne!(tag, HEAT_TAG, "the drive tag must not collide with heating");
    let atoms = AtomTable::with_common_atoms();
    let names = names(&atoms);

    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(probe_module(&names, &atoms, edge));
    let generation = module.generation();

    let scheduler = Scheduler::new(config(threshold), Arc::clone(&registry), NativeBifs::none())
        .expect("scheduler starts");

    for _ in 0..heat_drives {
        let pid = scheduler
            .spawn(names.module, names.driver, vec![Term::small_int(HEAT_TAG)])
            .expect("spawn heat drive");
        let (reason, _value) = scheduler.run_until_exit(pid);
        assert_eq!(
            reason,
            ExitReason::Normal,
            "heat drive must exit normally (error: {:?})",
            scheduler.take_exit_error(pid)
        );
    }

    let _reached = wait_until(|| {
        scheduler
            .jit_cache()
            .lookup(names.module, names.outer, outer_arity(edge), generation)
            .is_some()
    });
    let outer_compiled = scheduler
        .jit_cache()
        .lookup(names.module, names.outer, outer_arity(edge), generation)
        .is_some();
    let inner_compiled = scheduler
        .jit_cache()
        .lookup(names.module, names.inner, 1, generation)
        .is_some();

    let pid = scheduler
        .spawn(names.module, names.driver, vec![Term::small_int(tag)])
        .expect("spawn drive");
    let (reason, value) = scheduler.run_until_exit(pid);
    let exit_error = scheduler.take_exit_error(pid).map(|error| error.to_string());
    let observed = observed_for(tag);
    scheduler.shutdown();

    ArmOutcome {
        outer_compiled,
        inner_compiled,
        observed,
        exit: Some((reason, value.root())),
        exit_error,
    }
}

fn report(label: &str, outcome: &ArmOutcome) {
    println!("--- {label} ---");
    println!("  outer/N in JitCache  : {}", outcome.outer_compiled);
    println!("  inner/1 in JitCache  : {}", outcome.inner_compiled);
    println!("  mark/1 runs          : {}", outcome.observed.mark_runs);
    println!(
        "  work/1 runs (DIRTY)  : {}   <- a replay submits this twice",
        outcome.observed.work_runs
    );
    println!(
        "  mark thread          : {:?}",
        outcome.observed.mark_thread
    );
    println!(
        "  work thread          : {:?}   <- DIRTY WITNESS: must differ from mark's",
        outcome.observed.work_thread
    );
    match &outcome.exit {
        Some((reason, value)) => println!("  exit                 : {reason:?} value={value:?}"),
        None => println!("  exit                 : TIMED OUT"),
    }
    println!("  exit error           : {:?}", outcome.exit_error);
}

/// The instrument's own positive control: prove the dirty path actually fired.
///
/// Without this, a green is equally consistent with "the dirty dispatch never
/// happened", and the arm would be measuring nothing while looking healthy.
fn assert_dirty_dispatch_witnessed(label: &str, outcome: &ArmOutcome) {
    let mark_thread = outcome
        .observed
        .mark_thread
        .unwrap_or_else(|| panic!("{label}: mark/1 never ran, so the arm never reached the callee"));
    let work_thread = outcome
        .observed
        .work_thread
        .unwrap_or_else(|| panic!("{label}: work/1 never ran, so no dirty call was ever made"));
    assert_ne!(
        mark_thread, work_thread,
        "{label}: THE INSTRUMENT NEVER FIRED. work/1 ran on the same thread as \
         mark/1, so it was executed inline rather than dispatched to the dirty \
         pool — no `DirtyCall` transfer left the nested run, and this arm says \
         NOTHING about the transfer path. Its run counts are not evidence."
    );
}

/// POSITIVE CONTROL — interpreted, each native runs exactly once.
#[test]
fn control_interpreted_dirty_call_submits_the_work_once() {
    let outcome = run_arm_with(901, 1_000_000, 3, OuterEdge::External);
    report("CONTROL (interpreter only)", &outcome);

    assert!(
        !outcome.outer_compiled && !outcome.inner_compiled,
        "control arm must never compile anything"
    );
    assert_dirty_dispatch_witnessed("CONTROL", &outcome);
    assert_eq!(
        outcome.observed.work_runs, 1,
        "CONTROL FAILED: interpreted, the dirty work must be submitted exactly once"
    );
    assert_eq!(
        outcome.observed.mark_runs, 1,
        "CONTROL FAILED: interpreted, the callee prefix must run exactly once"
    );
    assert_eq!(
        outcome.exit.as_ref().map(|(reason, _)| reason),
        Some(&ExitReason::Normal),
        "control arm must exit normally (error: {:?})",
        outcome.exit_error
    );
}

/// THE ARM — `outer/N` compiled, external-call edge.
#[test]
fn jit_dirty_call_does_not_resubmit_the_compiled_prefix() {
    let control = run_arm_with(902, 1_000_000, 3, OuterEdge::External);
    report("CONTROL (interpreter only)", &control);
    assert_dirty_dispatch_witnessed("CONTROL", &control);
    assert_eq!(
        control.observed.work_runs, 1,
        "the control must be sound before the JIT arm is admissible"
    );

    let jit = run_arm_with(903, 2, 3, OuterEdge::External);
    report("JIT ARM (outer/1 compiled)", &jit);
    assert_dirty_dispatch_witnessed("JIT ARM", &jit);
    assert!(
        jit.outer_compiled,
        "UNWITNESSED JIT ARM: outer/1 never entered the JitCache, so this arm ran \
         interpreted and any green below is a FALSE GREEN"
    );
    assert!(
        !jit.inner_compiled,
        "inner/1 must stay interpreted, or the nested run is not the interpreted \
         run this probe is about"
    );

    assert_eq!(
        jit.observed.work_runs, control.observed.work_runs,
        "DIRTY CALL DOUBLE-SUBMITTED: the dirty native ran {} time(s) under the \
         JIT and {} time(s) interpreted. The helper restored the compiled \
         caller's entry over the nested run's position and reported the transfer \
         as a deopt, so the compiled prefix was re-executed and the dirty work \
         was submitted again.",
        jit.observed.work_runs, control.observed.work_runs
    );
    assert_eq!(
        jit.observed.mark_runs, control.observed.mark_runs,
        "the callee prefix ran a different number of times under the JIT"
    );
    assert_eq!(
        jit.exit, control.exit,
        "a dirty call under the JIT must be observably equal to the interpreted \
         one (exit error: {:?})",
        jit.exit_error
    );
}

/// THE SAME QUESTION AT THE CLOSURE HELPER.
///
/// `jit_dispatch_closure` carried the identical restore and took the identical
/// fix. Proving one site says nothing about the other — that is the mistake
/// this whole investigation keeps having to re-learn.
#[test]
fn jit_closure_dirty_call_does_not_resubmit_the_compiled_prefix() {
    let control = run_arm_with(911, 1_000_000, 3, OuterEdge::Closure);
    report("CONTROL (interpreter only, closure edge)", &control);
    assert_dirty_dispatch_witnessed("CONTROL closure", &control);
    assert_eq!(
        control.observed.work_runs, 1,
        "the control must be sound before the JIT arm is admissible"
    );

    let jit = run_arm_with(912, 2, 3, OuterEdge::Closure);
    report("JIT ARM (outer/2 compiled, closure edge)", &jit);
    assert_dirty_dispatch_witnessed("JIT ARM closure", &jit);
    assert!(
        jit.outer_compiled,
        "UNWITNESSED JIT ARM: outer/2 never entered the JitCache. If the tier \
         refused it, check that no `make_fun` reached the compiled function \
         (beamr#28) — `driver` mints the closure precisely so `outer/2` does not."
    );
    assert!(
        !jit.inner_compiled,
        "inner/1 must stay interpreted, or the nested run is not the interpreted \
         run this probe is about"
    );

    assert_eq!(
        jit.observed.work_runs, control.observed.work_runs,
        "DIRTY CALL DOUBLE-SUBMITTED ON THE CLOSURE PATH: the dirty native ran \
         {} time(s) under the JIT and {} time(s) interpreted.",
        jit.observed.work_runs, control.observed.work_runs
    );
    // THE ASSERTION THAT ACTUALLY CATCHES THIS DEFECT, and it was missing from
    // this arm while its sibling had it — which is how a calibration run that
    // reproduced the bug on this very path still reported GREEN.
    //
    // The pre-fix face is NOT the one the name suggests: the transfer is
    // laundered into a deopt, so the dirty work is never submitted on the first
    // pass at all. It is submitted once, from the replay. The dirty count stays
    // at 1 and the PREFIX count doubles. An arm that watches only the effect it
    // expected to double reads 1 == 1 and passes over a live defect.
    assert_eq!(
        jit.observed.mark_runs, control.observed.mark_runs,
        "COMPILED PREFIX REPLAYED ON THE CLOSURE PATH: the callee prefix ran {} \
         time(s) under the JIT and {} time(s) interpreted. \
         `jit_dispatch_closure` restored the compiled caller's entry over the \
         nested run's position and reported the transfer as a deopt, so the \
         prefix was re-executed.",
        jit.observed.mark_runs, control.observed.mark_runs
    );
    assert_eq!(
        jit.exit, control.exit,
        "a dirty call under a compiled closure caller must be observably equal \
         to the interpreted one (exit error: {:?})",
        jit.exit_error
    );
}
