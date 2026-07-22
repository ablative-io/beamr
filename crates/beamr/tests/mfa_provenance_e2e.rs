//! current_mfa lane — fail-first #2: stacktrace-HEAD provenance for a nested raise.
//!
//! `Process::current_mfa` had one production writer (func_info, reached only on a
//! failed multi-clause dispatch) and was read into the TOP raw-stacktrace entry.
//! A process that recovered from a `function_clause` and then raised elsewhere
//! mis-attributed the new raise to the stale failing function.
//!
//! `mfa_provenance:top_fun/0` swallows a `function_clause` (seeding the stale MFA
//! `{mfa_provenance, miss, 1}` on `main`) and then drives `f/1 -> g/1` where
//! `g/1` badmatches, returning the FUNCTION atom of the stacktrace head. Fail-first
//! RED (pre-derivation): the head reports the stale `miss`. GREEN (derive-at-read):
//! the head reports the true raising function `g`.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use beamr::atom::AtomTable;
use beamr::ets::OwnedTerm;
use beamr::loader::load_module;
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;
use beamr::process::ExitReason;
use beamr::term::Term;
use beamr::scheduler::{Scheduler, SchedulerConfig};

const DEADLINE: Duration = Duration::from_secs(5);

fn start(atoms: &AtomTable) -> Scheduler {
    let bifs = BifRegistryImpl::new();
    let registry = Arc::new(ModuleRegistry::new());
    load_module(
        include_bytes!("fixtures/mfa_provenance.beam"),
        atoms,
        &registry,
        &bifs,
    )
    .expect("mfa_provenance fixture loads");
    Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .expect("scheduler starts")
}

#[test]
fn stacktrace_head_attributes_the_true_raising_function_not_a_stale_mfa() {
    let atoms = AtomTable::with_common_atoms();
    let scheduler = Arc::new(start(&atoms));
    let module = atoms.intern("mfa_provenance");
    let top_fun = atoms.intern("top_fun");
    let g = atoms.intern("g");

    let pid = scheduler
        .spawn(module, top_fun, vec![])
        .expect("spawn top_fun/0");
    let (tx, rx) = mpsc::channel::<(ExitReason, OwnedTerm)>();
    let sched = Arc::clone(&scheduler);
    std::thread::spawn(move || {
        let _ = tx.send(sched.run_until_exit(pid));
    });
    let (reason, result) = rx
        .recv_timeout(DEADLINE)
        .unwrap_or_else(|_| panic!("top_fun/0 did not terminate within {DEADLINE:?}"));
    scheduler.shutdown();

    assert_eq!(
        reason,
        ExitReason::Normal,
        "top_fun/0 catches its own nested raise and returns the head function atom"
    );
    assert_eq!(
        result.root(),
        Term::atom(g),
        "the stacktrace HEAD of the badmatch raised in g/1 must attribute to `g`, \
         not the stale `miss` left by the earlier recovered function_clause"
    );
}
