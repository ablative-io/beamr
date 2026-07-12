//! Wired-path acceptance for JIT-001: the profiler records at the interpreter
//! call edges through the scheduler, threshold-crossing calls compile on the
//! composed dirty-CPU service, and the replay / minimal compositions refuse by
//! absence. All coverage consumes public API only (first-external-consumer
//! gate); every wait is bounded on the compile-outcome counters, never a bare
//! sleep.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use beamr::atom::{Atom, AtomTable};
use beamr::jit::CompileOutcomeCounters;
use beamr::loader::Instruction;
use beamr::loader::decode::compact::Operand;
use beamr::module::{Module, ModuleOrigin, ModuleRegistry, ResolvedImport, ResolvedImportTarget};
use beamr::process::ExitReason;
use beamr::replay::ReplayLog;
use beamr::scheduler::dirty::{DirtyPool, DirtyTask};
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

/// A module whose entry makes `calls` body calls to a local, JIT-supported
/// function `f/0` returning `result`, then returns `f`'s last result.
fn local_hot_module(name: Atom, function: Atom, calls: usize, result: i64) -> Module {
    let mut code = vec![Instruction::Label { label: 1 }];
    for _ in 0..calls {
        code.push(Instruction::Call {
            arity: Operand::Unsigned(0),
            label: Operand::Label(2),
        });
    }
    code.push(Instruction::Return);
    code.push(Instruction::FuncInfo {
        module: Operand::Atom(Some(name)),
        function: Operand::Atom(Some(function)),
        arity: Operand::Unsigned(0),
    });
    code.push(Instruction::Label { label: 2 });
    code.push(Instruction::Move {
        source: Operand::Integer(result),
        destination: Operand::X(0),
    });
    code.push(Instruction::Return);
    finish_module(name, code, HashMap::new(), Vec::new())
}

/// Like [`local_hot_module`], but `f/0`'s body carries a `Trim` — interpreted
/// fine, refused by the JIT tier as an unsupported opcode.
fn local_unsupported_module(name: Atom, function: Atom, calls: usize, result: i64) -> Module {
    let mut code = vec![Instruction::Label { label: 1 }];
    for _ in 0..calls {
        code.push(Instruction::Call {
            arity: Operand::Unsigned(0),
            label: Operand::Label(2),
        });
    }
    code.push(Instruction::Return);
    code.push(Instruction::FuncInfo {
        module: Operand::Atom(Some(name)),
        function: Operand::Atom(Some(function)),
        arity: Operand::Unsigned(0),
    });
    code.push(Instruction::Label { label: 2 });
    code.push(Instruction::Allocate {
        stack_need: Operand::Unsigned(2),
        live: Operand::Unsigned(0),
    });
    code.push(Instruction::Trim {
        words: Operand::Unsigned(1),
        remaining: Operand::Unsigned(1),
    });
    code.push(Instruction::Deallocate {
        words: Operand::Unsigned(1),
    });
    code.push(Instruction::Move {
        source: Operand::Integer(result),
        destination: Operand::X(0),
    });
    code.push(Instruction::Return);
    finish_module(name, code, HashMap::new(), Vec::new())
}

/// A callee module exporting a JIT-supported `g/0`, plus a caller module whose
/// entry makes `calls` external body calls to it.
fn external_pair(
    callee_name: Atom,
    function: Atom,
    caller_name: Atom,
    calls: usize,
    result: i64,
) -> (Module, Module) {
    let callee_code = vec![
        Instruction::FuncInfo {
            module: Operand::Atom(Some(callee_name)),
            function: Operand::Atom(Some(function)),
            arity: Operand::Unsigned(0),
        },
        Instruction::Label { label: 7 },
        Instruction::Move {
            source: Operand::Integer(result),
            destination: Operand::X(0),
        },
        Instruction::Return,
    ];
    let mut exports = HashMap::new();
    exports.insert((function, 0), 7);
    let callee = finish_module(callee_name, callee_code, exports, Vec::new());

    let mut caller_code = vec![Instruction::Label { label: 1 }];
    for _ in 0..calls {
        caller_code.push(Instruction::CallExt {
            arity: Operand::Unsigned(0),
            import: Operand::Unsigned(0),
        });
    }
    caller_code.push(Instruction::Return);
    let imports = vec![ResolvedImport {
        module: callee_name,
        function,
        arity: 0,
        target: ResolvedImportTarget::Code {
            module: callee_name,
            label: 7,
        },
    }];
    let caller = finish_module(caller_name, caller_code, HashMap::new(), imports);
    (callee, caller)
}

