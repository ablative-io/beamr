//! RED-FIRST PROBE — host-await suspension inside a JIT-compiled caller.
//!
//! HYPOTHESIS UNDER TEST
//! ---------------------
//! When compiled code calls an interpreted external function through
//! `jit_call_interpreted` (`crates/beamr/src/jit/runtime.rs:79`) and that nested
//! interpreted run SUSPENDS on a host await, the helper maps
//! `Ok(ExecutionResult::Waiting)` to `JitReturn::deopt(...)`
//! (`runtime.rs:194-196`) after restoring the caller's saved module and code
//! position (`runtime.rs:180-183`).
//!
//! A deopt is `Ok(None)` out of `call_native`
//! (`crates/beamr/src/interpreter/opcodes/core.rs:915`), which the call site
//! reads as "re-interpret the callee from its entry". The interpreter's run loop
//! (`crates/beamr/src/interpreter/mod.rs:254-263`) never inspects
//! `process.status()`, so it keeps stepping even though `handle_suspend`
//! (`crates/beamr/src/interpreter/opcodes/trampoline.rs:288-296`) has already
//! transitioned the process to `Waiting` and installed a live
//! `SuspensionRecord` whose `position` points INSIDE the nested callee.
//!
//! Predicted consequence: the compiled function's prefix is replayed
//! interpreted, so the host-await native runs a SECOND time — a duplicated host
//! request — and the suspension the embedder was handed a call id for is
//! superseded.
//!
//! SHAPE
//! -----
//! One module, three interpreted functions plus one native:
//!
//! ```text
//! driver/1  : CallExtOnly -> outer/1              (the heatable external edge)
//! outer/1   : CallExt     -> inner/1 ; Return     (THE JIT-COMPILED FUNCTION)
//! inner/1   : CallExt     -> host_await/1 ; Return
//! host_await/1 (native)   : request_await_suspend when tag != HEAT_TAG
//! ```
//!
//! `outer/1` is the compiled caller; its `CallExt` to `inner/1` (a `Code`
//! import) is what lowers to `jit_call_interpreted`. `inner/1` is interpreted
//! and calls the native that parks. `host_await/1` MUST be reached through an
//! interpreted frame: `jit_call_interpreted` refuses `Native` import targets
//! outright (`runtime.rs:142-146`), so a compiled function calling the native
//! directly deopts BEFORE any side effect and is not the shape under test.
//!
//! BOTH NESTED-RUN HELPERS
//! -----------------------
//! `jit_call_interpreted` is not the only helper that runs a nested interpreted
//! run from inside compiled code. `jit_dispatch_closure`
//! (`crates/beamr/src/jit/runtime_closure.rs`) does the same for
//! `CallFun`/`CallFun2`, and carried the identical unconditional restore and the
//! identical `Waiting -> deopt` laundering. The `OuterEdge` parameter swaps
//! `outer/1`'s edge between the two so both helpers are driven: a probe that
//! exercised only `CallExt` would report GREEN over a still-broken closure path,
//! which is the failure mode this file exists to prevent.
//!
//! CONTROL
//! -------
//! The identical drive with `jit_threshold` set so high nothing ever compiles.
//! The control is witnessed NEGATIVELY (`jit_cache().lookup(...) == None`,
//! `compile_outcome_counters().successes == 0`) and the JIT arm is witnessed
//! POSITIVELY (a live `JitCache` entry for `outer/1` before the drive). Without
//! both witnesses a green proves nothing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use beamr::atom::{Atom, AtomTable};
use beamr::loader::decode::compact::Operand;
use beamr::loader::{Instruction, LambdaEntry, lambda_unique_id};
use beamr::module::{Module, ModuleOrigin, ModuleRegistry, ResolvedImport, ResolvedImportTarget};
use beamr::native::{Capability, NativeEntry, ProcessContext};
use beamr::process::ExitReason;
use beamr::scheduler::{NativeBifs, Scheduler, SchedulerConfig};
use beamr::term::Term;

