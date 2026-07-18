//! The composition wall (EMB-002 R3).
//!
//! Pins the footgun's observable surface in both ruled shapes. Shape B: a
//! hot-load of a module importing `erlang:'+'/2` against the empty-registry
//! default composition returns a `HotLoadResult` whose report flags the import
//! under `deferred_by_module` — the exact signal the 2026-07-18 incident
//! needed, now readable at the API frame uses. Shape A: that same module then
//! refuses at execution exactly as the constructor docs warn, so the doc
//! warning is proven honest, not merely written. The gate1-populated twin
//! returns no deferred `erlang:*` — the both-directions confirm, walled.
//!
//! Reuses the EMB-001 fixture module (`guard_bif_probe.beam`, a typed-operand
//! arithmetic receive loop). A shared atom table is threaded through
//! `with_services_and_code_server` so the empty-registry composition is
//! reproduced with a table the assertions can resolve `erlang` / `'+'` against.

use std::sync::Arc;
use std::time::Duration;

use beamr::atom::{Atom, AtomTable};
use beamr::error::{ExecError, GuardBifResolution};
use beamr::ets::copy::OwnedTerm;
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;
use beamr::native::bifs::register_gate1_bifs;
use beamr::process::ExitReason;
use beamr::scheduler::{HotLoadResult, Scheduler, SchedulerConfig, SchedulerServices};
use beamr::term::Term;

const FIXTURE: &[u8] = include_bytes!("fixtures/guard_bif_probe.beam");

const SPAWN_VISIBILITY_DELAY: Duration = Duration::from_millis(100);

/// Build a scheduler over an explicit atom table and BIF registry — the
/// composition analogue an embedder uses to control what is registered.
fn scheduler_with(atoms: &Arc<AtomTable>, bifs: BifRegistryImpl) -> Scheduler {
    Scheduler::with_services_and_code_server(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        SchedulerServices::minimal(),
        Arc::new(ModuleRegistry::new()),
        Arc::clone(atoms),
        Arc::new(bifs),
    )
    .expect("scheduler starts")
}

fn deferred_has_plus_2(result: &HotLoadResult, erlang: Atom, plus: Atom) -> bool {
    result
        .unresolved
        .deferred_for(erlang)
        .iter()
        .any(|entry| entry.function == plus && entry.arity == 2)
}

#[test]
fn empty_registry_hot_load_report_flags_deferred_erlang_and_refuses_at_execution() {
    let atoms = Arc::new(AtomTable::with_common_atoms());
    // The footgun composition: services asked for, ZERO native BIFs registered.
    let scheduler = scheduler_with(&atoms, BifRegistryImpl::new());

    // Shape B: the load report already flags erlang:'+'/2 as deferred.
    let result = scheduler
        .hot_load_module(FIXTURE)
        .expect("guard_bif_probe hot-loads (deferred imports stay legal at load)");
    let erlang = atoms.intern("erlang");
    let plus = atoms.intern("+");
    assert!(
        result.unresolved.has_deferred(),
        "empty registry defers the module's erlang:* imports"
    );
    assert!(
        deferred_has_plus_2(&result, erlang, plus),
        "the exact 2026-07-18 signal: erlang:'+'/2 under deferred_by_module, \
         readable at the hot-load API"
    );

    // Shape A: the docs' warning is honest — the module refuses at execution.
    let pid = scheduler
        .spawn(atoms.intern("guard_bif_probe"), atoms.intern("run"), vec![])
        .expect("spawn guard_bif_probe:run/0");
    std::thread::sleep(SPAWN_VISIBILITY_DELAY);
    scheduler
        .send_to_mailbox(pid, OwnedTerm::immediate(Term::small_int(7)))
        .expect("mailbox admits the bump message");
    let (reason, _result) = scheduler.run_until_exit(pid);
    let exit_error = scheduler.take_exit_error(pid);
    scheduler.shutdown();

    assert_eq!(reason, ExitReason::Error, "the refusal is process-fatal");
    match exit_error.expect("the fatal exit retains its ExecError") {
        ExecError::GuardBifUnavailable { resolution, .. } => assert_eq!(
            resolution,
            GuardBifResolution::Deferred,
            "the report's deferred signal and the execution refusal agree"
        ),
        other => panic!("expected GuardBifUnavailable, got {other:?}"),
    }
}

#[test]
fn gate1_registry_hot_load_report_has_no_deferred_plus() {
    let atoms = Arc::new(AtomTable::with_common_atoms());
    let bifs = BifRegistryImpl::new();
    register_gate1_bifs(&bifs, &atoms).expect("gate1 bifs register");
    let scheduler = scheduler_with(&atoms, bifs);

    let result = scheduler
        .hot_load_module(FIXTURE)
        .expect("guard_bif_probe hot-loads under gate1");
    let erlang = atoms.intern("erlang");
    let plus = atoms.intern("+");
    scheduler.shutdown();

    assert!(
        !deferred_has_plus_2(&result, erlang, plus),
        "a gate1-populated composition resolves erlang:'+'/2 Native — not deferred"
    );
}