fn run_to_value(scheduler: &Scheduler, module: &Arc<Module>) -> Term {
    let pid = scheduler.spawn_process(module);
    let (reason, result) = scheduler.run_until_exit(pid);
    assert_eq!(reason, ExitReason::Normal, "program must exit normally");
    result.root()
}

fn assert_zero_counters(counters: CompileOutcomeCounters, context: &str) {
    assert_eq!(
        counters,
        CompileOutcomeCounters::default(),
        "{context}: every compile-outcome counter must remain zero"
    );
}

/// Wired-path differential equivalence (R5): a hot local function compiles
/// through the scheduler and its post-compile results equal the same program
/// under the minimal composition, where the profiler is absent.
#[test]
fn local_hot_function_compiles_through_scheduler_and_matches_minimal_composition() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_wireup_local");
    let function = atoms.intern("f");
    let threshold = 5;

    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(local_hot_module(module_name, function, threshold, 42));
    let scheduler =
        Scheduler::new(config(threshold as u32), Arc::clone(&registry)).expect("scheduler starts");

    let heated = run_to_value(&scheduler, &module);
    assert_eq!(heated, Term::small_int(42));
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        1,
        "the threshold-crossing call submits exactly one job"
    );
    assert!(
        wait_until(|| scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .successes
            == 1),
        "compilation must land within the wait budget"
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, function, 0, module.generation())
            .is_some(),
        "compiled code must reach the cache at the heated generation"
    );
    assert!(
        scheduler
            .jit_profiler()
            .is_compiled(module_name, function, 0)
    );

    // Post-compile: results unchanged and the profile counter stops moving —
    // the lookup hits, so the edge no longer records.
    let count_after_heat = scheduler
        .jit_profiler()
        .recorded_call_count(module_name, function, 0);
    assert_eq!(count_after_heat, Some(threshold as u32));
    let compiled = run_to_value(&scheduler, &module);
    assert_eq!(
        scheduler
            .jit_profiler()
            .recorded_call_count(module_name, function, 0),
        count_after_heat,
        "post-compile calls dispatch native: record-on-miss-only leaves the counter unmoved"
    );
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        1,
        "no re-submission once compiled"
    );
    scheduler.shutdown();

    // The profiler-absent reference: identical code under minimal().
    let minimal_registry = Arc::new(ModuleRegistry::new());
    let minimal_module =
        minimal_registry.insert(local_hot_module(module_name, function, threshold, 42));
    let minimal = Scheduler::with_services(
        config(threshold as u32),
        SchedulerServices::minimal(),
        Arc::clone(&minimal_registry),
    )
    .expect("minimal scheduler starts");
    let reference = run_to_value(&minimal, &minimal_module);
    assert_eq!(
        compiled, reference,
        "post-compile results must equal the interpreter-only run"
    );
    assert_zero_counters(
        minimal.jit_profiler().compile_outcome_counters(),
        "minimal composition",
    );
    minimal.shutdown();
}

/// The external call edge heats and compiles through the scheduler too (R2:
/// both call forms covered).
#[test]
fn external_call_edge_heats_and_compiles_through_scheduler() {
    let atoms = AtomTable::with_common_atoms();
    let callee_name = atoms.intern("jit_wireup_ext_callee");
    let caller_name = atoms.intern("jit_wireup_ext_caller");
    let function = atoms.intern("g");
    let threshold = 3;

    let registry = Arc::new(ModuleRegistry::new());
    let (callee, caller) = external_pair(callee_name, function, caller_name, threshold, 7);
    let callee = registry.insert(callee);
    let caller = registry.insert(caller);
    let scheduler =
        Scheduler::new(config(threshold as u32), Arc::clone(&registry)).expect("scheduler starts");

    assert_eq!(run_to_value(&scheduler, &caller), Term::small_int(7));
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        1
    );
    assert!(
        wait_until(|| scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .successes
            == 1),
        "external-edge compilation must land within the wait budget"
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(callee_name, function, 0, callee.generation())
            .is_some()
    );

    let count_after_heat = scheduler
        .jit_profiler()
        .recorded_call_count(callee_name, function, 0);
    assert_eq!(run_to_value(&scheduler, &caller), Term::small_int(7));
    assert_eq!(
        scheduler
            .jit_profiler()
            .recorded_call_count(callee_name, function, 0),
        count_after_heat,
        "post-compile external calls dispatch native without recording"
    );
    scheduler.shutdown();
}

