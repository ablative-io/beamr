//! BEAMR-R8-DEOPT R4 — REPLAY PARITY ACROSS THE DEOPT BOUNDARY.
//!
//! Distinct from R3, and R3 may not be substituted for it. R3 compares ONE
//! function's error; this compares a WHOLE workflow-shaped run's output — every
//! leg result, the derived routing token, and the routed-or-trapped outcome —
//! byte for byte, interpreted vs JIT'd-with-deopt, on a run that TAKES the error
//! edge. The happy-path-only parity run does not witness this, so both happy
//! modes AND the trapping mode are driven, and the trapping mode is the point.
//!
//! THE HARNESS, AND WHY IT IS NEW. beamr has no existing whole-run
//! JIT-vs-interpreter parity harness to extend. `tests/differential.rs` is a
//! PER-FUNCTION differential runner (compile one function, run it two ways); the
//! `jit_*_replay_probe` files compare an exit reason and a side-effect count,
//! not a run's output term. beamr's deterministic-replay scheduler
//! (`crate::replay`, `Scheduler::new_replay_with_registry`) cannot host this
//! comparison either — FINDING F9: nothing in `crates/beamr/src` or
//! `crates/beamr-cli/src` ever constructs a `ReplayRecorder`, so no live run in
//! this tree can produce a `ReplayLog`, and a hand-built log is exhausted at the
//! first timer-expiry decision ("replay mismatch: replay log exhausted before
//! timer expiry decision at replay cursor 0", measured). Hand-authoring an event
//! log covering every scheduling decision would be scaffolding, not evidence.
//! So this harness is built, and its shape is stated for the judge:
//!
//!   * one scheduler per arm, one `awl_replay` module per arm;
//!   * arm A: JIT live, heated on the HAPPY modes until the routing step is
//!     cached native, so the trapping mode's edge is taken IN NATIVE CODE;
//!   * arm B: JIT disabled immediately after construction and before the first
//!     spawn, so nothing is ever compiled and nothing already compiled is ever
//!     entered — the operator switch's own contract, which a large
//!     `jit_threshold` cannot promise;
//!   * all three modes driven in each arm, the trapping one LAST;
//!   * every mode's `{run, Legs, Token, Outcome}` rendered through the repo's
//!     own `beamr::term::format::format_term` and compared byte for byte,
//!     alongside the exit reason and any host error.
//!
//! Fixture provenance: `fixtures/awl_replay.erl` is derived from
//! `workflows/gates/gates.awl` in our own public first-party repo
//! `ablative-io/aion` — the seven-leg battery and its `step manifest` routing
//! select. That file's header records what was cut.

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
const HEAT_DRIVES: usize = 8;

/// The workflow's three modes. `unmeasured` is the one that takes the cold
/// routing trap; the other two are the happy runs the trap must not perturb.
const MODES: [&str; 3] = ["clean", "failing", "unmeasured"];
const TRAPPING_MODE: &str = "unmeasured";

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

struct Arm {
    scheduler: Arc<Scheduler>,
    module: Atom,
    route: Atom,
    run: Atom,
    generation: u64,
}

impl Arm {
    /// A deterministic-replay scheduler over the `awl_replay` fixture.
    fn start(atoms: &AtomTable, jit: bool) -> Self {
        let bifs = BifRegistryImpl::new();
        let registry = Arc::new(ModuleRegistry::new());
        load_module(
            include_bytes!("fixtures/awl_replay.beam"),
            atoms,
            &registry,
            &bifs,
        )
        .expect("awl_replay fixture loads");
        let module = atoms.intern("awl_replay");
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
            route: atoms.intern("route"),
            run: atoms.intern("run"),
            generation,
        }
    }

    fn drive(&self, mode: Atom) -> (ExitReason, OwnedTerm, Option<String>) {
        let pid = self
            .scheduler
            .spawn(self.module, self.run, vec![Term::atom(mode)])
            .expect("spawn run/1");
        let (tx, rx) = mpsc::channel();
        let sched = Arc::clone(&self.scheduler);
        std::thread::spawn(move || {
            let _ = tx.send(sched.run_until_exit(pid));
        });
        let (exit, value) = rx
            .recv_timeout(DEADLINE)
            .unwrap_or_else(|_| panic!("the workflow did not terminate within {DEADLINE:?}"));
        let host_error = self
            .scheduler
            .take_exit_error(pid)
            .map(|error| error.to_string());
        (exit, value, host_error)
    }

    fn route_cached(&self) -> bool {
        self.scheduler
            .jit_cache()
            .lookup(self.module, self.route, 1, self.generation)
            .is_some()
    }
}

