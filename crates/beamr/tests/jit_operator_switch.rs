//! Acceptance for the runtime JIT switch (#26): an operator can turn the JIT
//! off on a LIVE scheduler, and doing so stops compiled code being produced
//! AND stops compiled code being entered.
//!
//! Why the switch exists. `SchedulerConfig::jit_threshold` cannot serve as the
//! control, for two reasons that survive any value chosen for it: it is fixed
//! at construction, so it cannot be thrown on a live scheduler; and it governs
//! only whether code is COMPILED, never whether already-compiled code is
//! ENTERED. By the time a JIT fault has been diagnosed the code is cached, and
//! at that point no threshold keeps execution out of it. (A very large
//! threshold does defer compilation past any realistic workload, and is a fair
//! workaround before a scheduler is built — deferral, not refusal.) Otherwise
//! the only way out of a JIT defect was rebuilding beamr with
//! `default-features = false`: no lever at all for a running deployment.
//!
//! What the coverage is careful about. Every arm runs a POSITIVE CONTROL
//! first: the enabled scheduler must be observed compiling before the disabled
//! scheduler's silence is read as a measurement. An arm that never compiled in
//! the control is a dead instrument, and its quiet disabled arm would be worth
//! nothing.
//!
//! All coverage consumes public API only, and every wait is bounded on the
//! compile-outcome counters rather than a bare sleep.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use beamr::atom::{Atom, AtomTable};
use beamr::loader::Instruction;
use beamr::loader::decode::compact::Operand;
use beamr::module::{Module, ModuleOrigin, ModuleRegistry};
use beamr::process::ExitReason;
use beamr::scheduler::{NativeBifs, Scheduler, SchedulerConfig};
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

fn finish_module(name: Atom, code: Vec<Instruction>) -> Module {
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
        exports: HashMap::new(),
        label_index,
        code,
        function_table,
        line_table: Vec::new(),
        literals: Vec::new(),
        constant_pool: Default::default(),
        resolved_imports: Vec::new(),
        lambdas: Vec::new(),
        string_table: Vec::new(),
        line_info: Vec::new(),
    }
}

/// A function whose body calls a leaf `calls` times, so one run of the program
/// heats the leaf by exactly `calls` recorded calls.
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
    finish_module(name, code)
}

fn run_to_value(scheduler: &Scheduler, module: &Arc<Module>) -> Term {
    let pid = scheduler.spawn_process(module);
    let (reason, result) = scheduler.run_until_exit(pid);
    assert_eq!(reason, ExitReason::Normal, "program must exit normally");
    result.root()
}