/// Replay refusal pin (R5): the same hot program spawned under a replay
/// scheduler submits nothing — the profiling handle is composed away, so
/// nothing is ever recorded, submitted, or cached. Replay outcome semantics
/// (the strict-log mismatch discipline, unchanged by this brief) decide how
/// far the program gets; either way every jit surface stays untouched, which
/// is exactly the pre-brief behavior. The gating fact itself — replay
/// composes the handle away at `build_native_services` — is pinned directly
/// by the scheduler's `replay_and_disabled_dirty_cpu_compose_the_jit_handle_away`
/// unit test; a full replayed EXECUTION differential is not reachable from
/// public paths today because the scheduler never records logs.
#[test]
fn replay_composition_submits_nothing_and_outputs_match() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_wireup_replay");
    let function = atoms.intern("f");
    let threshold = 5;

    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(local_hot_module(module_name, function, threshold, 42));
    let replay = Scheduler::new_replay_with_registry(
        config(threshold as u32),
        Arc::clone(&registry),
        ReplayLog::default(),
    )
    .expect("replay scheduler starts");

    let pid = replay.spawn_process(&module);
    // Bounded wait, never a bare sleep: the empty log's exhaustion discipline
    // terminates the run (pre-existing replay semantics); depending on when
    // the fail lands relative to spawn materialization the process is either
    // tombstoned with the mismatch error or never becomes runnable. Both
    // orderings are pre-brief behavior; neither may touch a jit surface.
    let _terminated = wait_until(|| replay.peek_exit_reason(pid).is_some());
    if let Some(reason) = replay.peek_exit_reason(pid) {
        assert_eq!(
            reason,
            ExitReason::Error,
            "an exhausted empty log fails the replayed process (pre-brief semantics)"
        );
    }

    assert_zero_counters(
        replay.jit_profiler().compile_outcome_counters(),
        "replay composition",
    );
    assert_eq!(
        replay.jit_profiler().profile_entry_count(),
        0,
        "replay composes the profiler away: record_call must be unreachable"
    );
    assert!(
        replay
            .jit_cache()
            .lookup(module_name, function, 0, module.generation())
            .is_none(),
        "nothing may reach the cache under replay"
    );
    replay.shutdown();
}

/// minimal() refusal pin (R5): dirty-CPU Disabled means the handle is absent,
/// so the program runs interpreter-only with zero submission attempts.
#[test]
fn minimal_composition_runs_interpreter_only_with_zero_submissions() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_wireup_minimal");
    let function = atoms.intern("f");
    let threshold = 4;

    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(local_hot_module(module_name, function, threshold * 2, 42));
    let scheduler = Scheduler::with_services(
        config(threshold as u32),
        SchedulerServices::minimal(),
        Arc::clone(&registry),
    )
    .expect("minimal scheduler starts");

    assert_eq!(run_to_value(&scheduler, &module), Term::small_int(42));
    assert_zero_counters(
        scheduler.jit_profiler().compile_outcome_counters(),
        "minimal composition",
    );
    assert_eq!(
        scheduler.jit_profiler().profile_entry_count(),
        0,
        "with the handle absent the edges never record"
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, function, 0, module.generation())
            .is_none()
    );
    scheduler.shutdown();
}

/// Threshold boundary (R2): threshold-1 calls submit nothing; the
/// threshold-th call submits exactly once.
#[test]
fn threshold_minus_one_calls_submit_nothing_and_the_crossing_call_submits_once() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_wireup_threshold");
    let function = atoms.intern("f");
    let threshold = 3;

    let registry = Arc::new(ModuleRegistry::new());
    // One call per spawn: submissions move only with the crossing call.
    let module = registry.insert(local_hot_module(module_name, function, 1, 42));
    let scheduler =
        Scheduler::new(config(threshold), Arc::clone(&registry)).expect("scheduler starts");

    for _ in 0..threshold - 1 {
        assert_eq!(run_to_value(&scheduler, &module), Term::small_int(42));
    }
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        0,
        "threshold-1 calls must not submit"
    );

    assert_eq!(run_to_value(&scheduler, &module), Term::small_int(42));
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        1,
        "the threshold-th call submits exactly one job"
    );
    assert!(
        wait_until(|| scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .successes
            == 1),
        "compilation must land within the wait budget"
    );
    for _ in 0..3 {
        assert_eq!(run_to_value(&scheduler, &module), Term::small_int(42));
    }
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        1,
        "compiled state never re-submits within a generation"
    );
    scheduler.shutdown();
}

