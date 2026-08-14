//! #104 positive control: `spawn_process` enters at raw instruction 0, which
//! on a loader-produced module is the first function's `func_info` landing
//! pad — so the process dies with `error:function_clause` before any body
//! runs. The doc says so by name; this pin goes red the day ip-0 entry
//! becomes survivable (entry resolution, pad relocation, or a death with a
//! different shape), forcing the doc back into truth in the same change.

use std::sync::Arc;

use beamr::atom::AtomTable;
use beamr::loader::load_module;
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;
use beamr::native::bifs::register_gate1_bifs;
use beamr::native::gate3_bifs::register_gate3_bifs;
use beamr::native::process_bifs::register_gate2_bifs;
use beamr::native::stdlib_stubs::register_stdlib_stubs;
use beamr::process::ExitReason;
use beamr::scheduler::{NativeBifs, Scheduler, SchedulerConfig};
use beamr::term::Term;

#[test]
fn spawn_process_on_a_real_module_dies_on_the_landing_pad_while_spawn_runs_it() {
    let atoms = AtomTable::with_common_atoms();
    let bifs = BifRegistryImpl::new();
    register_gate1_bifs(&bifs, &atoms).expect("gate1 bifs register");
    register_gate2_bifs(&bifs, &atoms).expect("gate2 bifs register");
    register_gate3_bifs(&bifs, &atoms).expect("gate3 bifs register");
    register_stdlib_stubs(&bifs, &atoms).expect("stdlib stubs register");
    let registry = Arc::new(ModuleRegistry::new());
    let (module, _report) = load_module(
        include_bytes!("fixtures/proof.beam"),
        &atoms,
        &registry,
        &bifs,
    )
    .expect("proof.beam loads");
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::registry(Arc::new(bifs)),
    )
    .expect("scheduler starts");

    // Contrast arm: the module itself is fine — the exported entry runs to a
    // normal exit through the label-resolving spawn path.
    let entry_pid = scheduler
        .spawn(
            atoms.intern("proof"),
            atoms.intern("fibonacci"),
            vec![Term::small_int(10)],
        )
        .expect("spawn proof:fibonacci/1");
    let (entry_reason, entry_result) = scheduler.run_until_exit(entry_pid);
    assert_eq!(
        entry_reason,
        ExitReason::Normal,
        "proof:fibonacci(10) must succeed"
    );
    assert_eq!(entry_result.root(), Term::small_int(55));

    // Death arm: raw ip-0 entry on the SAME module walks Label/Line into the
    // first function's func_info landing pad. The documented shape is
    // specific: Exited(Error, nil) with an error:function_clause exception —
    // not merely "the process died".
    let pad_pid = scheduler.spawn_process(&module);
    let (pad_reason, pad_result) = scheduler.run_until_exit(pad_pid);
    let pad_exception = scheduler.take_exit_exception(pad_pid);
    scheduler.shutdown();

    assert_eq!(
        pad_reason,
        ExitReason::Error,
        "ip-0 entry on a loader-produced module must die on the landing pad"
    );
    assert_eq!(pad_result.root(), Term::NIL);
    let formatted = pad_exception
        .expect("the landing-pad death must surface its exception")
        .format_with_atoms(&atoms);
    assert!(
        formatted.contains("function_clause"),
        "the death must be the pad's own error:function_clause, got: {formatted}"
    );
}
