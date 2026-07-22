//! ADMISSION ARC LEG 1c — slicer label retention (A1 + A2), end-to-end through
//! the wired demand path, on top of the corrected interpreter (func_info now
//! raises function_clause, so a deopt from the func_info terminal is
//! differential-equal to TRUE semantics).
//!
//! Both slicers (`Module::function_instructions` + `aot::exported_instructions`,
//! under the co-updated R8 pin) now retain the func_info prelude. `FuncInfo` is a
//! `Coverage::Supported` DEOPT terminal, and the native entry branches to the
//! export label — never the prelude. This admits every multi-clause FRAMELESS
//! function; the frame / body-call continuation model stays Leg 3's.
//!
//! * `frameless.erl` (A2): loop/2 is a 4-clause function whose `select_val` fails
//!   to the func_info prelude label. Pre-1c that label was stripped ->
//!   UnknownLabel -> rejected. Post-1c it compiles and matches the interpreter.
//! * `jit_real_function` (label half vs frame half): prev/1 is a 3-clause
//!   frameless function -> compiles (label half green). loop/2 saves N across a
//!   body-position `?MODULE:prev(N)` call (a frame / Y-live-across-body-call) ->
//!   stays an honestly-pinned rejection (Leg 3's continuation model).

use std::sync::Arc;
use std::time::{Duration, Instant};

use beamr::atom::{Atom, AtomTable};
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;
use beamr::process::ExitReason;
use beamr::scheduler::{Scheduler, SchedulerConfig, SchedulerServices};
use beamr::term::Term;

const FRAMELESS: &[u8] = include_bytes!("fixtures/frameless.beam");
const REAL_FUNCTION: &[u8] = include_bytes!("fixtures/jit_real_function.beam");
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

fn scheduler(atoms: &Arc<AtomTable>, services: SchedulerServices) -> Scheduler {
    let bifs = BifRegistryImpl::new();
    Scheduler::with_services_and_code_server(
        config(2),
        services,
        Arc::new(ModuleRegistry::new()),
        Arc::clone(atoms),
        Arc::new(bifs),
    )
    .expect("scheduler starts")
}

fn run_zero_arity(scheduler: &Scheduler, module: Atom, function: Atom) -> Term {
    let pid = scheduler
        .spawn(module, function, Vec::new())
        .expect("spawn 0-arity entry");
    let (reason, result) = scheduler.run_until_exit(pid);
    assert_eq!(reason, ExitReason::Normal, "entry must exit normally");
    result.root()
}

/// A2 red->green: a multi-clause frameless function whose `select_val` fails to
/// the func_info prelude label compiles through the demand path and is
/// differential-equal to the interpreter-only composition.
#[test]
fn frameless_multi_clause_compiles_through_the_demand_path_and_matches_minimal() {
    let atoms = Arc::new(AtomTable::with_common_atoms());
    let run = atoms.intern("run");
    let loop_fn = atoms.intern("loop");
    let expected = Term::atom(atoms.intern("won"));

    let jit = scheduler(&atoms, SchedulerServices::from_config());
    let load = jit.hot_load_module(FRAMELESS).expect("frameless hot-loads");
    let module = load.module_name;
    let generation = load.generation;

    assert_eq!(run_zero_arity(&jit, module, run), expected);
    assert!(
        wait_until(|| jit.jit_profiler().compile_outcome_counters().successes == 1),
        "loop/2 must compile (A2: func_info prelude retained -> select_val fail resolves)"
    );
    assert!(
        jit.jit_cache()
            .lookup(module, loop_fn, 2, generation)
            .is_some(),
        "loop/2 is cached native after the wired compile"
    );
    let jit_result = run_zero_arity(&jit, module, run);
    jit.shutdown();

    let minimal = scheduler(&atoms, SchedulerServices::minimal());
    let minimal_module = minimal
        .hot_load_module(FRAMELESS)
        .expect("frameless hot-loads (minimal)")
        .module_name;
    let reference = run_zero_arity(&minimal, minimal_module, run);
    minimal.shutdown();

    assert_eq!(jit_result, reference, "JIT result equals interpreter-only");
    assert_eq!(reference, expected);
}

/// Label half (A2) red->green AND frame half honestly pinned: prev/1 (3-clause
/// frameless) compiles; loop/2 (frame + Y-live-across a body-position
/// `?MODULE:prev` call) stays walled — the continuation model is Leg 3's.
#[test]
fn real_function_prev_compiles_and_loop_stays_an_honest_frame_rejection() {
    let atoms = Arc::new(AtomTable::with_common_atoms());
    let run = atoms.intern("run");
    let prev = atoms.intern("prev");
    let loop_fn = atoms.intern("loop");

    let jit = scheduler(&atoms, SchedulerServices::from_config());
    let load = jit
        .hot_load_module(REAL_FUNCTION)
        .expect("jit_real_function hot-loads");
    let module = load.module_name;
    let generation = load.generation;

    let jit_result = run_zero_arity(&jit, module, run);

    // Drive run/0 repeatedly until BOTH verdicts settle: prev/1 and loop/2 compete
    // for the single dirty-CPU worker, so a submission may be transiently refused
    // (queue full) and must re-heat. The label half (prev/1) compiles through the
    // external ?MODULE:prev edge; the frame half (loop/2, a body-position call_ext
    // ahead of its tail self-call) stays an honest R3 body-call rejection.
    assert!(
        wait_until(|| {
            let _ = run_zero_arity(&jit, module, run);
            jit.jit_cache()
                .lookup(module, prev, 1, generation)
                .is_some()
                && jit.jit_profiler().is_unsupported(module, loop_fn, 2)
        }),
        "prev/1 must compile (label half) and loop/2 must stay walled (frame half is Leg 3's)"
    );
    assert!(
        jit.jit_cache()
            .lookup(module, loop_fn, 2, generation)
            .is_none(),
        "a walled function must not be cached native"
    );
    let jit_result2 = run_zero_arity(&jit, module, run);
    jit.shutdown();

    let minimal = scheduler(&atoms, SchedulerServices::minimal());
    let minimal_module = minimal
        .hot_load_module(REAL_FUNCTION)
        .expect("jit_real_function hot-loads (minimal)")
        .module_name;
    let reference = run_zero_arity(&minimal, minimal_module, run);
    minimal.shutdown();

    assert_eq!(
        jit_result, jit_result2,
        "post-compile result is stable across runs"
    );
    assert_eq!(
        jit_result2, reference,
        "JIT (prev/1 native, loop/2 interpreted) equals interpreter-only"
    );
}