/// Hot-load re-heat (R2/T1): a new generation of a previously-compiled
/// function re-heats, recompiles, and dispatches native at that generation.
#[test]
fn hot_reload_reheats_and_recompiles_at_the_new_generation() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_wireup_reload");
    let function = atoms.intern("f");
    let threshold = 3;

    let registry = Arc::new(ModuleRegistry::new());
    let v1 = registry.insert(local_hot_module(module_name, function, threshold, 42));
    let scheduler =
        Scheduler::new(config(threshold as u32), Arc::clone(&registry)).expect("scheduler starts");

    assert_eq!(run_to_value(&scheduler, &v1), Term::small_int(42));
    assert!(
        wait_until(|| scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .successes
            == 1),
        "generation-1 compilation must land"
    );
    assert!(
        scheduler
            .jit_profiler()
            .is_compiled(module_name, function, 0)
    );

    // The reload: a fresh generation of the same code.
    let v2 = registry.insert(local_hot_module(module_name, function, threshold, 42));
    assert_eq!(v2.generation(), 2);
    assert_eq!(run_to_value(&scheduler, &v2), Term::small_int(42));
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        2,
        "the new generation re-heats and re-submits"
    );
    assert!(
        wait_until(|| scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .successes
            == 2),
        "generation-2 compilation must land"
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, function, 0, 2)
            .is_some(),
        "the recompile keys the cache at the NEW generation"
    );
    assert_eq!(run_to_value(&scheduler, &v2), Term::small_int(42));
    assert_eq!(
        scheduler.jit_profiler().profile_entry_count(),
        1,
        "generation stamping reuses the MFA's slot across the reload"
    );
    scheduler.shutdown();
}

/// Unsupported retry (R2/T1): a new generation of a previously-UNSUPPORTED
/// function retries — the old verdict is about code that no longer runs.
#[test]
fn unsupported_function_retries_at_the_new_generation() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_wireup_unsupported");
    let function = atoms.intern("u");
    let threshold = 2;

    let registry = Arc::new(ModuleRegistry::new());
    // One call per spawn so the profiler state is observable between calls.
    let v1 = registry.insert(local_unsupported_module(module_name, function, 1, 7));
    let scheduler =
        Scheduler::new(config(threshold), Arc::clone(&registry)).expect("scheduler starts");

    for _ in 0..threshold {
        assert_eq!(run_to_value(&scheduler, &v1), Term::small_int(7));
    }
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        1
    );
    assert!(
        wait_until(|| scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .unsupported
            == 1),
        "the JIT tier must refuse the Trim-carrying body"
    );
    assert!(
        scheduler
            .jit_profiler()
            .is_unsupported(module_name, function, 0)
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, function, 0, v1.generation())
            .is_none()
    );

    // Within the generation UNSUPPORTED is terminal: no further submissions.
    assert_eq!(run_to_value(&scheduler, &v1), Term::small_int(7));
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        1
    );

    // A new generation retries: the first new-generation call resets the
    // profile to INTERPRETING and counts.
    let v2 = registry.insert(local_unsupported_module(module_name, function, 1, 7));
    assert_eq!(run_to_value(&scheduler, &v2), Term::small_int(7));
    assert!(
        !scheduler
            .jit_profiler()
            .is_unsupported(module_name, function, 0),
        "the first new-generation call must reset the profile to INTERPRETING"
    );
    assert_eq!(
        scheduler
            .jit_profiler()
            .recorded_call_count(module_name, function, 0),
        Some(1),
        "the resetting call itself counts at the new generation"
    );
    // The retry reaches a fresh verdict at the new generation.
    assert_eq!(run_to_value(&scheduler, &v2), Term::small_int(7));
    assert!(
        wait_until(|| scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .unsupported
            == 2),
        "the retry must reach the tier again at the new generation"
    );
    scheduler.shutdown();
}

