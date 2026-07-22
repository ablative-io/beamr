//! End-to-end proof that `Ref = monitor(process, Pid)` then
//! `receive {'DOWN', Ref, ...}` MATCHES at the receive level, on the real
//! interpreter + scheduler + supervision path.
//!
//! This is the exact hot path the boxed-reference landing unblocks
//! (gleam_otp `actor.call`): a watcher establishes a monitor and selective-
//! receives the bound reference. Before the fix, `monitor/2` returned a small
//! int while the delivered `{'DOWN', Ref, process, Pid, Reason}` carried a
//! boxed reference — different term ranks that never compare equal — so the
//! receive never matched and fell through to its after-branch. Compiled from
//! `fixtures/monitor_down_probe.erl` with OTP-29 erlc (`.erl` + `.beam`
//! committed together, per the probe-workload fixture precedent).

use std::sync::Arc;

use beamr::atom::AtomTable;
use beamr::loader::load_module;
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;
use beamr::native::bifs::register_gate1_bifs;
use beamr::native::gate3_bifs::register_gate3_bifs;
use beamr::native::process_bifs::register_gate2_bifs;
use beamr::process::ExitReason;
use beamr::scheduler::{Scheduler, SchedulerConfig};
use beamr::term::Term;

const PROBE_BEAM: &[u8] = include_bytes!("fixtures/monitor_down_probe.beam");

fn bif_registry(atom_table: &AtomTable) -> BifRegistryImpl {
    let registry = BifRegistryImpl::new();
    register_gate1_bifs(&registry, atom_table).expect("gate1 bifs register");
    register_gate2_bifs(&registry, atom_table).expect("gate2 bifs register");
    register_gate3_bifs(&registry, atom_table).expect("gate3 bifs register");
    registry
}

/// A watcher that monitors an already-dead target reaches its
/// `{'DOWN', Ref, ...}` clause — proving monitor/2's return term-matches the
/// DOWN message reference at the receive level.
#[test]
fn monitor_then_receive_down_matches_on_bound_ref() {
    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let bifs = bif_registry(&atom_table);
    let module_registry = Arc::new(ModuleRegistry::new());

    let (_module, unresolved) =
        load_module(PROBE_BEAM, &atom_table, &module_registry, &bifs).expect("probe fixture loads");
    assert!(
        unresolved.is_empty(),
        "monitor_down_probe has unresolved imports (interpreter must dispatch every opcode): {unresolved}"
    );

    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            jit_threshold: None,
            ..SchedulerConfig::default()
        },
        Arc::clone(&module_registry),
    )
    .expect("scheduler starts");

    let probe = atom_table.intern("monitor_down_probe");

    // Run the target to completion so the watcher monitors an already-dead pid.
    // `run_until_exit` returns only once the exit tombstone is written — the
    // exact store the immediate-DOWN monitor path keys off — so the DOWN is
    // deterministically delivered without any sleep or poll of our own.
    let target = scheduler
        .spawn(probe, atom_table.intern("target"), Vec::new())
        .expect("spawn monitor_down_probe:target/0");
    let (target_reason, _target_result) = scheduler.run_until_exit(target);
    assert_eq!(target_reason, ExitReason::Normal, "target exits normally");

    // The watcher: monitor(process, DeadTarget), then receive the bound DOWN.
    let watcher = scheduler
        .spawn(probe, atom_table.intern("watch"), vec![Term::pid(target)])
        .expect("spawn monitor_down_probe:watch/1");
    let (watcher_reason, watcher_result) = scheduler.run_until_exit(watcher);
    let exit_error = scheduler.take_exit_error(watcher);
    let exit_exception = scheduler.take_exit_exception(watcher);
    scheduler.shutdown();

    assert_eq!(
        watcher_reason,
        ExitReason::Normal,
        "watcher exits normally; error: {exit_error:?}, exception: {exit_exception:?}"
    );
    assert_eq!(
        watcher_result.root(),
        Term::atom(atom_table.intern("matched")),
        "watcher must reach the {{'DOWN', Ref, ...}} clause (got the after-branch \
         `no_match`: monitor/2's return never term-matched the DOWN reference)"
    );
}
