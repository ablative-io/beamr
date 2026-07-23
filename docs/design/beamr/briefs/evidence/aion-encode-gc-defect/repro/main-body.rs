// Shared harness body for the AION-ENCODE-GC-DEFECT signature repro.
//
// Included verbatim by red/src/main.rs (beamr =0.16.0) and
// green/src/main.rs (beamr =0.16.2); each leg defines LEG before the
// include. The VM setup mirrors aion's embedding
// (crates/aion/src/runtime/handle.rs at aion main): shared AtomTable +
// BifRegistryImpl handed to Scheduler::with_code_server, registration
// order gate1 → gate3 → stdlib stubs → gleam ffi → otp stubs. aion's
// engine-NIF replacements are omitted — the fixture spawns no
// processes and calls no engine NIFs.
//
// Exit codes: 0 = fixture completed (green), 2 = fixture process exited
// abnormally with the VM intact (reason prints), 3 = harness setup
// failure (not evidence). The committed red never reaches any of these:
// at 0.16.0 the process dies by SIGSEGV (shell reports 139) inside the
// encode's collection, before completion output — see the run record.

use std::sync::Arc;

use beamr::atom::AtomTable;
use beamr::loader::load_module;
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;
use beamr::native::bifs::register_gate1_bifs;
use beamr::native::gate3_bifs::register_gate3_bifs;
use beamr::native::gleam_ffi::register_gleam_ffi_bifs;
use beamr::native::otp_stubs::{init_otp_atoms, register_otp_stubs};
use beamr::native::stdlib_stubs::register_stdlib_stubs;
use beamr::process::ExitReason;
use beamr::scheduler::{Scheduler, SchedulerConfig};

fn run() -> i32 {
    let beam_path = match std::env::args().nth(1) {
        Some(path) => path,
        None => {
            eprintln!("usage: {LEG} <path-to-encode_gc_repro.beam>");
            return 3;
        }
    };
    let bytes = match std::fs::read(&beam_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("{LEG}: cannot read {beam_path}: {error}");
            return 3;
        }
    };

    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let module_registry = Arc::new(ModuleRegistry::new());
    let bif_registry = Arc::new(BifRegistryImpl::new());
    register_gate1_bifs(&bif_registry, &atom_table).expect("register gate1 bifs");
    register_gate3_bifs(&bif_registry, &atom_table).expect("register gate3 bifs");
    register_stdlib_stubs(&bif_registry, &atom_table).expect("register stdlib stubs");
    register_gleam_ffi_bifs(&bif_registry, &atom_table).expect("register gleam ffi bifs");
    init_otp_atoms(&atom_table);
    register_otp_stubs(&bif_registry, &atom_table).expect("register otp stubs");

    let (module, unresolved) =
        load_module(&bytes, &atom_table, &module_registry, &*bif_registry)
            .expect("load fixture module");
    if !unresolved.is_empty() {
        eprintln!("{LEG}: unresolved imports: {unresolved:?}");
    }

    let scheduler = Scheduler::with_code_server(
        SchedulerConfig {
            thread_count: Some(1),
            ..Default::default()
        },
        Arc::clone(&module_registry),
        Arc::clone(&atom_table),
        Arc::clone(&bif_registry),
    )
    .expect("construct scheduler");

    let entry = atom_table.intern("main");
    let pid = scheduler
        .spawn(module.name, entry, vec![])
        .expect("spawn fixture entry");
    let (reason, result) = scheduler.run_until_exit(pid);
    scheduler.shutdown();

    println!("leg: {LEG}");
    println!("exit_reason: {reason:?}");
    println!("result: {result:?}");
    match reason {
        ExitReason::Normal => 0,
        _ => 2,
    }
}

fn main() {
    std::process::exit(run());
}