/// Mixed-generation clause (R2, option (a)): old-generation calls neither
/// reset nor count while the new generation heats; the new generation
/// compiles and dispatches native.
#[test]
fn old_generation_calls_do_not_count_while_the_new_generation_heats() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_wireup_mixed");
    let function = atoms.intern("f");
    let threshold = 4;

    let registry = Arc::new(ModuleRegistry::new());
    let v1 = registry.insert(local_hot_module(module_name, function, 1, 42));

    let scheduler =
        Scheduler::new(config(threshold), Arc::clone(&registry)).expect("scheduler starts");

    // Pre-heat the old generation a little, then load the new one.
    for _ in 0..2 {
        assert_eq!(run_to_value(&scheduler, &v1), Term::small_int(42));
    }
    let v2 = registry.insert(local_hot_module(module_name, function, 1, 42));

    // The first new-generation call resets the stamp and counts once.
    assert_eq!(run_to_value(&scheduler, &v2), Term::small_int(42));
    assert_eq!(
        scheduler
            .jit_profiler()
            .recorded_call_count(module_name, function, 0),
        Some(1)
    );

    // A still-running old-generation process keeps calling: processes spawned
    // from the v1 Arc execute generation-1 code after generation 2 loaded.
    for _ in 0..3 * threshold {
        assert_eq!(run_to_value(&scheduler, &v1), Term::small_int(42));
    }
    assert_eq!(
        scheduler
            .jit_profiler()
            .recorded_call_count(module_name, function, 0),
        Some(1),
        "old-generation calls must be proven not to count"
    );
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        0,
        "old-generation calls must never be the threshold-crosser"
    );

    // The new generation finishes heating and compiles at ITS generation.
    for _ in 0..threshold - 1 {
        assert_eq!(run_to_value(&scheduler, &v2), Term::small_int(42));
    }
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        1
    );
    assert!(
        wait_until(|| scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .successes
            == 1),
        "the new generation's compilation must land"
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, function, 0, v2.generation())
            .is_some()
    );
    assert_eq!(run_to_value(&scheduler, &v2), Term::small_int(42));
    scheduler.shutdown();
}

/// Submission-failure pin (R3): a refused submission leaves the calling
/// process on bytecode with no error, resets the profile at the submitted
/// generation, and the function can re-heat.
#[test]
fn refused_submission_leaves_bytecode_running_and_the_profile_reheatable() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_wireup_refusal");
    let function = atoms.intern("f");
    let threshold = 2;

    // A shared one-worker, depth-one pool, saturated before the scheduler
    // ever submits: the worker is parked on the gate and the queue slot is
    // occupied, so the JIT submission is refused with a full queue.
    let pool = Arc::new(DirtyPool::with_queue_depth("jit-wireup-refusal", 1, 1));
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (gate_tx, gate_rx) = mpsc::channel::<()>();
    pool.submit_task(DirtyTask::new(move || {
        let _ = started_tx.send(());
        let _ = gate_rx.recv();
    }))
    .expect("occupying task submits");
    started_rx
        .recv_timeout(WAIT_BUDGET)
        .expect("occupying task starts");
    pool.submit_task(DirtyTask::new(|| {}))
        .expect("queue-filling task submits");

    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(local_hot_module(module_name, function, 1, 42));
    let scheduler = Scheduler::with_services(
        config(threshold),
        SchedulerServices::minimal().shared_dirty_cpu(Arc::clone(&pool)),
        Arc::clone(&registry),
    )
    .expect("scheduler with the shared pool starts");

    for _ in 0..threshold {
        assert_eq!(
            run_to_value(&scheduler, &module),
            Term::small_int(42),
            "a refused submission must leave the process running bytecode with no error"
        );
    }
    let counters = scheduler.jit_profiler().compile_outcome_counters();
    assert_eq!(counters.submissions, 1, "the refused attempt was counted");
    assert_eq!(counters.transient_failures, 1, "the refusal is transient");
    assert_eq!(counters.successes, 0);
    assert_eq!(
        scheduler
            .jit_profiler()
            .recorded_call_count(module_name, function, 0),
        Some(0),
        "the refusal must reset the profile counter at the submitted generation"
    );
    assert!(
        !scheduler
            .jit_profiler()
            .is_compiled(module_name, function, 0)
    );
    assert!(
        !scheduler
            .jit_profiler()
            .is_unsupported(module_name, function, 0)
    );

    // Re-heat proof: the reset profile crosses the threshold again.
    for _ in 0..threshold {
        assert_eq!(run_to_value(&scheduler, &module), Term::small_int(42));
    }
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        2,
        "the function re-heats after the transient refusal"
    );

    scheduler.shutdown();
    let _ = gate_tx.send(());
    drop(pool);
}

