//! BEAMR-R8-DEOPT R3 / R3b — error IDENTITY across the deopt boundary, for the
//! four error-raising terminals `Badmatch`, `CaseEnd`, `IfEnd` and `Badrecord`.
//!
//! ACCEPTANCE IS IDENTITY, NOT "AN ERROR WAS RAISED". Each specimen's error edge
//! is forced and TAKEN, and the run is measured twice IN THIS ONE BINARY:
//! JIT'd-with-deopt, and never-JIT'd. What is compared is the whole caught
//! `{caught, Class, Reason, Stacktrace}` term rendered through the repo's own
//! `beamr::term::format::format_term` — class, reason term AND stacktrace, byte
//! for byte. An assertion that would still pass against a different reason term
//! is not an identity assertion, and none below would.
//!
//! TAKEN-NESS IS ASSERTED, NOT INFERRED — three mechanical ways:
//!   1. the JIT arm asserts the SPECIMEN is in the JIT cache at the moment of
//!      the error drive. beamr's pre-pass admission is WHOLE-FUNCTION (the walk
//!      in `ir_control.rs` errs on the FIRST non-Supported variant, with no
//!      compile-around), so a cache entry for a function whose only cold edge is
//!      the terminal IS the proof that the terminal was lowered rather than
//!      rejecting its whole container — which is the entire point of R8;
//!   2. the never-JIT'd arm asserts the cache is EMPTY for that same key;
//!   3. the happy input returns its value through that same cached native body
//!      in the same run (the R3b null arm), so the body is demonstrably entered.
//!
//! THE BADMATCH ARM IS NOT HERE. It is a RESTART-FIDELITY fixture built on AWL's
//! own burst shape and lives in `jit_badmatch_restart_fidelity.rs`; that file's
//! header explains why an operand check would be vacuous. The erlc-emitted list
//! destructure that would be the natural Badmatch specimen here reaches its trap
//! through `TypeTestOp::IsNonemptyList` / `IsNil`, NEITHER of which this JIT tier
//! lowers (`jit/ir_control_validation.rs`), so it is still rejected after R8 for
//! a reason that is not the terminal — measured as its own R5 rejection specimen
//! in `jit_error_terminal_prepass_acceptance.rs`.
//!
//! `Badrecord` is IDENTITY OF A NON-RAISE, and is labelled as such wherever it
//! appears: beamr's interpreter has no `Badrecord` execution arm
//! (`interpreter/opcodes/mod.rs` dispatches Badmatch/CaseEnd/IfEnd only, and
//! `interpreter/opcodes/exceptions.rs` defines no `badrecord`), so the variant
//! falls to the `other =>` catch-all and BOTH arms yield the same
//! `ExecError::UnsupportedOpcode`. Identity holds. It is NOT evidence that the
//! terminal raises correctly.
//!
//! Fixture provenance: `fixtures/awl_terminals.erl` is DERIVED from production
//! documents in our own public first-party repo `ablative-io/aion` —
//! `workflows/gates/gates.awl` and `workflows/investigate/investigate.awl`, the
//! census's own poisoned population. That file's header records each specimen's
//! source line and exactly what was cut.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use beamr::atom::{Atom, AtomTable};
use beamr::ets::OwnedTerm;
use beamr::loader::load_module;
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;
use beamr::process::ExitReason;
use beamr::scheduler::{NativeBifs, Scheduler, SchedulerConfig};
use beamr::term::Term;
use beamr::term::format::format_term;

const DEADLINE: Duration = Duration::from_secs(30);
const WAIT_BUDGET: Duration = Duration::from_secs(20);
/// Happy-path drives before the tier is asked for a verdict on the specimen.
const HEAT_DRIVES: usize = 8;

fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + WAIT_BUDGET;
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// A composition over the `awl_terminals` fixture. `jit == false` disables the
/// JIT IMMEDIATELY after construction and BEFORE any spawn, which the operator
/// switch's own contract names as what guarantees nothing is ever compiled AND
/// that already-compiled code is never entered — a large `jit_threshold` cannot
/// promise the second half.
struct Composition {
    scheduler: Arc<Scheduler>,
    module: Atom,
    generation: u64,
}

impl Composition {
    fn start(atoms: &AtomTable, jit: bool) -> Self {
        let bifs = BifRegistryImpl::new();
        let registry = Arc::new(ModuleRegistry::new());
        load_module(
            include_bytes!("fixtures/awl_terminals.beam"),
            atoms,
            &registry,
            &bifs,
        )
        .expect("awl_terminals fixture loads");
        let module = atoms.intern("awl_terminals");
        let generation = registry
            .lookup(module)
            .expect("fixture module is registered")
            .generation;
        let scheduler = Arc::new(
            Scheduler::new(
                SchedulerConfig {
                    thread_count: Some(1),
                    dirty_cpu_threads: Some(1),
                    dirty_io_threads: Some(1),
                    jit_threshold: Some(2),
                    ..SchedulerConfig::default()
                },
                registry,
                NativeBifs::registry(Arc::new(bifs)),
            )
            .expect("scheduler starts"),
        );
        if !jit {
            scheduler.set_jit_enabled(false);
        }
        Self {
            scheduler,
            module,
            generation,
        }
    }

