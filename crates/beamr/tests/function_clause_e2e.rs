//! ADMISSION ARC LEG 1 PART (i) — the interpreter function_clause defect.
//!
//! A function reached with no matching clause falls through to `func_info` (the
//! multi-clause dispatch landing pad). On `main` `core::func_info` set the MFA and
//! CONTINUED, so control fell back into the body, re-dispatched, missed again, and
//! looped forever — a full-core spin (the coordinator's byte-verified third
//! defect; `ExecError::FunctionClause` was never constructed anywhere in `src`).
//!
//! `func_info` now raises a catchable `error:function_clause` (MFA carried in the
//! stacktrace), mirroring `case_end`/`if_end`. Fail-first RED (pre-fix) =
//! watchdog-bounded non-termination on `probe(b)`. GREEN = `probe(b)` exits
//! `error:function_clause`, and a `try ... catch error:function_clause` in LOADED
//! BYTECODE (`caught/1`) observes the raise and returns `caught`.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use beamr::atom::{Atom, AtomTable};
use beamr::ets::OwnedTerm;
use beamr::loader::load_module;
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;
use beamr::process::ExitReason;
use beamr::scheduler::{Scheduler, SchedulerConfig};
use beamr::term::Term;

const DEADLINE: Duration = Duration::from_secs(5);

fn start(atoms: &AtomTable) -> Scheduler {
    let bifs = BifRegistryImpl::new();
    let registry = Arc::new(ModuleRegistry::new());
    load_module(
        include_bytes!("fixtures/fc_probe.beam"),
        atoms,
        &registry,
        &bifs,
    )
    .expect("fc_probe fixture loads");
    Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .expect("scheduler starts")
}

/// Deadline-bound run (NO-POLLING): spawn `fc_probe:function(arg)` and wait up to
/// `DEADLINE` on a watchdog thread. `None` = the process never terminated (the
/// pre-fix infinite-loop RED); `Some` carries the exit reason and result.
fn run_bounded(
    scheduler: &Arc<Scheduler>,
    function: Atom,
    arg: Term,
    pid_slot: &mut u64,
) -> Option<(ExitReason, OwnedTerm)> {
    let module = scheduler_module();
    let pid = scheduler.spawn(module, function, vec![arg]).expect("spawn");
    *pid_slot = pid;
    let (tx, rx) = mpsc::channel();
    let sched = Arc::clone(scheduler);
    std::thread::spawn(move || {
        let _ = tx.send(sched.run_until_exit(pid));
    });
    rx.recv_timeout(DEADLINE).ok()
}

fn scheduler_module() -> Atom {
    // `fc_probe` is interned at load; re-interning in a fresh common table yields
    // the same id (deterministic ordering), so the tests recompute it locally.
    AtomTable::with_common_atoms().intern("fc_probe")
}

#[test]
fn no_matching_clause_raises_function_clause_instead_of_looping() {
    let atoms = AtomTable::with_common_atoms();
    let scheduler = Arc::new(start(&atoms));
    let probe = atoms.intern("probe");
    let b = Term::atom(atoms.intern("b"));

    let mut pid = 0;
    let (reason, _result) = run_bounded(&scheduler, probe, b, &mut pid).unwrap_or_else(|| {
        panic!("probe(b) did not terminate within {DEADLINE:?} — func_info looped (pre-fix defect)")
    });
    let exception = scheduler.take_exit_exception(pid);
    scheduler.shutdown();

    assert_eq!(
        reason,
        ExitReason::Error,
        "an unmatched function clause is process-fatal"
    );
    let exception = exception.expect("function_clause must surface as an exit exception");
    assert_eq!(
        exception.view().class,
        Term::atom(Atom::ERROR),
        "class must be error"
    );
    assert_eq!(
        exception.view().reason,
        Term::atom(Atom::FUNCTION_CLAUSE),
        "reason must be the function_clause atom"
    );
}

#[test]
fn function_clause_is_catchable_in_loaded_bytecode() {
    let atoms = AtomTable::with_common_atoms();
    let scheduler = Arc::new(start(&atoms));
    let caught_fn = atoms.intern("caught");
    let b = Term::atom(atoms.intern("b"));
    let caught_atom = atoms.intern("caught");

    let mut pid = 0;
    let (reason, result) = run_bounded(&scheduler, caught_fn, b, &mut pid).unwrap_or_else(|| {
        panic!("caught(b) did not terminate within {DEADLINE:?} (pre-fix defect)")
    });
    scheduler.shutdown();

    assert_eq!(
        reason,
        ExitReason::Normal,
        "the bytecode try/catch handles function_clause and exits normally"
    );
    assert_eq!(
        result.root(),
        Term::atom(caught_atom),
        "`try probe(b) catch error:function_clause -> caught` must observe the raise \
         and return `caught` — Rust-side surfacing alone is not evidence"
    );
}
