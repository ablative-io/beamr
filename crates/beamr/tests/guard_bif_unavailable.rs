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
//!
//! The third arm is the divergence guard (B-178 R9): the loader-time and the
//! runtime registries are DIFFERENT OBJECTS with no provenance recorded on
//! `Module`/`ResolvedImport`, so a refusal that asserts anything about the
//! runtime registry's contents can accuse a correctly-composed caller. That arm
//! builds the divergence on purpose — empty at load, gate1-populated at
//! construction — and walls the refusal text against every claim it cannot
//! substantiate.

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
use beamr::scheduler::{NativeBifs, Scheduler, SchedulerConfig};
use beamr::term::Term;

const FIXTURE: &[u8] = include_bytes!("fixtures/guard_bif_probe.beam");

/// The exact one-log-line refusal the 2026-07-18 sanction demanded, rendered
/// through the runtime atom table (`format_with_atoms`) — the true MFA channel.
const EXPECTED_DISPLAY: &str = "guard bif erlang:+/2 unavailable: import resolved Deferred \
    (the LOAD-TIME native BIF registry had no entry and the target module is not loaded); \
    runtime natives: Wired — imports bind at LOAD time against the loader's registry, and a \
    scheduler declared NativeBifs::none() also reports Wired because none() wires a registry \
    with no BIFs registered; schedulers declare natives at construction (see NativeBifs::none \
    / NativeBifs::registry)";

/// The plain-`Display` fallback channel: `ExecError`'s `Display` resolves atoms
/// through a fresh `AtomTable::with_common_atoms()`, and neither `erlang` nor
/// `+` is a common atom, so both render the shared `#<unknown atom>` fallback
/// token (identical to the `Undef` arm's behaviour). Pinned so both channels
/// are load-bearing; the fold amendment (dff20af) ruled `format_with_atoms` the
/// exact-string carrier and this dual-channel wall its rot-guard.
const EXPECTED_DISPLAY_FALLBACK: &str = "guard bif #<unknown atom>:#<unknown atom>/2 unavailable: import resolved Deferred \
    (the LOAD-TIME native BIF registry had no entry and the target module is not loaded); \
    runtime natives: Wired — imports bind at LOAD time against the loader's registry, and a \
    scheduler declared NativeBifs::none() also reports Wired because none() wires a registry \
    with no BIFs registered; schedulers declare natives at construction (see NativeBifs::none \
    / NativeBifs::registry)";

/// Substrings the guard-BIF refusal SHALL NEVER carry.
///
/// Three classes, all unsubstantiable at the mint point: a claim about what the
/// runtime registry CONTAINS (`BifRegistry` exposes no emptiness predicate, and
/// the registry an import bound against at LOAD time is a different object from
/// the one wired at runtime); a runtime-state token that is FALSE of the
/// divergence scenario — a services bundle reached the dispatch carrying a
/// registry, so neither the absent nor the unwired state applies; and a
/// present-tense claim about whether the TARGET MODULE is loaded.
///
/// The fourth-from-last entry is the arm that was RED before the attribution
/// landed: the old hint said "native BIF registry has no entry" without saying
/// WHICH of the two registries it meant, which is false of the runtime one
/// here. It is banned unqualified, so the "LOAD-TIME" qualifier cannot be
/// reverted quietly.
///
/// The last entry walls the OTHER half of the same sentence, and it is banned
/// for the same reason rather than because it happens to be false here. Import
/// resolution runs exactly once, at load (`loader::load::resolve_imports` is
/// the sole producer of `Deferred`, and nothing re-resolves), while a
/// `Deferred` import is late-bound against the LIVE module registry on the call
/// path (`opcodes::core` resolves it through `ctx.registry` at execution time).
/// "Deferred, and the target module is loaded NOW" is therefore a normal
/// runtime state in this tree — and the mint point, which sees only the
/// IMPORTING module and the services bundle, cannot tell the two apart. Both
/// conjuncts of the hint must be stamped LOAD-TIME or neither is.
///
/// Shortening this list is hollowing the wall, not fixing a test.
const BANNED_ACCUSATIONS: &[&str] = &[
    "empty",
    "your registry",
    "Absent",
    "Unwired",
    "native BIF registry has no entry",
    "the target module is not loaded",
];

/// spawn -> mailbox delivery has a visibility window: `send_to_mailbox` returns
/// `NoSuchProcess` until a worker first schedules the process. Sleep past it.
const SPAWN_VISIBILITY_DELAY: Duration = Duration::from_millis(100);

