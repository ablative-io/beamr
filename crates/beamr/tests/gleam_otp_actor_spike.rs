//! OTP-actor spike: a real `gleam_otp` v1.2.0 actor runs on beamr and completes
//! an `actor.call` round-trip.
//!
//! The fixture `tests/fixtures/gleam_otp_spike/` is a `gleam new` project
//! (gleam 1.17.0, gleam_otp 1.2.0, gleam_erlang 1.3.0, gleam_stdlib 1.0.3). Its
//! `actor_spike:run/0`:
//!   1. starts a counter actor via `actor.new |> actor.on_message |> actor.start`
//!      (which `process.spawn`s the actor — `proc_lib:spawn_link/1` over a
//!      capturing closure — `process.monitor`s it, and `selector_receive`s the
//!      init ack on the bound monitor reference),
//!   2. fires two `actor.send` casts (`Inc`) — cross-process local sends,
//!   3. does a synchronous `actor.call` for the count — `process.call` monitors
//!      the actor, sends `Get(reply_subject)`, and selective-receives the reply
//!      on the bound monitor reference (the exact boxed-reference hot path that
//!      `monitor_down_e2e.rs` unblocked),
//!   4. returns the observed count (`2`).
//!
//! `run_until_exit` blocks until the process writes its exit tombstone — no
//! sleep, no busy-poll (NO-POLLING is design law); the actor's own bounded
//! receives guard liveness. Asserting `run/0` returns `2` proves the call
//! round-trip completed across two real scheduler processes. `beams/` holds
//! exactly the module-closure of `actor_spike:run/0` (computed with `beam_lib`),
//! committed per the house pre-compiled-fixture convention.
//!
//! NOTE: the native `gleam_erlang_ffi` selector shadow has been RETIRED (it was
//! pinned to an older gleam_erlang selector protocol — bare-tag matching, a bare
//! `select` result — incompatible with gleam_erlang 1.3.0's `#(tag, arity)` keys
//! and `Result`-wrapped `selector_receive`). No BIF registry registers it any
//! more, so the real, loaded `gleam_erlang_ffi.beam` bytecode serves `select/1,2`
//! etc. (a plain `receive` with `is_map_key`/`element`/`tuple_size` guards over a
//! handler map) — the faithful, version-matched path. `selector_family_absent...`
//! below pins the honest-undef behaviour when neither shadow nor bytecode serve
//! the family. See the handoff for the retirement migration table.

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
use beamr::term::Term;

fn full_bif_registry(atom_table: &AtomTable) -> BifRegistryImpl {
    let registry = BifRegistryImpl::new();
    register_gate1_bifs(&registry, atom_table).expect("gate1");
    register_gate2_bifs(&registry, atom_table).expect("gate2");
    register_gate3_bifs(&registry, atom_table).expect("gate3");
    register_stdlib_stubs(&registry, atom_table).expect("stdlib");
    // The native `gleam_erlang_ffi` selector shadow has been RETIRED (it was
    // pinned to an older gleam_erlang selector protocol and silently mismatched
    // gleam_erlang >= 1.3). The selector family is now served by the loaded
    // `gleam_erlang_ffi.beam` bytecode — the faithful, version-matched path.
    register_gleam_ffi_bifs(&registry, atom_table).expect("gleam_ffi");
    register_meridian_ffi(&registry, atom_table).expect("meridian_ffi");
    init_otp_atoms(atom_table);
    register_otp_stubs(&registry, atom_table).expect("otp_stubs");
    registry
}

