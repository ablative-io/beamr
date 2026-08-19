//! RED-FIRST PROBE — does a MID-BODY YIELD inside a JIT-compiled caller replay
//! the caller's prefix, the way a suspension did (aion#85)?
//!
//! HYPOTHESIS UNDER TEST
//! ---------------------
//! `jit_call_interpreted` restores the caller's saved code position
//! unconditionally after the nested interpreted run returns. The suspension fix
//! made `Waiting` and `DirtyCall` skip that restore, because their resume
//! position is the one that matters. **`Yielded` was deliberately left on the
//! old path**, as a hypothesis with a named discriminator rather than a change
//! made on suspicion. This is that discriminator.
//!
//! The suspicion, stated so it can be refuted: `Ok(ExecutionResult::Yielded)` is
//! reached only AFTER the nested run has executed. The interpreter sets the
//! resume position immediately before yielding (`jump_position_with_reduction`
//! writes the target, then returns `Yield`), and the callers of `invoke_jit` set
//! the process's position to the compiled function's ENTRY before entering
//! compiled code. So the helper's restore should overwrite a mid-callee resume
//! position with the compiled entry, and the rescheduled process should re-enter
//! the compiled function from the top — re-executing its tail external call.
//!
//! If that is right, an observable effect performed by the callee BEFORE it
//! yields happens more than once under the JIT and exactly once interpreted.
//! A yield is not a deopt, so the deopt-after-side-effect admission guard says
//! nothing about this path.
//!
//! SHAPE
//! -----
//! ```text
//! driver/1 : CallExtOnly -> outer/1
//! outer/1  : CallExt     -> inner/1 ; Return      (THE JIT-COMPILED FUNCTION)
//! inner/1  : CallExt     -> count/1               (THE OBSERVABLE EFFECT)
//!            then a chain of local tail calls long enough to exhaust the
//!            slice's reduction budget, so the yield lands AFTER the effect
//!            and INSIDE the nested run ; Return
//! count/1 (native) : increments a per-tag counter and returns. Never parks —
//!            this probe is about yields, and a park would confound it with the
//!            suspension defect that is already fixed.
//! ```
//!
//! Reductions are charged per call/jump-with-reduction, not per instruction
//! (`core.rs::jump_position_with_reduction`), and a bare `Jump` does NOT charge
//! one — hence a chain of `CallOnly` local tail calls rather than a loop, which
//! would need arithmetic and a comparison to terminate.
//!
//! `inner/1` is deliberately POISONED against the tier (the same
//! `SelectTupleArity`-with-no-candidates representative the other probes use).
//! If `inner` compiled, the nested run would not be the interpreted run this
//! probe is about.
//!
//! CONTROLS
//! --------
//! * The interpreted arm is the reference: the effect must happen exactly once.
//! * The JIT arm is witnessed POSITIVELY — `outer/1` must be in the `JitCache`
//!   before the drive, or its result is inadmissible either way.
//! * The yield must be witnessed too: if the burn chain never exhausts a slice,
//!   nothing yields and BOTH arms trivially show one effect. That green would
//!   mean "the instrument never fired". So `slice/1` sits at the far end of the
//!   chain and reads the process's reduction counter: `decrement_reductions`
//!   SATURATES at zero, so `BURN_STEPS` (> the 4000 budget) charged steps run
//!   inside one un-rescheduled slice MUST arrive with the counter at exactly
//!   zero. A non-zero counter can only come from `reset_reductions`, which only
//!   runs when the process is rescheduled. Both arms assert it. This is a
//!   direct observation of the yield rather than an inference from arithmetic
//!   — an earlier revision of this probe promised the witness in this comment
//!   and did not implement it, and its green was worth nothing until it did.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use beamr::atom::{Atom, AtomTable};
use beamr::loader::{Instruction, LambdaEntry, lambda_unique_id};
use beamr::loader::decode::compact::Operand;
use beamr::module::{Module, ModuleOrigin, ModuleRegistry, ResolvedImport, ResolvedImportTarget};
use beamr::native::{Capability, NativeEntry, ProcessContext};
use beamr::process::ExitReason;
use beamr::scheduler::{NativeBifs, Scheduler, SchedulerConfig};
use beamr::term::Term;

const WAIT_BUDGET: Duration = Duration::from_secs(20);
/// The per-slice reduction budget is 4000 (`process::DEFAULT_REDUCTION_BUDGET`).
/// The chain is sized well past it so the yield is guaranteed to land mid-chain
/// — i.e. after the effect and inside the nested run.
const BURN_STEPS: u32 = 6000;
/// `process::DEFAULT_REDUCTION_BUDGET`, restated here because the probe's
/// arithmetic depends on it: `BURN_STEPS` must exceed it for the yield witness
/// to mean anything.
const BURN_BUDGET: u32 = 4000;
/// First label of the burn chain. Kept clear of the function labels 1-4.
const BURN_LABEL_BASE: u32 = 100;
const HEAT_TAG: i64 = 0;
const RESULT: i64 = 31337;

