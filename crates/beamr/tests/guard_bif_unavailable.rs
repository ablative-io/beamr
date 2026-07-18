//! The attribution-artifact wall (EMB-001 R4).
//!
//! The 2026-07-18 gc_bif attribution cost a multi-seat day because a frame-host
//! child died at its first arithmetic instruction with a single static string
//! (`InvalidOperand("guard bif native import")`) that named neither the failing
//! MFA nor the resolution state. The true cause was `Scheduler::with_services`
//! installing an EMPTY BIF registry, so `erlang:'+'/2` resolved `Deferred`.
//!
//! This is that attribution become the permanent both-directions wall: load the
//! typed-operand arithmetic fixture against an EMPTY registry and assert the
//! refusal now names `erlang:+/2` AND the `Deferred` resolution; load it against
//! a gate1-populated registry and assert the arithmetic runs clean.

use std::sync::Arc;
use std::time::Duration;

use beamr::atom::AtomTable;
use beamr::error::{ExecError, GuardBifResolution};
use beamr::ets::copy::OwnedTerm;
use beamr::loader::load_module;
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;
use beamr::native::bifs::register_gate1_bifs;
use beamr::process::ExitReason;
use beamr::scheduler::{Scheduler, SchedulerConfig};
use beamr::term::Term;

const FIXTURE: &[u8] = include_bytes!("fixtures/guard_bif_probe.beam");

/// The exact one-log-line refusal the 2026-07-18 sanction demanded, as landed.
const EXPECTED_DISPLAY: &str = "guard bif erlang:+/2 unavailable: import resolved Deferred \
    (native BIF registry has no entry and the target module is not loaded)";

/// spawn -> mailbox delivery has a visibility window: `send_to_mailbox` returns
/// `NoSuchProcess` until a worker first schedules the process. Sleep past it.
const SPAWN_VISIBILITY_DELAY: Duration = Duration::from_millis(100);

fn start_scheduler(atoms: &AtomTable, bifs: &BifRegistryImpl) -> (Scheduler, Arc<ModuleRegistry>) {
    let registry = Arc::new(ModuleRegistry::new());
    load_module(FIXTURE, atoms, &registry, bifs).expect("guard_bif_probe fixture loads");
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .expect("scheduler starts");
    (scheduler, registry)
}

fn spawn_probe(scheduler: &Scheduler, atoms: &AtomTable) -> u64 {
    let pid = scheduler
        .spawn(atoms.intern("guard_bif_probe"), atoms.intern("run"), vec![])
        .expect("spawn guard_bif_probe:run/0");
    std::thread::sleep(SPAWN_VISIBILITY_DELAY);
    pid
}

fn send_int(scheduler: &Scheduler, pid: u64, value: i64) {
    scheduler
        .send_to_mailbox(pid, OwnedTerm::immediate(Term::small_int(value)))
        .expect("mailbox admits the small-int message");
}

#[test]
fn empty_registry_refusal_names_the_mfa_and_deferred_resolution() {
    let atoms = AtomTable::with_common_atoms();
    // The exact 2026-07-18 composition: no native BIFs registered at all.
    let bifs = BifRegistryImpl::new();
    let (scheduler, _registry) = start_scheduler(&atoms, &bifs);

    let pid = spawn_probe(&scheduler, &atoms);
    // The `bump` message drives the `Observed + 1` gc_bif clause.
    send_int(&scheduler, pid, 7);
    let (reason, _result) = scheduler.run_until_exit(pid);
    let exit_error = scheduler.take_exit_error(pid);
    scheduler.shutdown();

    assert_eq!(
        reason,
        ExitReason::Error,
        "the guard-bif refusal is process-fatal at first arithmetic execution"
    );
    let exit_error = exit_error.expect("the fatal exit retains its ExecError");
    match exit_error {
        ExecError::GuardBifUnavailable {
            arity, resolution, ..
        } => {
            assert_eq!(arity, 2, "erlang:'+'/2");
            assert_eq!(
                resolution,
                GuardBifResolution::Deferred,
                "empty registry + unloaded target module = Deferred"
            );
        }
        other => panic!("expected GuardBifUnavailable, got {other:?}"),
    }
    assert_eq!(
        exit_error.format_with_atoms(&atoms),
        EXPECTED_DISPLAY,
        "the refusal renders the sanction's one-log-line standard"
    );
}

#[test]
fn populated_registry_runs_the_arithmetic_clean() {
    let atoms = AtomTable::with_common_atoms();
    let bifs = BifRegistryImpl::new();
    register_gate1_bifs(&bifs, &atoms).expect("gate1 bifs register");
    let (scheduler, _registry) = start_scheduler(&atoms, &bifs);

    let pid = spawn_probe(&scheduler, &atoms);
    // Two `bump`s (Observed -> 2), then `report` returns the accumulator.
    send_int(&scheduler, pid, 7);
    send_int(&scheduler, pid, 7);
    send_int(&scheduler, pid, 2);
    let (reason, result) = scheduler.run_until_exit(pid);
    let exit_error = scheduler.take_exit_error(pid);
    scheduler.shutdown();

    assert_eq!(
        reason,
        ExitReason::Normal,
        "a populated registry resolves erlang:'+'/2 Native; no refusal"
    );
    assert!(
        exit_error.is_none(),
        "clean arithmetic leaves no exit error: {:?}",
        exit_error.map(|error| error.format_with_atoms(&atoms))
    );
    assert_eq!(
        result.root(),
        Term::small_int(2),
        "the observable arithmetic result: 0 + 1 + 1 = 2"
    );
}