    /// Deadline-bound drive. A specimen that never terminates is a RED with its
    /// own message, never a hang.
    fn drive(&self, function: Atom, arg: Term) -> Drive {
        let pid = self
            .scheduler
            .spawn(self.module, function, vec![arg])
            .expect("spawn");
        let (tx, rx) = mpsc::channel::<(ExitReason, OwnedTerm)>();
        let sched = Arc::clone(&self.scheduler);
        std::thread::spawn(move || {
            let _ = tx.send(sched.run_until_exit(pid));
        });
        let (exit, value) = rx
            .recv_timeout(DEADLINE)
            .unwrap_or_else(|_| panic!("drive did not terminate within {DEADLINE:?}"));
        let host_error = self
            .scheduler
            .take_exit_error(pid)
            .map(|error| error.to_string());
        Drive {
            exit,
            value,
            host_error,
        }
    }

    fn cached(&self, specimen: Atom) -> bool {
        self.scheduler
            .jit_cache()
            .lookup(self.module, specimen, 1, self.generation)
            .is_some()
    }
}

struct Drive {
    exit: ExitReason,
    value: OwnedTerm,
    host_error: Option<String>,
}

impl Drive {
    fn rendered(&self, atoms: &AtomTable) -> String {
        format!(
            "exit={:?} value={} host_error={:?}",
            self.exit,
            format_term(self.value.root(), atoms),
            self.host_error
        )
    }
}

/// One terminal's measurement in one composition, both arms, IN ONE RUN.
struct Arm {
    /// Was the SPECIMEN — the function carrying the error terminal — admitted
    /// and cached native at the moment of the error drive?
    specimen_cached: bool,
    /// R3b null arm: the happy path's rendered outcome.
    happy: String,
    /// R3 deopt arm: the rendered outcome of the TAKEN error edge.
    caught: String,
}

/// One terminal's entry points and the two inputs that select its arms.
#[derive(Clone, Copy)]
struct Entries {
    /// The function carrying the error terminal — the one put under JIT.
    specimen: Atom,
    /// The try/catch wrapper that observes the raise as data.
    probe: Atom,
    /// The happy-path entry, which is also the heat.
    happy_entry: Atom,
    happy_arg: Term,
    error_arg: Term,
}

/// Heat the specimen on its happy path, wait for the tier's verdict, then
/// measure the null (happy) arm and the deopt (error) arm IN THAT ORDER, IN THE
/// SAME RUN — R3b's "the cure must not perturb the path that never traps",
/// measured beside the trap it cured.
fn measure(
    atoms: &AtomTable,
    composition: &Composition,
    entries: &Entries,
    expect_cached: bool,
) -> Arm {
    let Entries {
        specimen,
        probe,
        happy_entry,
        happy_arg,
        error_arg,
    } = *entries;
    for _ in 0..HEAT_DRIVES {
        let heat = composition.drive(happy_entry, happy_arg);
        assert_eq!(
            heat.exit,
            ExitReason::Normal,
            "heat drive of the happy path must exit normally (host error: {:?})",
            heat.host_error
        );
    }
    if expect_cached {
        assert!(
            wait_until(|| composition.cached(specimen)),
            "the specimen carrying the error terminal must be ADMITTED and cached \
             native — this is the whole contract of BEAMR-R8-DEOPT. beamr's \
             pre-pass admission is whole-function, so a cache entry here IS the \
             proof the terminal was lowered instead of rejecting its container"
        );
    }
    let specimen_cached = composition.cached(specimen);

    let happy = composition.drive(happy_entry, happy_arg);
    assert_eq!(
        happy.exit,
        ExitReason::Normal,
        "the cured function's happy path must still exit normally (host error: {:?})",
        happy.host_error
    );

    let caught = composition.drive(probe, error_arg);

    Arm {
        specimen_cached,
        happy: happy.rendered(atoms),
        caught: caught.rendered(atoms),
    }
}

/// The full R3 + R3b measurement for one terminal: both compositions, one run.
struct Identity {
    native: Arm,
    interpreted: Arm,
}