fn counts() -> &'static Mutex<HashMap<i64, usize>> {
    static COUNTS: OnceLock<Mutex<HashMap<i64, usize>>> = OnceLock::new();
    COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Reduction counter observed at the two ends of the burn chain, per tag:
/// `(at the effect, at the end of the chain)`. This is the YIELD WITNESS — see
/// `reductions_at_end` in `ArmOutcome`.
fn budgets() -> &'static Mutex<HashMap<i64, (u32, u32)>> {
    static BUDGETS: OnceLock<Mutex<HashMap<i64, (u32, u32)>>> = OnceLock::new();
    BUDGETS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn count_for(tag: i64) -> usize {
    counts().lock().expect("counts").get(&tag).copied().unwrap_or(0)
}

fn budget_for(tag: i64) -> (u32, u32) {
    budgets().lock().expect("budgets").get(&tag).copied().unwrap_or((0, 0))
}

/// `jit_yield_probe:count/1` — the observable effect. Never parks.
///
/// Returns the TAG rather than a constant so the burn chain carries it to
/// `slice/1` at the far end; `slice/1` supplies the constant the process exits
/// with, so both arms still exit with the same value.
fn count(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let tag = args
        .first()
        .and_then(|term| term.as_small_int())
        .ok_or_else(|| Term::atom(Atom::BADARG))?;
    *counts().lock().expect("counts").entry(tag).or_insert(0) += 1;
    let remaining = context
        .process_mut()
        .map_or(0, |process| process.reduction_counter());
    budgets().lock().expect("budgets").entry(tag).or_insert((0, 0)).0 = remaining;
    Ok(Term::small_int(tag))
}

/// `jit_yield_probe:slice/1` — THE YIELD WITNESS, at the far end of the burn
/// chain. Records the reduction counter that survived the chain.
///
/// The budget is `DEFAULT_REDUCTION_BUDGET` = 4000 and `decrement_reductions`
/// SATURATES at zero, so a chain of `BURN_STEPS` > 4000 charged steps that ran
/// without ever being rescheduled must arrive here with the counter at exactly
/// zero. A NON-ZERO counter here can only mean `reset_reductions` ran — i.e.
/// the process yielded and was given a fresh slice. That is a direct
/// observation of the yield, not an inference from the chain length.
fn slice(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let tag = args
        .first()
        .and_then(|term| term.as_small_int())
        .ok_or_else(|| Term::atom(Atom::BADARG))?;
    let remaining = context
        .process_mut()
        .map_or(0, |process| process.reduction_counter());
    budgets().lock().expect("budgets").entry(tag).or_insert((0, 0)).1 = remaining;
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
    native: Atom,
    witness: Atom,
}

fn names(atoms: &AtomTable) -> Names {
    Names {
        module: atoms.intern("jit_yield_probe"),
        driver: atoms.intern("driver"),
        outer: atoms.intern("outer"),
        inner: atoms.intern("inner"),
        native: atoms.intern("count"),
        witness: atoms.intern("slice"),
    }
}

/// Which helper the compiled caller's only edge goes through.
///
/// `CallExt` lowers to `jit_call_interpreted` (`jit/runtime.rs`); `CallFun`
/// lowers to `jit_dispatch_closure` (`jit/runtime_closure.rs`). Both helpers
/// carried the same unconditional module/position restore, so a probe that
/// only drives `CallExt` reports GREEN over a still-broken closure path.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum OuterEdge {
    /// `outer/1` tail-calls `inner/1` directly.
    External,
    /// `outer/2` receives a closure over `inner/1` and tail-calls it.
    Closure,
}