const WAIT_BUDGET: Duration = Duration::from_secs(10);
/// Quiet window after the first native invocation, so a deopt-replay's SECOND
/// invocation has landed before the embedder publishes. Deliberately generous:
/// this probe must not race the thing it is measuring.
const SETTLE: Duration = Duration::from_millis(400);
/// The argument value that puts the native in non-suspending "heat" mode.
const HEAT_TAG: i64 = 0;
/// The value the embedder publishes as the awaited result.
const AWAITED: i64 = 4242;

// ── native-side observation ────────────────────────────────────────────────

/// Per-tag record of what the native did. Tags isolate concurrently running
/// tests from each other (pids collide across schedulers, tags do not).
#[derive(Clone, Debug, Default)]
struct NativeLog {
    /// One entry per suspending invocation, holding the allocated call id.
    call_ids: Vec<Option<u64>>,
    /// Non-parking re-invocations (probe 2 only).
    reentries: usize,
}

fn logs() -> &'static Mutex<HashMap<i64, NativeLog>> {
    static LOGS: OnceLock<Mutex<HashMap<i64, NativeLog>>> = OnceLock::new();
    LOGS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn log_for(tag: i64) -> NativeLog {
    logs().lock().expect("logs").entry(tag).or_default().clone()
}

/// Total heat-mode invocations, used only to bound the heating loop.
static HEAT_INVOCATIONS: AtomicU64 = AtomicU64::new(0);

/// Handshake phase for the RE-ENTRANT native (probe 2), keyed by tag.
/// 0 = idle, 1 = a second invocation is blocked inside the native,
/// 2 = the test has published and released it.
fn phases() -> &'static Mutex<HashMap<i64, u64>> {
    static PHASES: OnceLock<Mutex<HashMap<i64, u64>>> = OnceLock::new();
    PHASES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn phase(tag: i64) -> u64 {
    phases().lock().expect("phases").get(&tag).copied().unwrap_or(0)
}

fn set_phase(tag: i64, value: u64) {
    phases().lock().expect("phases").insert(tag, value);
}

/// `jit_await_probe:host_await/1`.
///
/// * tag == `HEAT_TAG`: returns immediately (heating must not park).
/// * otherwise: parks under a RESULT-GATED host await
///   (`ProcessContext::request_await_suspend`, `native/context/mod.rs:1684`) and
///   records the allocated call id. Always suspends — this is the aion shape,
///   where every host call awaits.
fn host_await(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let tag = args
        .first()
        .and_then(|term| term.as_small_int())
        .ok_or_else(|| Term::atom(Atom::BADARG))?;
    if tag == HEAT_TAG {
        HEAT_INVOCATIONS.fetch_add(1, Ordering::AcqRel);
        return Ok(Term::small_int(7));
    }
    let call_id = context.request_await_suspend(None);
    logs()
        .lock()
        .expect("logs")
        .entry(tag)
        .or_default()
        .call_ids
        .push(call_id);
    Ok(Term::NIL)
}

/// The value a RE-ENTRANT native returns when it is invoked a second time for
/// the same logical operation — the "I already have your answer" path most host
/// bridges take on re-execution.
const REENTRY_VALUE: i64 = 999;

/// `jit_await_probe:host_await/1`, RE-ENTRANT flavor.
///
/// First invocation parks under a gated host await, exactly as above. A second
/// invocation does NOT park: it blocks until the test has published a result
/// against the FIRST suspension (which is still the live one at that moment),
/// then returns `REENTRY_VALUE`. This is the shape that makes the destination of
/// the published value observable instead of collapsing into a
/// `Waiting -> Waiting` transition failure.
fn host_await_reentrant(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let tag = args
        .first()
        .and_then(|term| term.as_small_int())
        .ok_or_else(|| Term::atom(Atom::BADARG))?;
    if tag == HEAT_TAG {
        HEAT_INVOCATIONS.fetch_add(1, Ordering::AcqRel);
        return Ok(Term::small_int(7));
    }
    let already_parked = !log_for(tag).call_ids.is_empty();
    if !already_parked {
        let call_id = context.request_await_suspend(None);
        logs()
            .lock()
            .expect("logs")
            .entry(tag)
            .or_default()
            .call_ids
            .push(call_id);
        return Ok(Term::NIL);
    }
    logs()
        .lock()
        .expect("logs")
        .entry(tag)
        .or_default()
        .reentries += 1;
    set_phase(tag, 1);
    let deadline = Instant::now() + WAIT_BUDGET;
    while phase(tag) != 2 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    Ok(Term::small_int(REENTRY_VALUE))
}