/// Load the fixture against `bifs` and start a scheduler declaring `natives`.
///
/// The two are separate parameters because the two arms declare differently:
/// the refusal arm's scheduler resolves no natives, the clean arm's carries the
/// same registry the loader bound the imports against.
fn start_scheduler(
    atoms: &AtomTable,
    bifs: &BifRegistryImpl,
    natives: NativeBifs,
) -> (Scheduler, Arc<ModuleRegistry>) {
    let registry = Arc::new(ModuleRegistry::new());
    load_module(FIXTURE, atoms, &registry, bifs).expect("guard_bif_probe fixture loads");
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        natives,
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
    let (scheduler, _registry) = start_scheduler(&atoms, &bifs, NativeBifs::none());

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
    // Channel 1 (the exact-string carrier): format_with_atoms renders the true
    // MFA through the runtime atom table — the sanctioned one-log-line.
    assert_eq!(
        exit_error.format_with_atoms(&atoms),
        EXPECTED_DISPLAY,
        "the refusal renders the sanction's one-log-line standard"
    );
    // Channel 2 (the deliberate carrier arm, walled so it cannot silently rot):
    // plain Display renders the same shape with the common-atom fallback token.
    assert_eq!(
        exit_error.to_string(),
        EXPECTED_DISPLAY_FALLBACK,
        "the plain-Display fallback channel is byte-exact too"
    );
}

#[test]
fn diverged_registries_refusal_does_not_accuse_the_runtime_registry() {
    let atoms = AtomTable::with_common_atoms();
    // The divergence, built on purpose: the loader binds `erlang:'+'/2` against
    // an EMPTY registry, fixing the import `Deferred` at load time, while the
    // scheduler is declared with a gate1-populated one. The refusal therefore
    // mints in a context where a registry IS wired and IS populated — the case
    // in which any emptiness claim would accuse an innocent composition.
    let loader_bifs = BifRegistryImpl::new();
    let runtime_bifs = Arc::new(BifRegistryImpl::new());
    register_gate1_bifs(&runtime_bifs, &atoms).expect("gate1 bifs register");
    let (scheduler, _registry) = start_scheduler(
        &atoms,
        &loader_bifs,
        NativeBifs::registry(Arc::clone(&runtime_bifs)),
    );

    let pid = spawn_probe(&scheduler, &atoms);
    send_int(&scheduler, pid, 7);
    let (reason, _result) = scheduler.run_until_exit(pid);
    let exit_error = scheduler.take_exit_error(pid);
    scheduler.shutdown();

    // (1) The refusal happened: load-time resolution decides, so a populated
    // runtime registry does not rescue an import bound against an empty one.
    assert_eq!(
        reason,
        ExitReason::Error,
        "the guard-bif refusal is process-fatal even with natives wired at runtime"
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
                "the LOAD-TIME registry had no entry and the target module is not loaded"
            );
        }
        other => panic!("expected GuardBifUnavailable, got {other:?}"),
    }

    // (2) The divergence was actually constructed. Without this the test can go
    // green by never building the subject at all, and (3) would be vacuous for
    // a second, undetectable reason.
    assert!(
        runtime_bifs
            .lookup(atoms.intern("erlang"), atoms.intern("+"), 2)
            .is_some(),
        "the registry handed to the constructor resolves erlang:'+'/2 — the two \
         registries genuinely diverge"
    );

    // (3) The refusal does not accuse. Both rendering channels are walled: the
    // exact-string carrier and the plain-Display fallback.
    let rendered = exit_error.format_with_atoms(&atoms);
    let fallback = exit_error.to_string();
    for banned in BANNED_ACCUSATIONS {
        assert!(
            !rendered.contains(banned),
            "the refusal must not carry {banned:?}, which is false or \
             unsubstantiable here: {rendered}"
        );
        assert!(
            !fallback.contains(banned),
            "the plain-Display channel must not carry {banned:?} either: {fallback}"
        );
    }
}

#[test]
fn populated_registry_runs_the_arithmetic_clean() {
    let atoms = AtomTable::with_common_atoms();
    let bifs = Arc::new(BifRegistryImpl::new());
    register_gate1_bifs(&bifs, &atoms).expect("gate1 bifs register");
    let (scheduler, _registry) =
        start_scheduler(&atoms, &bifs, NativeBifs::registry(Arc::clone(&bifs)));

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