/// One arm's whole transcript: the routing step's admission state plus every
/// mode's full run output.
struct Transcript {
    route_cached: bool,
    runs: Vec<(String, String)>,
}

fn measure(atoms: &AtomTable, arm: &Arm, expect_cached: bool) -> Transcript {
    // Heat on the HAPPY modes only — the routing step must be admitted and
    // cached before the trapping mode is driven, or its edge is not taken in
    // native code and R4 measures nothing.
    let clean = atoms.intern("clean");
    for _ in 0..HEAT_DRIVES {
        let (exit, _, host_error) = arm.drive(clean);
        assert_eq!(
            exit,
            ExitReason::Normal,
            "heat run must exit normally (host error: {host_error:?})"
        );
    }
    if expect_cached {
        assert!(
            wait_until(|| arm.route_cached()),
            "route/1 — the workflow's routing step, whose cold fall-through is a \
             CaseEnd — must be ADMITTED and cached native before the trapping \
             mode is driven"
        );
    }
    let route_cached = arm.route_cached();

    let runs = MODES
        .iter()
        .map(|mode| {
            let (exit, value, host_error) = arm.drive(atoms.intern(mode));
            (
                (*mode).to_owned(),
                format!(
                    "exit={exit:?} out={} host_error={host_error:?}",
                    format_term(value.root(), atoms)
                ),
            )
        })
        .collect();

    Transcript { route_cached, runs }
}

#[test]
fn workflow_taking_the_error_edge_is_byte_identical_interpreted_and_jit_with_deopt() {
    let atoms = AtomTable::with_common_atoms();

    let jit = Arm::start(&atoms, true);
    let native = measure(&atoms, &jit, true);
    jit.scheduler.shutdown();

    let plain = Arm::start(&atoms, false);
    let interpreted = measure(&atoms, &plain, false);
    plain.scheduler.shutdown();

    println!("---- BEAMR-R8-DEOPT R4 replay-parity transcript ----");
    println!(
        "  harness: one Scheduler per arm over the same awl_replay module; arm B \
         has set_jit_enabled(false) applied before its first spawn"
    );
    println!(
        "  routing step cached native? JIT'd-with-deopt arm={} never-JIT'd arm={}",
        native.route_cached, interpreted.route_cached
    );
    for ((mode, native_out), (_, interpreted_out)) in native.runs.iter().zip(&interpreted.runs) {
        let taken = if mode == TRAPPING_MODE {
            " <- ERROR EDGE TAKEN"
        } else {
            ""
        };
        println!("  mode {mode}{taken}");
        println!("    JIT'd-with-deopt : {native_out}");
        println!("    never-JIT'd      : {interpreted_out}");
    }

    assert!(
        native.route_cached,
        "the JIT arm's routing step must be cached native, or the trapping run \
         never crosses the deopt boundary and R4 witnesses nothing"
    );
    assert!(
        !interpreted.route_cached,
        "the never-JIT'd arm must have no cached native routing step"
    );
    assert_eq!(
        native.runs, interpreted.runs,
        "R4 — the whole workflow-shaped run's output must be BYTE-IDENTICAL \
         interpreted vs JIT'd-with-deopt, on the run that TAKES the error edge \
         as well as on the runs that do not"
    );

    let trapping = native
        .runs
        .iter()
        .find(|(mode, _)| mode == TRAPPING_MODE)
        .expect("the trapping mode is driven");
    assert!(
        trapping.1.contains("trapped") && trapping.1.contains("case_clause"),
        "the trapping mode must actually TAKE the routing step's cold edge and \
         carry the {{case_clause, V}} reason through the whole run's output — \
         otherwise this run is a second happy-path parity run wearing R4's name. \
         Got: {}",
        trapping.1
    );
    assert!(
        native
            .runs
            .iter()
            .filter(|(mode, _)| mode != TRAPPING_MODE)
            .all(|(_, out)| out.contains("routed") && !out.contains("trapped")),
        "the happy modes must route, not trap — the cure must not perturb the \
         paths that never reach the cold edge"
    );
}