/// `outer/N`'s arity depends on the edge.
///
/// On the closure edge the closure is minted by `driver` and PASSED IN rather
/// than built inside `outer`. That is forced, not stylistic: a function
/// containing `make_fun` can never be JIT-compiled at runtime, because the
/// lambda table is never plumbed past `ModuleCompileMetadata::EMPTY`
/// (beamr#28). An `outer` that minted its own closure is refused by the tier
/// and can never satisfy this arm's compiled-caller witness.
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
            function: names.native,
            arity: 1,
            target: ResolvedImportTarget::Native(NativeEntry {
                function: count,
                dirty_kind: None,
                capability: Capability::Pure,
            }),
        },
        ResolvedImport {
            module: names.module,
            function: names.witness,
            arity: 1,
            target: ResolvedImportTarget::Native(NativeEntry {
                function: slice,
                dirty_kind: None,
                capability: Capability::Pure,
            }),
        },
    ];
    let mut exports = HashMap::new();
    exports.insert((names.driver, 1), 1);
    exports.insert((names.outer, outer_arity(edge)), 2);
    exports.insert((names.inner, 1), 3);

    let mut code = vec![
        // driver/1
        Instruction::FuncInfo {
            module: Operand::Atom(Some(names.module)),
            function: Operand::Atom(Some(names.driver)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 1 },
    ];
    // On the closure edge `driver` mints the closure and hands it over as
    // outer/2's second argument, laid out for `CallFun`: tag in x0, fun in x1.
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
        // inner/1 — interpreted (poisoned against the tier), performs the
        // effect and THEN burns the rest of the slice.
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
        // THE EFFECT. x0 (the tag) is both the argument and what the burn chain
        // leaves untouched, so a replay is observable as a second count.
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(2),
        },
    ]);
    // The burn chain: each `CallOnly` is a local tail call, and a local call
    // charges a reduction, so the chain exhausts the slice partway through and
    // yields AFTER the effect above.
    for step in 0..BURN_STEPS {
        code.push(Instruction::CallOnly {
            arity: Operand::Unsigned(1),
            label: Operand::Label(BURN_LABEL_BASE + step),
        });
        code.push(Instruction::Label {
            label: BURN_LABEL_BASE + step,
        });
    }
    // THE YIELD WITNESS, at the far end of the chain. x0 still holds the tag:
    // `count/1` returned it, and every `CallOnly` above has arity 1.
    code.push(Instruction::CallExt {
        arity: Operand::Unsigned(1),
        import: Operand::Unsigned(3),
    });
    code.push(Instruction::Return);

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
        // Lambda 0 closes over nothing and points at `inner/1`'s entry label.
        // Always present: an unused entry is inert on the `External` edge.
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
    effects: usize,
    /// Reduction counter observed at the effect, before the burn chain.
    reductions_at_effect: u32,
    /// Reduction counter observed after the burn chain. NON-ZERO is the yield
    /// witness — see `slice/1`.
    reductions_at_end: u32,
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

fn run_arm(tag: i64, threshold: u32, heat_drives: usize) -> ArmOutcome {
    run_arm_with(tag, threshold, heat_drives, OuterEdge::External)
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
    let effects = count_for(tag);
    let (reductions_at_effect, reductions_at_end) = budget_for(tag);
    scheduler.shutdown();

    ArmOutcome {
        outer_compiled,
        inner_compiled,
        effects,
        reductions_at_effect,
        reductions_at_end,
        exit: Some((reason, value.root())),
        exit_error,
    }
}

fn report(label: &str, outcome: &ArmOutcome) {
    println!("--- {label} ---");
    println!("  outer/1 in JitCache : {}", outcome.outer_compiled);
    println!("  inner/1 in JitCache : {}", outcome.inner_compiled);
    println!("  EFFECTS (native runs): {}", outcome.effects);
    println!(
        "  reductions at effect: {} (budget {BURN_BUDGET})",
        outcome.reductions_at_effect
    );
    println!(
        "  reductions at end   : {}  <- YIELD WITNESS: non-zero after {BURN_STEPS} \
         charged steps means the slice was RESET, i.e. the process yielded",
        outcome.reductions_at_end
    );
    match &outcome.exit {
        Some((reason, value)) => println!("  exit                : {reason:?} value={value:?}"),
        None => println!("  exit                : TIMED OUT"),
    }
    println!("  exit error          : {:?}", outcome.exit_error);
}

/// The instrument's own positive control: prove the arm actually yielded.
///
/// Without this, BOTH arms show one effect whenever the burn chain fails to
/// exhaust a slice, and the probe reports a green that means "nothing fired".
const _: () = assert!(
    BURN_STEPS > BURN_BUDGET,
    "the burn chain must outlast one slice or the yield witness proves nothing"
);

fn assert_yield_witnessed(label: &str, outcome: &ArmOutcome) {
    assert!(
        outcome.reductions_at_end > 0,
        "{label}: THE INSTRUMENT NEVER FIRED. {BURN_STEPS} charged steps ran on a \
         {BURN_BUDGET}-reduction budget and the counter arrived at the end of the \
         chain at zero, which is what a single un-rescheduled slice looks like \
         (decrement saturates). No yield occurred, so this arm says NOTHING about \
         the mid-body yield path and its effect count is not evidence."
    );
    assert!(
        outcome.reductions_at_end <= BURN_BUDGET,
        "{label}: reduction counter {} exceeds the budget {BURN_BUDGET} — the \
         probe's model of the scheduler is wrong and its arithmetic cannot be \
         trusted",
        outcome.reductions_at_end
    );
}