// ── module construction ────────────────────────────────────────────────────

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
    lambdas: Vec<LambdaEntry>,
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
        lambdas,
        string_table: Vec::new(),
        line_info: Vec::new(),
    }
}

struct Names {
    module: Atom,
    driver: Atom,
    outer: Atom,
    inner: Atom,
    native: Atom,
}

fn names(atoms: &AtomTable) -> Names {
    Names {
        module: atoms.intern("jit_await_probe"),
        driver: atoms.intern("driver"),
        outer: atoms.intern("outer"),
        inner: atoms.intern("inner"),
        native: atoms.intern("host_await"),
    }
}

/// How the compiled `outer/1` reaches `inner/1`.
///
/// The two shapes exercise the two DIFFERENT nested-run helpers, which is the
/// whole point of having both: `CallExt` lowers to `jit_call_interpreted`
/// (`jit/runtime.rs`), `CallFun` lowers to `jit_dispatch_closure`
/// (`jit/runtime_closure.rs`). Both helpers ran the nested interpreted run with
/// the same unconditional module/position restore and the same
/// `Waiting -> deopt` laundering, so a probe that only drives `CallExt` reports
/// GREEN over a still-broken closure path.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum OuterEdge {
    /// `outer/1` tail-calls `inner/1` directly.
    External,
    /// `outer/1` builds a closure over `inner/1` and tail-calls it.
    Closure,
}