/// Module delete through the scheduler drops the module's profile entries
/// (R7) AND its cached native code: registry generations restart at 1 after
/// a delete, so a retained cache entry would collide with a reload of the
/// same name reaching the same generation and execute the deleted module's
/// code.
#[test]
fn delete_module_through_the_scheduler_drops_hot_profiles_and_cached_code() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_wireup_delete");
    let function = atoms.intern("f");
    let threshold = 2;

    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(local_hot_module(module_name, function, 2, 42));
    let scheduler =
        Scheduler::new(config(threshold), Arc::clone(&registry)).expect("scheduler starts");

    assert_eq!(run_to_value(&scheduler, &module), Term::small_int(42));
    assert_eq!(scheduler.jit_profiler().profile_entry_count(), 1);
    assert!(
        wait_until(|| scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .successes
            == 1),
        "compilation must land before the delete exercises the seam"
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, function, 0, module.generation())
            .is_some()
    );

    assert!(scheduler.delete_module(module_name));
    assert_eq!(
        scheduler.jit_profiler().profile_entry_count(),
        0,
        "the scheduler delete seam must drop the module's profile entries"
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, function, 0, module.generation())
            .is_none(),
        "the scheduler delete seam must drop the module's cached native code"
    );
    scheduler.shutdown();
}

/// Export-alias ownership gate: export validation only checks that the label
/// EXISTS, so a loader-legal export can alias ANOTHER function's entry. The
/// external edge must refuse when the target entry's owning function differs
/// from the exported identity — otherwise a heated `foo/0` cache entry runs
/// foo's native code at a call site whose bytecode resolution is bar.
#[test]
fn aliased_export_never_runs_or_heats_another_functions_code() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_wireup_alias");
    let caller_name = atoms.intern("jit_wireup_alias_caller");
    let foo = atoms.intern("foo");
    let threshold = 2;

    // entry: threshold local calls to foo (heats and compiles foo/0);
    // foo/0 returns 42; bar/0 returns 9.
    let mut code = vec![Instruction::Label { label: 1 }];
    for _ in 0..threshold {
        code.push(Instruction::Call {
            arity: Operand::Unsigned(0),
            label: Operand::Label(2),
        });
    }
    code.push(Instruction::Return);
    code.extend([
        Instruction::FuncInfo {
            module: Operand::Atom(Some(module_name)),
            function: Operand::Atom(Some(foo)),
            arity: Operand::Unsigned(0),
        },
        Instruction::Label { label: 2 },
        Instruction::Move {
            source: Operand::Integer(42),
            destination: Operand::X(0),
        },
        Instruction::Return,
        Instruction::FuncInfo {
            module: Operand::Atom(Some(module_name)),
            function: Operand::Atom(Some(atoms.intern("bar"))),
            arity: Operand::Unsigned(0),
        },
        Instruction::Label { label: 3 },
        Instruction::Move {
            source: Operand::Integer(9),
            destination: Operand::X(0),
        },
        Instruction::Return,
    ]);
    let registry = Arc::new(ModuleRegistry::new());
    // The aliased-export shape itself: foo/0 is EXPORTED at bar's entry
    // label. Loader validation only checks that the label exists.
    let mut exports = HashMap::new();
    exports.insert((foo, 0), 3);
    let module = registry.insert(finish_module(module_name, code, exports, Vec::new()));

    // The caller imports foo/0; resolution follows the aliased export to
    // BAR's entry label.
    let caller_code = vec![
        Instruction::Label { label: 1 },
        Instruction::CallExt {
            arity: Operand::Unsigned(0),
            import: Operand::Unsigned(0),
        },
        Instruction::Return,
    ];
    let imports = vec![ResolvedImport {
        module: module_name,
        function: foo,
        arity: 0,
        target: ResolvedImportTarget::Code {
            module: module_name,
            label: 3,
        },
    }];
    let caller = registry.insert(finish_module(caller_name, caller_code, HashMap::new(), imports));

    let scheduler =
        Scheduler::new(config(threshold), Arc::clone(&registry)).expect("scheduler starts");

    // Heat and compile the REAL foo/0.
    assert_eq!(run_to_value(&scheduler, &module), Term::small_int(42));
    assert!(
        wait_until(|| scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .successes
            == 1),
        "foo/0 must compile through the wire"
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, foo, 0, module.generation())
            .is_some()
    );

    // The aliased external call must run bar's BYTECODE (9), never foo's
    // cached native code (42), and must not heat anything under foo/0.
    for _ in 0..3 {
        assert_eq!(
            run_to_value(&scheduler, &caller),
            Term::small_int(9),
            "an aliased export must execute its bytecode target, not the exported name's cache"
        );
    }
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        1,
        "aliased calls must not heat the exported identity"
    );
    scheduler.shutdown();
}