/// Load the committed actor-spike beams (optionally skipping one, by file name)
/// into a fresh module registry and boot a single-threaded scheduler wired to
/// the shared atom + BIF tables (so cross-module dispatch and export funs
/// resolve).
fn load_and_boot(
    atom_table: &Arc<AtomTable>,
    module_registry: &Arc<ModuleRegistry>,
    bif_registry: Arc<BifRegistryImpl>,
    exclude: Option<&str>,
) -> Scheduler {
    let beams_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gleam_otp_spike/beams");
    assert!(
        beams_dir.is_dir(),
        "missing committed actor-spike beams at {}",
        beams_dir.display()
    );
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&beams_dir)
        .expect("read beams dir")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "beam"))
        .filter(|path| {
            exclude.is_none_or(|name| path.file_name().and_then(|n| n.to_str()) != Some(name))
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no .beam files in beams dir");
    for path in &paths {
        let bytes = std::fs::read(path).expect("read fixture beam");
        load_module(&bytes, atom_table, module_registry, &*bif_registry)
            .unwrap_or_else(|err| panic!("failed to load {}: {err}", path.display()));
    }
    Scheduler::with_code_server(
        SchedulerConfig {
            thread_count: Some(1),
            jit_threshold: None,
            ..SchedulerConfig::default()
        },
        Arc::clone(module_registry),
        Arc::clone(atom_table),
        bif_registry,
    )
    .expect("scheduler starts")
}

/// Spawn `actor_spike:MODULE_ENTRY/0` (loading all committed beams), run it to
/// exit, and return its exit reason plus the value it returned (deep-copied at
/// the exit boundary).
fn run_entry(entry: &str) -> (ExitReason, Term, Option<String>) {
    run_entry_inner(entry, None)
}

/// As [`run_entry`], but skips the named fixture beam, so its exports resolve
/// neither to the (retired) native shadow nor to loaded bytecode.
fn run_entry_without(entry: &str, exclude: &str) -> (ExitReason, Term, Option<String>) {
    run_entry_inner(entry, Some(exclude))
}

fn run_entry_inner(entry: &str, exclude: Option<&str>) -> (ExitReason, Term, Option<String>) {
    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let bif_registry = Arc::new(full_bif_registry(&atom_table));
    let module_registry = Arc::new(ModuleRegistry::new());
    let scheduler = load_and_boot(&atom_table, &module_registry, bif_registry, exclude);

    let pid = scheduler
        .spawn(
            atom_table.intern("actor_spike"),
            atom_table.intern(entry),
            vec![],
        )
        .unwrap_or_else(|err| panic!("spawn actor_spike:{entry}/0: {err:?}"));
    let (reason, result) = scheduler.run_until_exit(pid);
    let exit_exception = scheduler.take_exit_exception(pid);
    let exit_error = scheduler.take_exit_error(pid);
    scheduler.shutdown();

    let diagnostic = exit_exception
        .map(|exception| exception.format_with_atoms(&atom_table))
        .or_else(|| exit_error.map(|error| error.format_with_atoms(&atom_table)));
    (reason, result.root(), diagnostic)
}

/// A real `gleam_otp` actor starts, takes two casts, and answers a synchronous
/// `actor.call` — the reply travels back across the monitored selective-receive
/// path and `run/0` returns the observed count.
#[test]
fn gleam_otp_actor_call_round_trip_returns_count() {
    let (reason, result, diagnostic) = run_entry("run");
    assert_eq!(
        reason,
        ExitReason::Normal,
        "actor_spike:run/0 must exit normally; diagnostic: {diagnostic:?}"
    );
    assert_eq!(
        result,
        Term::small_int(2),
        "actor.call must return the count observed after two casts"
    );
}

/// Hazard fail-first (GREEN side): cross-process local send + the gleam_erlang
/// 1.3.0 compound-key selector path — a spawned closure captures a subject,
/// sends across the process boundary, and the parent `selector_receive`s the
/// value. With the native selector shadow RETIRED, the loaded gleam_erlang_ffi
/// bytecode serves `select` and this returns the correct 77. (Pre-retirement,
/// with the shadow registered, this returned `error:badarg` — the shadow's bare
/// `select` result fails the `Result` match; see the retirement commit.)
#[test]
fn spawned_closure_subject_send_selector_receives() {
    let (reason, result, diagnostic) = run_entry("subject_probe");
    assert_eq!(
        reason,
        ExitReason::Normal,
        "subject_probe/0 must exit normally; diagnostic: {diagnostic:?}"
    );
    assert_eq!(
        result,
        Term::small_int(77),
        "selector must receive the cross-process send"
    );
}

/// Hazard fail-first (honest-undef side): with the native shadow retired AND the
/// `gleam_erlang_ffi.beam` bytecode NOT loaded, the selector family resolves to
/// neither — so exercising it raises a catchable `error:undef`, not a
/// silently-wrong value. This pins that the retirement replaced silently-wrong
/// shapes with an honest, diagnosable failure.
#[test]
fn selector_family_absent_yields_honest_undef() {
    let (reason, _result, diagnostic) = run_entry_without("subject_probe", "gleam_erlang_ffi.beam");
    assert_eq!(
        reason,
        ExitReason::Error,
        "subject_probe/0 must fail (not return a value) when the selector family is unserved; \
         diagnostic: {diagnostic:?}"
    );
    let diagnostic = diagnostic.unwrap_or_default();
    assert!(
        diagnostic.contains("undef") || diagnostic.contains("gleam_erlang_ffi"),
        "the failure must be an honest undef naming the unserved selector function, got: \
         {diagnostic}"
    );
}

/// Regression for cross-process local send received via a plain `{Ref, Message}`
/// receive (no selector) — isolates delivery from selector matching.
#[test]
fn spawned_closure_subject_send_plain_receive() {
    let (reason, result, diagnostic) = run_entry("receive_probe");
    assert_eq!(
        reason,
        ExitReason::Normal,
        "receive_probe/0 must exit normally; diagnostic: {diagnostic:?}"
    );
    assert_eq!(
        result,
        Term::small_int(88),
        "plain receive must get the cross-process send"
    );
}
