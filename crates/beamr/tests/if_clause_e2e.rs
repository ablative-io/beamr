//! ADMISSION ARC LEG 1 — fourth defect: `if_end` raised the wrong reason SHAPE.
//!
//! beamr's `if_end` built the reason as `{if_clause, []}` (tuple-wrapped), but
//! BEAM's if_clause reason is the BARE atom `if_clause` (unlike `badmatch` /
//! `case_clause`, which do carry the offending value). A loaded-bytecode
//! `try ... catch error:if_clause -> caught end` matches on OTP-29 (raw reason =
//! bare `if_clause`) and FAILED TO MATCH on beamr — the exception escaped the
//! try/catch and the process died where BEAM recovers.
//!
//! Fail-first = loaded-bytecode catchability. RED (pre-fix): the catch does not
//! match the tuple reason, the raise escapes, `main_catch/0` dies Error. GREEN
//! (post-fix): the bare-atom reason matches and `main_catch/0` returns `caught`.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use beamr::atom::AtomTable;
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
        include_bytes!("fixtures/if_probe.beam"),
        atoms,
        &registry,
        &bifs,
    )
    .expect("if_probe fixture loads");
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
fn if_clause_is_catchable_in_loaded_bytecode() {
    let atoms = AtomTable::with_common_atoms();
    let scheduler = Arc::new(start(&atoms));
    let module = atoms.intern("if_probe");
    let main_catch = atoms.intern("main_catch");
    let caught = atoms.intern("caught");

    let pid = scheduler
        .spawn(module, main_catch, vec![])
        .expect("spawn main_catch/0");
    let (tx, rx) = mpsc::channel::<(ExitReason, OwnedTerm)>();
    let sched = Arc::clone(&scheduler);
    std::thread::spawn(move || {
        let _ = tx.send(sched.run_until_exit(pid));
    });
    let (reason, result) = rx
        .recv_timeout(DEADLINE)
        .unwrap_or_else(|_| panic!("main_catch/0 did not terminate within {DEADLINE:?}"));
    scheduler.shutdown();

    assert_eq!(
        reason,
        ExitReason::Normal,
        "the bytecode `catch error:if_clause` must handle the raise and exit normally"
    );
    assert_eq!(
        result.root(),
        Term::atom(caught),
        "`try h(id(b)) catch error:if_clause -> caught` must observe the BARE-atom \
         if_clause reason and return `caught` — pre-fix the {{if_clause,[]}} tuple escaped"
    );
}