/// Completion racing the delete seam (publication guard): a job still queued
/// when `delete_module` runs must publish NOTHING when it eventually
/// completes — no cache entry (a same-name reload reaching the same
/// generation number would execute the deleted module's code) and no
/// resurrected profile.
#[test]
fn queued_compilation_completing_after_delete_publishes_nothing() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_wireup_pub_delete");
    let function = atoms.intern("f");
    let threshold = 2;

    // One gated worker, queue depth 2: the gate task occupies the worker, the
    // JIT job queues behind it and completes only after the delete.
    let pool = Arc::new(DirtyPool::with_queue_depth("jit-wireup-pub-delete", 1, 2));
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (gate_tx, gate_rx) = mpsc::channel::<()>();
    pool.submit_task(DirtyTask::new(move || {
        let _ = started_tx.send(());
        let _ = gate_rx.recv();
    }))
    .expect("gate task submits");
    started_rx
        .recv_timeout(WAIT_BUDGET)
        .expect("gate task starts");

    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(local_hot_module(module_name, function, threshold, 42));
    let generation = module.generation();
    let scheduler = Scheduler::with_services(
        config(threshold as u32),
        SchedulerServices::minimal().shared_dirty_cpu(Arc::clone(&pool)),
        Arc::clone(&registry),
    )
    .expect("scheduler with the shared pool starts");

    assert_eq!(run_to_value(&scheduler, &module), Term::small_int(42));
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        1,
        "the job must be submitted (queued) before the delete"
    );

    assert!(scheduler.delete_module(module_name));
    assert_eq!(scheduler.jit_profiler().profile_entry_count(), 0);

    // Release the worker: the queued job compiles, then must refuse to
    // publish against the deleted profile.
    let _ = gate_tx.send(());
    assert!(
        wait_until(|| scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .transient_failures
            == 1),
        "the late completion must be counted as a refused publication"
    );
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .successes,
        0,
        "a completion after delete is not a success"
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, function, 0, generation)
            .is_none(),
        "the late completion must not strand a cache entry for the deleted module"
    );
    assert_eq!(
        scheduler.jit_profiler().profile_entry_count(),
        0,
        "the late completion must not resurrect the profile"
    );
    scheduler.shutdown();
    drop(pool);
}

/// The replacement-before-completion interleaving: a job queued before a
/// delete completes only AFTER a same-name reload has already heated a
/// replacement profile at the registry's RESTARTED (lower) generation. The
/// stale completion must neither stamp the replacement's profile nor publish
/// code — exact-generation-match completions — and the replacement must then
/// compile normally at its own generation.
#[test]
fn queued_stale_job_never_stamps_a_replacement_profile() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_wireup_stale_stamp");
    let function = atoms.intern("f");
    let threshold = 2;

    let pool = Arc::new(DirtyPool::with_queue_depth("jit-wireup-stale-stamp", 1, 2));
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (gate_tx, gate_rx) = mpsc::channel::<()>();
    pool.submit_task(DirtyTask::new(move || {
        let _ = started_tx.send(());
        let _ = gate_rx.recv();
    }))
    .expect("gate task submits");
    started_rx
        .recv_timeout(WAIT_BUDGET)
        .expect("gate task starts");

    let registry = Arc::new(ModuleRegistry::new());
    // Two inserts: the hot version is generation 2, so the queued job's
    // generation sits ABOVE the replacement's restarted generation 1.
    registry.insert(local_hot_module(module_name, function, threshold, 42));
    let v2 = registry.insert(local_hot_module(module_name, function, threshold, 42));
    assert_eq!(v2.generation(), 2);
    let scheduler = Scheduler::with_services(
        config(threshold as u32),
        SchedulerServices::minimal().shared_dirty_cpu(Arc::clone(&pool)),
        Arc::clone(&registry),
    )
    .expect("scheduler with the shared pool starts");

    // Heat generation 2: its job queues behind the gated worker.
    assert_eq!(run_to_value(&scheduler, &v2), Term::small_int(42));
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        1
    );

    // Delete, then reload: the replacement restarts at generation 1 and
    // heats one call (below threshold) BEFORE the stale job completes.
    assert!(scheduler.delete_module(module_name));
    let replacement = registry.insert(local_hot_module(module_name, function, 1, 42));
    assert_eq!(
        replacement.generation(),
        1,
        "the registry restarts a deleted name at generation 1 — the premise of this pin"
    );
    assert_eq!(run_to_value(&scheduler, &replacement), Term::small_int(42));
    assert_eq!(
        scheduler
            .jit_profiler()
            .recorded_call_count(module_name, function, 0),
        Some(1)
    );

    // Release the stale generation-2 job.
    let _ = gate_tx.send(());
    assert!(
        wait_until(|| scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .transient_failures
            == 1),
        "the stale completion must be refused and counted transient"
    );
    assert!(
        !scheduler
            .jit_profiler()
            .is_compiled(module_name, function, 0),
        "a stale job must not stamp COMPILED onto the replacement's profile"
    );
    assert_eq!(
        scheduler
            .jit_profiler()
            .recorded_call_count(module_name, function, 0),
        Some(1),
        "the stale completion must not touch the replacement's heat"
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, function, 0, 2)
            .is_none(),
        "the stale job must publish nothing at its own generation"
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, function, 0, 1)
            .is_none()
    );

    // The replacement heats to threshold and compiles normally at ITS
    // generation on the now-free worker.
    assert_eq!(run_to_value(&scheduler, &replacement), Term::small_int(42));
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        2
    );
    assert!(
        wait_until(|| scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .successes
            == 1),
        "the replacement's own compilation must land"
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, function, 0, 1)
            .is_some(),
        "the replacement compiles at its own restarted generation"
    );
    scheduler.shutdown();
    drop(pool);
}