/// POSITIVE CONTROL — interpreted, the effect happens exactly once.
///
/// This also witnesses that the shape is sound: if the burn chain broke the
/// program, this arm fails and the JIT arm's verdict is inadmissible.
#[test]
fn control_interpreted_mid_body_yield_runs_the_effect_once() {
    let outcome = run_arm(601, 1_000_000, 3);
    report("CONTROL (interpreter only)", &outcome);

    assert!(
        !outcome.outer_compiled && !outcome.inner_compiled,
        "control arm must never compile anything"
    );
    assert_yield_witnessed("CONTROL", &outcome);
    assert_eq!(
        outcome.effects, 1,
        "CONTROL FAILED: interpreted, the effect must happen exactly once"
    );
    assert_eq!(
        outcome.exit.as_ref().map(|(reason, _)| reason),
        Some(&ExitReason::Normal),
        "control arm must exit normally (error: {:?})",
        outcome.exit_error
    );
}

/// THE PROBE. `outer/1` compiled; a mid-body yield inside the nested run.
///
/// RED means the `Yielded` arm carries the same replay defect the suspension
/// arm did, and the fix must cover it too. GREEN means the hypothesis is
/// refuted and the deliberate decision to leave `Yielded` alone was right.
/// Either result is worth having; neither is assumed.
#[test]
fn jit_mid_body_yield_does_not_replay_the_compiled_prefix() {
    let control = run_arm(701, 1_000_000, 3);
    report("CONTROL (interpreter only)", &control);
    assert_yield_witnessed("CONTROL", &control);
    assert_eq!(
        control.effects, 1,
        "the control must be sound before the JIT arm is admissible"
    );

    let jit = run_arm(702, 2, 3);
    report("JIT ARM (outer/1 compiled)", &jit);
    assert_yield_witnessed("JIT ARM", &jit);
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
        jit.effects, control.effects,
        "MID-BODY YIELD REPLAYED THE COMPILED PREFIX: the effect ran {} time(s) \
         under the JIT and {} time(s) interpreted. jit_call_interpreted restores \
         the caller's saved position over the position the nested run set just \
         before yielding, and the rescheduled process re-enters the compiled \
         function from its entry — the same shape as aion#85, on a path the \
         deopt-after-side-effect guard does not cover because a yield is not a \
         deopt.",
        jit.effects, control.effects
    );
    assert_eq!(
        jit.exit, control.exit,
        "a mid-body yield under the JIT must be observably equal to the \
         interpreted one (exit error: {:?})",
        jit.exit_error
    );
}

/// THE SAME QUESTION AT THE OTHER HELPER.
///
/// `CallFun` lowers to `jit_dispatch_closure`, a second nested-run site that
/// carried the identical unconditional restore. It took the identical fix. An
/// arm that only drives `CallExt` would report GREEN over a still-broken
/// closure path — which is exactly the mistake this file exists to prevent, so
/// the closure edge gets its own arm rather than an argument from symmetry.
///
/// This is also the path a fork/join workload reaches, which is why it is the
/// cell that matters most outside these tests.
#[test]
fn jit_closure_mid_body_yield_does_not_replay_the_compiled_prefix() {
    let control = run_arm_with(801, 1_000_000, 3, OuterEdge::Closure);
    report("CONTROL (interpreter only, closure edge)", &control);
    assert_yield_witnessed("CONTROL closure", &control);
    assert_eq!(
        control.effects, 1,
        "the control must be sound before the JIT arm is admissible"
    );

    let jit = run_arm_with(802, 2, 3, OuterEdge::Closure);
    report("JIT ARM (outer/2 compiled, closure edge)", &jit);
    assert_yield_witnessed("JIT ARM closure", &jit);
    assert!(
        jit.outer_compiled,
        "UNWITNESSED JIT ARM: outer/2 never entered the JitCache, so this arm \
         ran interpreted and any green below is a FALSE GREEN. If the tier \
         refused it, check that no `make_fun` reached the compiled function \
         (beamr#28) — `driver` mints the closure precisely so `outer/2` does not."
    );
    assert!(
        !jit.inner_compiled,
        "inner/1 must stay interpreted, or the nested run is not the \
         interpreted run this probe is about"
    );

    assert_eq!(
        jit.effects, control.effects,
        "MID-BODY YIELD REPLAYED THE COMPILED PREFIX ON THE CLOSURE PATH: the \
         effect ran {} time(s) under the JIT and {} time(s) interpreted. \
         `jit_dispatch_closure` restores the caller's saved position over the \
         position the nested run set just before yielding.",
        jit.effects, control.effects
    );
    assert_eq!(
        jit.exit, control.exit,
        "a mid-body yield under a compiled closure caller must be observably \
         equal to the interpreted one (exit error: {:?})",
        jit.exit_error
    );
}