fn probe_module(
    names: &Names,
    atoms: &AtomTable,
    native: beamr::native::NativeFn,
    poison_outer: bool,
    edge: OuterEdge,
) -> Module {
    let imports = vec![
        // 0: driver/1 -> outer/1 (the heatable external tail edge).
        ResolvedImport {
            module: names.module,
            function: names.outer,
            arity: 1,
            target: ResolvedImportTarget::Code {
                module: names.module,
                label: 2,
            },
        },
        // 1: outer/1 -> inner/1. In COMPILED outer/1 this is the edge that
        //    lowers to `jit_call_interpreted` and runs a nested interpreted run.
        ResolvedImport {
            module: names.module,
            function: names.inner,
            arity: 1,
            target: ResolvedImportTarget::Code {
                module: names.module,
                label: 3,
            },
        },
        // 2: inner/1 -> host_await/1 (native, parks).
        ResolvedImport {
            module: names.module,
            function: names.native,
            arity: 1,
            target: ResolvedImportTarget::Native(NativeEntry {
                function: native,
                dirty_kind: None,
                capability: Capability::Pure,
            }),
        },
    ];
    let mut exports = HashMap::new();
    exports.insert((names.driver, 1), 1);
    exports.insert((names.outer, 1), 2);
    exports.insert((names.inner, 1), 3);
    // The JIT-unsupported representative (`SelectTupleArity` with an empty
    // candidate list, the same one jit_wireup.rs uses): interpreted it jumps
    // straight to the fail label and falls through; the JIT tier refuses the
    // WHOLE function. Used to build a discriminator arm in which `inner/1`
    // still compiles but `outer/1` — the compiled CALLER — does not.
    let mut outer_body = Vec::new();
    if poison_outer {
        outer_body.push(Instruction::SelectTupleArity {
            value: Operand::X(0),
            fail: Operand::Label(4),
            list: Operand::List(vec![]),
        });
        outer_body.push(Instruction::Label { label: 4 });
    }
    match edge {
        OuterEdge::External => outer_body.push(Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(1),
        }),
        OuterEdge::Closure => {
            // x0 = tag on entry. `MakeFun` writes the closure to x0, so park the
            // tag in x1 first and swap them back: `CallFun{arity:1}` wants the
            // argument in x0 and the fun in x1.
            outer_body.push(Instruction::Move {
                source: Operand::X(0),
                destination: Operand::X(1),
            });
            outer_body.push(Instruction::MakeFun {
                operands: vec![Operand::Unsigned(0)],
            });
            outer_body.push(Instruction::Swap {
                left: Operand::X(0),
                right: Operand::X(1),
            });
            outer_body.push(Instruction::CallFun {
                arity: Operand::Unsigned(1),
            });
        }
    }
    outer_body.push(Instruction::Return);

    let mut code = vec![
        // driver/1 (x0 = tag)
        Instruction::FuncInfo {
            module: Operand::Atom(Some(names.module)),
            function: Operand::Atom(Some(names.driver)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 1 },
        Instruction::CallExtOnly {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
        // outer/1 (x0 = tag) — THE COMPILED FUNCTION.
        Instruction::FuncInfo {
            module: Operand::Atom(Some(names.module)),
            function: Operand::Atom(Some(names.outer)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 2 },
        // inner/1 (x0 = tag) — interpreted; calls the parking native.
        Instruction::FuncInfo {
            module: Operand::Atom(Some(names.module)),
            function: Operand::Atom(Some(names.inner)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 3 },
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(2),
        },
        Instruction::Return,
    ];
    // Splice outer/1's body in after its Label 2 (index 4).
    let inner_tail = code.split_off(5);
    code.extend(outer_body);
    code.extend(inner_tail);
    // Lambda 0 closes over nothing and points at `inner/1`'s entry label. Always
    // present: an unused lambda table entry is inert on the `External` edge.
    let lambdas = vec![LambdaEntry {
        function: names.inner,
        arity: 1,
        label: 3,
        num_free: 0,
        unique_id: lambda_unique_id(atoms, names.module, names.inner, 1, 0)
            .expect("inner/1 lambda id"),
    }];
    finish_module(names.module, code, exports, imports, lambdas)
}

// ── arm runner ─────────────────────────────────────────────────────────────

#[derive(Debug)]
struct ArmOutcome {
    /// `true` when `outer/1` had a live `JitCache` entry when the drive started.
    outer_compiled: bool,
    inner_compiled: bool,
    compile_successes: u64,
    /// Non-parking re-invocations of the native (probe 2).
    reentries: usize,
    /// One entry per suspending native invocation on the drive process.
    call_ids: Vec<Option<u64>>,
    /// `wake_with_result_for(pid, first_call_id, ..)` — did the embedder's
    /// answer to the request it was actually handed get published?
    published_for_first_call_id: bool,
    /// Rescue publish against whatever suspension is current, tried only when
    /// the honest one was refused.
    rescue_published: Option<bool>,
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

/// Runs one arm end to end. `threshold` selects the arm: a low threshold heats
/// `outer/1` into the cache, a huge one guarantees the interpreter.
fn run_arm(tag: i64, threshold: u32, heat_drives: usize) -> ArmOutcome {
    run_arm_with(
        tag,
        threshold,
        heat_drives,
        host_await,
        false,
        false,
        OuterEdge::External,
    )
}

fn run_arm_with(
    tag: i64,
    threshold: u32,
    heat_drives: usize,
    native: beamr::native::NativeFn,
    reentrant: bool,
    poison_outer: bool,
    edge: OuterEdge,
) -> ArmOutcome {
    assert_ne!(tag, HEAT_TAG, "the drive tag must not collide with heating");
    let atoms = AtomTable::with_common_atoms();
    let names = names(&atoms);

    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(probe_module(&names, &atoms, native, poison_outer, edge));
    let generation = module.generation();

    let scheduler = Scheduler::new(config(threshold), Arc::clone(&registry), NativeBifs::none())
        .expect("scheduler starts");

    // Heat on the non-suspending tag so no heat drive ever parks.
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

    // Compilation is asynchronous on the dirty-CPU service. Wait for the cache
    // entry itself — the only witness that the drive will dispatch native code.
    let _reached = wait_until(|| {
        scheduler
            .jit_cache()
            .lookup(names.module, names.outer, 1, generation)
            .is_some()
    });
    let outer_compiled = scheduler
        .jit_cache()
        .lookup(names.module, names.outer, 1, generation)
        .is_some();
    let inner_compiled = scheduler
        .jit_cache()
        .lookup(names.module, names.inner, 1, generation)
        .is_some();
    let compile_successes = scheduler.jit_profiler().compile_outcome_counters().successes;

    // ── the drive ──
    let pid = scheduler
        .spawn(names.module, names.driver, vec![Term::small_int(tag)])
        .expect("spawn drive");

    let saw_first = wait_until(|| !log_for(tag).call_ids.is_empty());
    assert!(saw_first, "tag {tag}: the host-await native never ran");
    if reentrant {
        // Wait, bounded, for a replayed second invocation to announce itself
        // and block. Its ABSENCE is a legitimate outcome (the control arm never
        // replays), so this is a timed wait, not an assertion.
        let deadline = Instant::now() + SETTLE;
        while phase(tag) != 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
    } else {
        // Let any deopt-replay finish before publishing, so the measurement is
        // of the steady state and not of a race.
        std::thread::sleep(SETTLE);
    }

    let call_ids = log_for(tag).call_ids;
    let first_call_id = call_ids
        .first()
        .copied()
        .flatten()
        .expect("the first suspension must have allocated a call id");

    // The honest embedder act: answer the request whose call id the native was
    // handed (`Scheduler::wake_with_result_for`, execution.rs:325).
    let published_for_first_call_id =
        scheduler.wake_with_result_for(pid, first_call_id, Term::small_int(AWAITED));
    let rescue_published = if published_for_first_call_id {
        None
    } else {
        // Fall back to the pid-resolved publish so the probe still learns where
        // a value delivered to the CURRENT suspension lands.
        Some(scheduler.wake_with_result(pid, Term::small_int(AWAITED)))
    };
    // Release a blocked re-entrant invocation, if any, now that the publish has
    // happened against the still-live FIRST suspension.
    set_phase(tag, 2);

    let (sender, receiver) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let (reason, value) = scheduler.run_until_exit(pid);
        let error = scheduler.take_exit_error(pid).map(|e| e.to_string());
        let _ = sender.send((reason, value.root(), error));
        scheduler.shutdown();
    });
    let observed = receiver.recv_timeout(Duration::from_secs(15)).ok();
    let (exit, exit_error) = match observed {
        Some((reason, value, error)) => (Some((reason, value)), error),
        None => (None, None),
    };
    if exit.is_some() {
        let _ = waiter.join();
    }

    ArmOutcome {
        outer_compiled,
        inner_compiled,
        compile_successes,
        reentries: log_for(tag).reentries,
        call_ids,
        published_for_first_call_id,
        rescue_published,
        exit,
        exit_error,
    }
}

fn report(label: &str, outcome: &ArmOutcome) {
    println!("--- {label} ---");
    println!("  outer/1 in JitCache        : {}", outcome.outer_compiled);
    println!("  inner/1 in JitCache        : {}", outcome.inner_compiled);
    println!("  compile successes          : {}", outcome.compile_successes);
    println!(
        "  host-await parks           : {} (call ids {:?})",
        outcome.call_ids.len(),
        outcome.call_ids
    );
    println!("  native re-invocations      : {}", outcome.reentries);
    println!(
        "  published for FIRST call id: {}",
        outcome.published_for_first_call_id
    );
    println!("  rescue publish (current)   : {:?}", outcome.rescue_published);
    match &outcome.exit {
        Some((reason, value)) => println!("  exit                       : {reason:?} value={value:?}"),
        None => println!("  exit                       : TIMED OUT (process never exited)"),
    }
    println!("  exit error                 : {:?}", outcome.exit_error);
}

// ── the probe ──────────────────────────────────────────────────────────────

/// POSITIVE CONTROL. No compilation anywhere; the host await must round-trip.
///
/// If this fails the probe itself is broken and the JIT arm's verdict is
/// inadmissible.
#[test]
fn control_interpreted_host_await_delivers_the_value() {
    let outcome = run_arm(101, 1_000_000, 4);
    report("CONTROL (interpreter only)", &outcome);

    assert!(
        !outcome.outer_compiled && !outcome.inner_compiled,
        "control arm must never compile anything"
    );
    assert_eq!(
        outcome.compile_successes, 0,
        "control arm must record no compile successes"
    );
    assert_eq!(
        outcome.call_ids.len(),
        1,
        "control arm: the host-await native must run EXACTLY once"
    );
    assert!(
        outcome.published_for_first_call_id,
        "control arm: the embedder's answer to the handed-out call id must publish"
    );
    assert_eq!(
        outcome.exit,
        Some((ExitReason::Normal, Term::small_int(AWAITED))),
        "control arm: the awaited value must be the process result (error: {:?})",
        outcome.exit_error
    );
}

/// THE PROBE. Same drive, `outer/1` compiled and dispatched native.
#[test]
fn jit_host_await_suspension_matches_the_interpreter() {
    let control = run_arm(201, 1_000_000, 4);
    report("CONTROL (interpreter only)", &control);
    assert_eq!(
        control.call_ids.len(),
        1,
        "the control must be sound before the JIT arm is admissible"
    );
    assert_eq!(
        control.exit,
        Some((ExitReason::Normal, Term::small_int(AWAITED))),
        "the control must be sound before the JIT arm is admissible"
    );

    let jit = run_arm(202, 2, 4);
    report("JIT ARM (outer/1 compiled)", &jit);
    assert!(
        jit.outer_compiled,
        "UNWITNESSED JIT ARM: outer/1 never entered the JitCache, so this arm ran \
         interpreted and any green below is a FALSE GREEN"
    );

    assert_eq!(
        jit.call_ids.len(),
        control.call_ids.len(),
        "HOST SIDE EFFECT REPLAYED: the host-await native ran {} time(s) under the JIT \
         and {} time(s) interpreted. A deopt out of jit_call_interpreted \
         (jit/runtime.rs:194-196) restarts the compiled function interpreted from its \
         entry, re-running the parked host call.",
        jit.call_ids.len(),
        control.call_ids.len()
    );
    assert_eq!(
        jit.published_for_first_call_id, control.published_for_first_call_id,
        "the embedder's answer to the call id it was handed must publish in both arms"
    );
    assert_eq!(
        jit.exit, control.exit,
        "JIT-live host await must be observably equal to the interpreted one \
         (exit error: {:?})",
        jit.exit_error
    );
}

/// PROBE 2 — WHERE DOES THE PUBLISHED VALUE LAND?
///
/// Probe 1's JIT arm dies on the replayed park (`Waiting -> Waiting`), which
/// hides the delivery question. Here the native is RE-ENTRANT: its second
/// invocation does not park. It blocks instead, so the embedder publishes its
/// answer while the FIRST suspension is still the live one — the exact ordering
/// a host bridge sees — and the probe then observes what the process does with
/// it.
///
/// The control arm never replays (no second invocation), publishes against the
/// same call id, and must exit with the awaited value.
#[test]
fn jit_host_await_published_result_lands_where_the_interpreter_puts_it() {
    let control = run_arm_with(301, 1_000_000, 4, host_await_reentrant, true, false, OuterEdge::External);
    report("CONTROL (interpreter only, re-entrant native)", &control);
    assert!(
        !control.outer_compiled,
        "control arm must never compile outer/1"
    );
    assert_eq!(control.reentries, 0, "control arm must not re-invoke the native");
    assert!(
        control.published_for_first_call_id,
        "control arm: publishing against the handed-out call id must succeed"
    );
    assert_eq!(
        control.exit,
        Some((ExitReason::Normal, Term::small_int(AWAITED))),
        "control arm: the awaited value must be the process result (error: {:?})",
        control.exit_error
    );

    let jit = run_arm_with(302, 2, 4, host_await_reentrant, true, false, OuterEdge::External);
    report("JIT ARM (outer/1 compiled, re-entrant native)", &jit);
    assert!(
        jit.outer_compiled,
        "UNWITNESSED JIT ARM: outer/1 never entered the JitCache — any green is FALSE"
    );

    assert_eq!(
        jit.exit,
        control.exit,
        "PUBLISHED HOST RESULT WENT NOWHERE: the embedder's answer published \
         successfully against the live suspension ({}), yet the JIT arm exited with a \
         different value than the interpreted arm. jit exit error: {:?}",
        jit.published_for_first_call_id,
        jit.exit_error
    );
    assert_eq!(
        jit.reentries, control.reentries,
        "the native was re-invoked {} time(s) under the JIT and {} time(s) interpreted: \
         the deopt out of jit_call_interpreted (jit/runtime.rs:194-196) restarted the \
         compiled function interpreted from its entry while the process was already Waiting",
        jit.reentries, control.reentries
    );
}

/// DISCRIMINATOR — the defect requires the COMPILED CALLER, not merely a live
/// JIT tier.
///
/// `outer/1` carries a JIT-unsupported opcode so the tier refuses it, while
/// `inner/1` (whose only edge is to a `Native` import, which
/// `jit_call_interpreted` rejects at `runtime.rs:142-146` before running
/// anything) still compiles and is still dispatched. If this arm behaves like
/// the interpreted control, the mechanism is pinned to the compiled caller's
/// nested interpreted run and not to JIT dispatch in general.
#[test]
fn defect_requires_the_compiled_caller_not_merely_a_live_jit() {
    let arm = run_arm_with(401, 2, 4, host_await_reentrant, true, true, OuterEdge::External);
    report("DISCRIMINATOR (outer/1 refused, inner/1 compiled)", &arm);

    assert!(
        !arm.outer_compiled,
        "the discriminator needs outer/1 REFUSED by the tier"
    );
    assert!(
        arm.inner_compiled,
        "the discriminator needs a live, dispatching JIT: inner/1 must be compiled"
    );
    assert_eq!(
        arm.reentries, 0,
        "with the caller interpreted there must be no deopt-replay of the host call"
    );
    assert_eq!(
        arm.exit,
        Some((ExitReason::Normal, Term::small_int(AWAITED))),
        "with the caller interpreted the awaited value must be delivered (error: {:?})",
        arm.exit_error
    );
}

/// THE CLOSURE PATH — the second nested-run helper.
///
/// `jit_call_interpreted` (`jit/runtime.rs`) is not the only helper that runs a
/// nested interpreted run from inside compiled code: `jit_dispatch_closure`
/// (`jit/runtime_closure.rs`) does the same for `CallFun`/`CallFun2`, and it
/// carried the identical unconditional module/position restore and the identical
/// `Waiting -> deopt` laundering. Every arm above drives `CallExt` only, so
/// WITHOUT THIS ARM a fix applied to one helper would report green while the
/// other stayed broken — the acceptance instrument would have lied.
///
/// Same shape as probe 2, with one edge changed: `outer/1` builds a closure over
/// `inner/1` and tail-calls it instead of calling it externally.
#[test]
fn jit_closure_path_host_await_matches_the_interpreter() {
    let control = run_arm_with(
        501,
        1_000_000,
        4,
        host_await_reentrant,
        true,
        false,
        OuterEdge::Closure,
    );
    report("CONTROL (interpreter only, closure edge)", &control);
    assert!(
        !control.outer_compiled,
        "control arm must never compile outer/1"
    );
    assert_eq!(
        control.reentries, 0,
        "control arm must not re-invoke the native"
    );
    assert_eq!(
        control.exit,
        Some((ExitReason::Normal, Term::small_int(AWAITED))),
        "CONTROL FAILED: the closure edge must deliver the awaited value \
         interpreted, or this arm measures a broken probe rather than the JIT \
         (error: {:?})",
        control.exit_error
    );

    let jit = run_arm_with(
        502,
        2,
        4,
        host_await_reentrant,
        true,
        false,
        OuterEdge::Closure,
    );
    report("JIT ARM (outer/1 compiled, closure edge)", &jit);
    assert!(
        jit.outer_compiled,
        "UNWITNESSED JIT ARM: outer/1 never entered the JitCache, so this arm ran \
         interpreted and any green below is a FALSE GREEN"
    );

    assert_eq!(
        jit.reentries, control.reentries,
        "CLOSURE PATH REPLAYED: the native was re-invoked {} time(s) under the JIT \
         and {} time(s) interpreted. jit_dispatch_closure (jit/runtime_closure.rs) \
         restored the caller's position over the nested run's park position and \
         laundered Waiting into a deopt, restarting the compiled function from its \
         entry while the process was already Waiting.",
        jit.reentries, control.reentries
    );
    assert_eq!(
        jit.call_ids.len(),
        control.call_ids.len(),
        "CLOSURE PATH HOST SIDE EFFECT REPLAYED: {} park(s) under the JIT vs {} \
         interpreted",
        jit.call_ids.len(),
        control.call_ids.len()
    );
    assert_eq!(
        jit.exit, control.exit,
        "the closure edge must be observably equal to the interpreted one \
         (exit error: {:?})",
        jit.exit_error
    );
}
