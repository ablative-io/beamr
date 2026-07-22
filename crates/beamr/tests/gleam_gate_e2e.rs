//! Fresh-Gleam gate test.
//!
//! The fixtures in `tests/fixtures/gleam_gate/` are the unmodified build
//! output of a `gleam new` project (gleam 1.17.0, stdlib 1.0.3,
//! gleam_json 3.x) whose `main/0` exercises list.map/filter/fold,
//! int.to_string passed as a function value (an export fun),
//! string.join/uppercase, JSON encoding and parsing through
//! gleam/dynamic/decode (field/string/int decoders over the compiled
//! gleam_stdlib FFI), io.println, and the stdlib surfaces that used to be
//! (wrongly) served by native shadows and now run as compiled bytecode:
//! int/float.parse, float.to_string, string
//! starts_with/ends_with/contains/replace/slice/pop_grapheme/inspect,
//! UTF codepoint round-trips, dict.get, bit_array
//! slice/concat/base16/base64, and uri.percent_encode/percent_decode.
//! EXPECTED_PAYLOAD is the verbatim output of `gleam run` on real
//! Erlang/OTP — ground truth, not what beamr happens to produce. It runs
//! in the default `cargo test` so a regression in any of those paths
//! fails the suite, not just a manual CLI run.

use std::path::PathBuf;
use std::sync::Arc;

use beamr::atom::AtomTable;
use beamr::loader::load_module;
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;
use beamr::native::bifs::register_gate1_bifs;
use beamr::native::gate3_bifs::register_gate3_bifs;
use beamr::native::gleam_ffi::register_gleam_ffi_bifs;
use beamr::native::meridian_ffi::register_meridian_ffi;
use beamr::native::otp_stubs::{init_otp_atoms, register_otp_stubs};
use beamr::native::process_bifs::register_gate2_bifs;
use beamr::native::stdlib_stubs::register_stdlib_stubs;
use beamr::process::ExitReason;
use beamr::scheduler::{Scheduler, SchedulerConfig};
use beamr::term::binary_ref::BinaryRef;

/// Verbatim stdout of `gleam run` on real Erlang/OTP — the io.println stage
/// markers, the inspect probes, and the final payload line. Asserting the
/// whole stream pins the print path and string.inspect output, and the stage
/// markers bisect any future regression to the failing stdlib area.
const EXPECTED_STDOUT: &str = include_str!("fixtures/gleam_gate/expected_stdout.txt");

#[derive(Default)]
struct RecordingSink(std::sync::Mutex<Vec<u8>>);

impl beamr::io::IoSink for RecordingSink {
    fn write(&self, bytes: &[u8]) {
        self.0.lock().expect("sink lock").extend_from_slice(bytes);
    }
}

const EXPECTED_PAYLOAD: &str = r##"{"doubled":"2,4,6,8,10,12","evens":"2,4,6","total":21,"label":"GATE","decoded":"gate:3","parsed_int":"42","bad_int":"rejected","parsed_float":"5.0","string_checks":"T|T|T|a+b+c|éjà","popped":"é>x","inspected":"#(1, \"two\", True)","codepoints":"101,233","codepoint_round_trip":"eé","dict_hit":"2","dict_miss":"miss","sliced":"ell","joined":"hello!","hex":"68656C6C6F","b64":"aGVsbG8=","b64_round_trip":"hello","encoded":"a%20b%26c","decoded_uri":"a b&c"}"##;

fn full_bif_registry(atom_table: &AtomTable) -> BifRegistryImpl {
    let registry = BifRegistryImpl::new();
    register_gate1_bifs(&registry, atom_table).expect("gate1");
    register_gate2_bifs(&registry, atom_table).expect("gate2");
    register_gate3_bifs(&registry, atom_table).expect("gate3");
    register_stdlib_stubs(&registry, atom_table).expect("stdlib");
    register_gleam_ffi_bifs(&registry, atom_table).expect("gleam_ffi");
    register_meridian_ffi(&registry, atom_table).expect("meridian_ffi");
    init_otp_atoms(atom_table);
    register_otp_stubs(&registry, atom_table).expect("otp_stubs");
    registry
}

#[test]
fn fresh_gleam_project_runs_end_to_end() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gleam_gate");
    assert!(
        fixture_dir.is_dir(),
        "missing committed fixtures at {}",
        fixture_dir.display()
    );

    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let bif_registry = Arc::new(full_bif_registry(&atom_table));
    let module_registry = Arc::new(ModuleRegistry::new());

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&fixture_dir)
        .expect("read fixture dir")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "beam"))
        .collect();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "fixture directory contains no .beam files"
    );
    for path in paths {
        let bytes = std::fs::read(&path).expect("read fixture beam");
        load_module(&bytes, &atom_table, &module_registry, &*bif_registry)
            .unwrap_or_else(|err| panic!("failed to load {}: {err}", path.display()));
    }

    // Share the loading atom table and BIF registry with the scheduler so
    // export funs (`fun M:F/A` values like `int.to_string`) dispatch.
    let scheduler = Scheduler::with_code_server(
        SchedulerConfig {
            thread_count: Some(1),
            jit_threshold: None,
            ..SchedulerConfig::default()
        },
        Arc::clone(&module_registry),
        Arc::clone(&atom_table),
        bif_registry,
    )
    .expect("scheduler starts");
    let sink = Arc::new(RecordingSink::default());
    scheduler.set_output_sink(sink.clone());

    let pid = scheduler
        .spawn(
            atom_table.intern("beamr_gate"),
            atom_table.intern("main"),
            vec![],
        )
        .expect("spawn beamr_gate:main/0");
    let (reason, result) = scheduler.run_until_exit(pid);
    let exit_exception = scheduler.take_exit_exception(pid);
    let exit_error = scheduler.take_exit_error(pid);
    scheduler.shutdown();

    assert_eq!(
        reason,
        ExitReason::Normal,
        "exit_exception: {:?}, exit_error: {:?}",
        exit_exception.map(|exception| exception.format_with_atoms(&atom_table)),
        exit_error.map(|error| error.format_with_atoms(&atom_table))
    );
    let payload = BinaryRef::new(result.root()).expect("main returns the JSON payload binary");
    assert_eq!(
        std::str::from_utf8(payload.as_bytes()).expect("utf8 payload"),
        EXPECTED_PAYLOAD
    );
    let printed = sink.0.lock().expect("sink lock").clone();
    assert_eq!(
        std::str::from_utf8(&printed).expect("utf8 stdout"),
        EXPECTED_STDOUT
    );
}