/// Disabling BEFORE any process runs: nothing is ever compiled, and the
/// program still computes the right answer interpreted.
///
/// The enabled scheduler is the control. Both schedulers run the identical
/// program over the identical threshold; the only difference is the switch.
#[test]
fn a_scheduler_disabled_before_first_run_never_compiles_and_still_computes() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_switch_never");
    let function = atoms.intern("f");
    let threshold = 5;

    // CONTROL: switch on — this program is observed to compile.
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(local_hot_module(module_name, function, threshold, 42));
    let enabled = Scheduler::new(
        config(threshold as u32),
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .expect("scheduler starts");
    assert_eq!(run_to_value(&enabled, &module), Term::small_int(42));
    assert!(
        wait_until(|| enabled.jit_profiler().compile_outcome_counters().successes == 1),
        "CONTROL FAILED: the enabled scheduler must compile this program, or \
         the disabled arm below proves nothing"
    );
    enabled.shutdown();

    // MEASUREMENT: same program, same threshold, switch off from the start.
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(local_hot_module(module_name, function, threshold, 42));
    let disabled = Scheduler::new(
        config(threshold as u32),
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .expect("scheduler starts");
    assert!(disabled.set_jit_enabled(false), "was on before the flip");

    for _ in 0..4 {
        assert_eq!(
            run_to_value(&disabled, &module),
            Term::small_int(42),
            "a disabled JIT must not change results"
        );
    }
    let counters = disabled.jit_profiler().compile_outcome_counters();
    assert_eq!(
        counters.submissions, 0,
        "a disabled scheduler must submit no compilation, however hot the code gets"
    );
    assert_eq!(counters.successes, 0, "and must compile nothing");
    assert!(
        disabled
            .jit_cache()
            .lookup(module_name, function, 0, module.generation())
            .is_none(),
        "nothing may reach the cache while the switch is off"
    );
    disabled.shutdown();
}

/// The half that matters: code compiled BEFORE the switch was thrown is not
/// entered afterwards — and the cache survives, so re-enabling costs no
/// recompilation.
///
/// This is the shape a naive switch gets wrong. Stopping new compilation while
/// continuing to dispatch into already-cached code would leave an operator
/// convinced the JIT was off while the very code they were escaping kept
/// running — and cache presence is exactly the variable that distinguishes a
/// fresh process from a poisoned one.
#[test]
fn code_compiled_before_the_switch_is_not_entered_after_it_and_the_cache_survives() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_switch_cached");
    let function = atoms.intern("f");
    let threshold = 5;

    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(local_hot_module(module_name, function, threshold, 7));
    let scheduler = Scheduler::new(
        config(threshold as u32),
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .expect("scheduler starts");

    // Heat it with the switch ON until compiled code is genuinely in the cache.
    assert_eq!(run_to_value(&scheduler, &module), Term::small_int(7));
    assert!(
        wait_until(|| scheduler.jit_profiler().compile_outcome_counters().successes == 1),
        "CONTROL FAILED: the function must actually compile before the switch \
         is thrown, or there is no cached code to withhold"
    );
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, function, 0, module.generation())
            .is_some(),
        "CONTROL: compiled code is in the cache at this generation"
    );
    let submissions_when_hot = scheduler
        .jit_profiler()
        .compile_outcome_counters()
        .submissions;

    // Throw the switch on the LIVE scheduler.
    scheduler.set_jit_enabled(false);
    for _ in 0..4 {
        assert_eq!(
            run_to_value(&scheduler, &module),
            Term::small_int(7),
            "results are unchanged with the JIT switched off mid-life"
        );
    }
    assert!(
        scheduler
            .jit_cache()
            .lookup(module_name, function, 0, module.generation())
            .is_some(),
        "the switch WITHHOLDS the cache; it must not destroy or evict it"
    );
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        submissions_when_hot,
        "no fresh submission while off — the switch is not silently recompiling"
    );

    // Reversible, and cheap: the cached code is still there to be used again,
    // so re-enabling submits nothing new.
    scheduler.set_jit_enabled(true);
    assert_eq!(run_to_value(&scheduler, &module), Term::small_int(7));
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        submissions_when_hot,
        "re-enabling reuses the surviving cache entry rather than recompiling"
    );
    scheduler.shutdown();
}

/// The motivating measurement, pinned so it cannot rot into folklore: a raised
/// threshold buys DELAY, not immunity. Hotness accumulates across runs and the
/// trip test is `>=`, so a function under a raised threshold still compiles
/// once enough calls have gone by — the reason an operator needs a switch
/// rather than a bigger number.
///
/// Bounded to stay a unit-scale test: rather than exhausting a `u32::MAX`
/// counter, it sets a threshold above one run's heat and shows the second run
/// crossing it. The deferral-not-refusal property is the same one at scale.
#[test]
fn raising_the_threshold_delays_compilation_but_never_prevents_it() {
    let atoms = AtomTable::with_common_atoms();
    let module_name = atoms.intern("jit_switch_threshold");
    let function = atoms.intern("f");

    // A threshold ABOVE one run's heat: one run must not compile.
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(local_hot_module(module_name, function, 3, 1));
    let scheduler = Scheduler::new(config(6), Arc::clone(&registry), NativeBifs::none())
        .expect("scheduler starts");
    assert_eq!(run_to_value(&scheduler, &module), Term::small_int(1));
    assert_eq!(
        scheduler
            .jit_profiler()
            .compile_outcome_counters()
            .submissions,
        0,
        "three calls under a threshold of six must not submit"
    );

    // Keep running: the counter accumulates across runs and the threshold is
    // reached. A higher threshold buys delay, never immunity — which is why an
    // operator needs `set_jit_enabled`, not a bigger number.
    assert_eq!(run_to_value(&scheduler, &module), Term::small_int(1));
    assert!(
        wait_until(|| scheduler.jit_profiler().compile_outcome_counters().successes == 1),
        "accumulated heat must cross the raised threshold and compile"
    );
    scheduler.shutdown();
}