fn identity_of(
    atoms: &AtomTable,
    specimen: &str,
    probe: &str,
    happy_entry: &str,
    happy_arg: &str,
    error_arg: &str,
) -> Identity {
    let entries = Entries {
        specimen: atoms.intern(specimen),
        probe: atoms.intern(probe),
        happy_entry: atoms.intern(happy_entry),
        happy_arg: Term::atom(atoms.intern(happy_arg)),
        error_arg: Term::atom(atoms.intern(error_arg)),
    };

    let jit = Composition::start(atoms, true);
    let native = measure(atoms, &jit, &entries, true);
    jit.scheduler.shutdown();

    let plain = Composition::start(atoms, false);
    let interpreted = measure(atoms, &plain, &entries, false);
    assert!(
        !interpreted.specimen_cached,
        "the never-JIT'd arm must have NO cached native body for the specimen"
    );
    plain.scheduler.shutdown();

    Identity {
        native,
        interpreted,
    }
}

/// The identity assertion, plus the transcript the flight report carries.
fn assert_identity(label: &str, identity: &Identity) {
    let native = &identity.native;
    let interpreted = &identity.interpreted;
    println!("---- BEAMR-R8-DEOPT R3 transcript: {label} ----");
    println!(
        "  specimen cached native? native-arm={} never-JIT'd-arm={}",
        native.specimen_cached, interpreted.specimen_cached
    );
    println!("  R3b null arm  (happy path, SAME RUN as the deopt arm)");
    println!("    native-with-deopt : {}", native.happy);
    println!("    never-JIT'd       : {}", interpreted.happy);
    println!("  R3 deopt arm  (error edge TAKEN)");
    println!("    native-with-deopt : {}", native.caught);
    println!("    never-JIT'd       : {}", interpreted.caught);

    assert!(
        native.specimen_cached,
        "{label}: the specimen must be cached native for its error edge to be \
         taken in native code"
    );
    assert_eq!(
        native.happy, interpreted.happy,
        "{label}: R3b — the cure must not perturb the path that never traps; the \
         happy path must be byte-identical JIT'd vs interpreted"
    );
    assert_eq!(
        native.caught, interpreted.caught,
        "{label}: R3 — ERROR IDENTITY. Class, reason term and stacktrace must be \
         byte-identical between the native-with-deopt run and the never-JIT'd run"
    );
}

#[test]
fn case_end_error_identity_across_the_deopt_boundary() {
    let atoms = AtomTable::with_common_atoms();
    let identity = identity_of(
        &atoms,
        "verdict_code",
        "probe_verdict",
        "happy_verdict",
        "already_fixed",
        "no_such_verdict",
    );
    assert_identity("CaseEnd — reason {case_clause, V}", &identity);
    assert!(
        identity
            .native
            .caught
            .contains("{caught, error, {case_clause, no_such_verdict}"),
        "CaseEnd's reason must be the TUPLE {{case_clause, <the unmatched value>}} \
         with class `error` (interpreter/opcodes/exceptions.rs `case_end`), not \
         merely 'an error'. Got: {}",
        identity.native.caught
    );
}

#[test]
fn if_end_error_identity_across_the_deopt_boundary() {
    let atoms = AtomTable::with_common_atoms();
    let identity = identity_of(
        &atoms,
        "report_presence",
        "probe_presence",
        "happy_presence",
        "report",
        "absent",
    );
    assert_identity("IfEnd — reason is the BARE ATOM if_clause", &identity);
    assert!(
        identity
            .native
            .caught
            .contains("{caught, error, if_clause,"),
        "IfEnd's reason must be the BARE ATOM `if_clause` with class `error`. \
         A tuple-wrapped {{if_clause, []}} does not match `catch error:if_clause` \
         in loaded bytecode — the prior regression carried in \
         interpreter/opcodes/exceptions.rs's own comment. Got: {}",
        identity.native.caught
    );
    assert!(
        !identity.native.caught.contains("{if_clause"),
        "IfEnd's reason must NOT be tuple-wrapped. Got: {}",
        identity.native.caught
    );
}

/// F2: `Badrecord` has no interpreter execution arm, so this witnesses IDENTITY
/// OF A NON-RAISE. It is buildable and it is honest; it is NOT evidence that the
/// terminal raises correctly, and the flight report labels it in those words.
#[test]
fn badrecord_identity_of_a_non_raise_across_the_deopt_boundary() {
    let atoms = AtomTable::with_common_atoms();
    let identity = identity_of(
        &atoms,
        "gate_passed",
        "probe_gate_passed",
        "happy_gate_passed",
        "ok",
        "bad",
    );
    assert_identity(
        "Badrecord — IDENTITY OF A NON-RAISE (no interpreter arm exists)",
        &identity,
    );
    assert!(
        identity.native.caught.contains("badrecord"),
        "the non-raise must still name the opcode it refused, so the identity is \
         about `Badrecord` and not about some other failure. Got: {}",
        identity.native.caught
    );
}