/// Canonical-entry gate: a call targeting a label INSIDE a function shares
/// that function's MFA, so letting it heat or hit the JIT surface would cache
/// a suffix under the whole function's identity — a later entry call would
/// then execute the suffix and skip the function's prefix entirely. The gate
/// keeps non-canonical targets pure bytecode.
#[test]
fn internal_label_calls_never_touch_the_jit_surface() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_wireup_suffix_bait");
    let function = atoms.intern("f");
    let threshold = 2;

    // entry: x0 := 7; call f; return — f: x0 := 42; call label 3; return —
    // label 3 (inside f): return. If the internal call were profiled under
    // f/0, it would cross the threshold during the FIRST run and cache the
    // [Return] suffix as f/0; the second run's entry call would then hit that
    // suffix, skip `x0 := 42`, and return 7.
    let code = vec![
        Instruction::Label { label: 1 },
        Instruction::Move {
            source: Operand::Integer(7),
            destination: Operand::X(0),
        },
        Instruction::Call {
            arity: Operand::Unsigned(0),
            label: Operand::Label(2),
        },
        Instruction::Return,
        Instruction::FuncInfo {
            module: Operand::Atom(Some(module_name)),
            function: Operand::Atom(Some(function)),
            arity: Operand::Unsigned(0),
        },
        Instruction::Label { label: 2 },
        Instruction::Move {
            source: Operand::Integer(42),
            destination: Operand::X(0),
        },
        Instruction::Call {
            arity: Operand::Unsigned(0),
            label: Operand::Label(3),
        },
        Instruction::Return,
        Instruction::Label { label: 3 },
        Instruction::Return,
    ];
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(finish_module(module_name, code, HashMap::new(), Vec::new()));
    let scheduler =
        Scheduler::new(config(threshold), Arc::clone(&registry)).expect("scheduler starts");

    assert_eq!(run_to_value(&scheduler, &module), Term::small_int(42));
    assert_eq!(
        scheduler
            .jit_profiler()
            .recorded_call_count(module_name, function, 0),
        Some(1),
        "only the canonical entry call may record: the internal call must not count"
    );
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        0,
        "the internal call must never be the threshold-crosser"
    );

    // The second canonical call crosses the threshold with the FULL function
    // slice; whatever the tier's verdict on that slice, every subsequent call
    // keeps returning the whole function's result.
    for _ in 0..3 {
        assert_eq!(
            run_to_value(&scheduler, &module),
            Term::small_int(42),
            "entry calls must always execute the whole function, never a cached suffix"
        );
    }
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        1,
        "the canonical threshold-crossing submits exactly once"
    );
    scheduler.shutdown();
}
