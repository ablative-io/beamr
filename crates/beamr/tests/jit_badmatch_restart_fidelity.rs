//! BEAMR-R8-DEOPT R3 — the DISCRIMINATING Badmatch fixture: RESTART FIDELITY.
//!
//! R3 says the Badmatch specimen must trap on the SUBJECT operand. The naive
//! reading of that is a trap, and the AWL bytes are why. Both construction sites
//! in our own public first-party repo `ablative-io/aion` emit the SAME operand:
//!
//!   crates/aion-awl/src/mir/select/emit/burst.rs:293-295  Badmatch { value: X(0) }
//!   crates/aion-awl/src/mir/select/emit/burst.rs:393-395  Badmatch { value: X(0) }
//!
//! so NO lowering can report the walked tail by mis-reading the operand — the
//! operand is `X(0)` either way, and recording "the operand was X(0)" proves
//! nothing. The subject/tail distinction lives one instruction earlier, in the
//! `Move` at `burst.rs:389-392` under the BC-2b-5 comment at `:386-388`, which
//! re-points `X0` at the subject from its `Y` home — because `assert_list`'s own
//! unrolled walk CLOBBERS `X0` with the tail on every bind (`burst.rs:370-373`,
//! `Move { source: X(2), destination: X(0) }`).
//!
//! WHAT THIS FIXTURE GUARDS is therefore restart fidelity: that a deopt restarts
//! the function from its BYTECODE ENTRY, so the restarted interpreter
//! re-executes that `Move` itself, rather than resuming mid-function or
//! marshalling native register state across the deopt. Under a state-marshalling
//! lowering `X0` would still hold the walked tail and the reason would
//! misattribute the mismatch — the exact BC-2b-5 regression.
//!
//! THE SPECIMEN IS AWL'S OWN BURST SHAPE, transposed one step. `assert_list`
//! (`burst.rs:344-396`) walks with `TypeTestOp::IsNonemptyList` + `GetList` and
//! terminates on `TypeTestOp::IsNil`. This JIT tier lowers NEITHER of those two
//! type tests (`jit/ir_control_validation.rs:29-35` admits only IsInteger /
//! IsAtom / IsPid / IsBinary / IsList / IsTuple), so the literal list burst is
//! still rejected after R8 — FOR A REASON THAT IS NOT THE ERROR TERMINAL, and
//! the flight report records that as its own finding. The shape reproduced here
//! is structurally identical, instruction for instruction, with the list walk
//! transposed to a 2-tuple chain: home the subject in `Y0`, load it into `X0`,
//! unroll the binds each clobbering `X0` with the walked tail, terminate on a
//! shape test, and on the fail edge re-point `X0` at the subject from `Y0`
//! before `Badmatch { value: X(0) }`.
//!
//! ONE FURTHER TRANSPOSITION, DECLARED. AWL homes the subject in a `Y` slot and
//! this specimen homes it in `X3`. The reason is FINDING F7, measured in
//! `jit_deopt_frame_restart_control.rs`: a deopt that happens AFTER an
//! `Allocate` leaves the native frame on the stack, the restart pushes a second
//! one, and the raise-time stacktrace then carries a duplicated frame. That
//! divergence is NOT this brief's — the `func_info` template, untouched here,
//! diverges by EXACTLY the same one frame, which is what the control asserts —
//! but it would make this fixture's stacktrace comparison measure the seam
//! instead of the subject. Homing in `X3` keeps the clobber-then-restore shape
//! that IS the point (X0 is overwritten by the walked tail on every bind and
//! re-pointed at the subject only by the BC-2b-5 `Move`) while leaving the
//! stacktrace free to be compared byte for byte.
//!
//! Both arms run IN THIS ONE BINARY: native-with-deopt, and never-JIT'd.
//! Identity is asserted on class, reason term AND stacktrace.
//!
//! First-party fixture note: beamr is our own product, this box is our own fleet
//! venue, and `ablative-io/aion` is our own public repo. Every document and
//! estate touched here is a first-party local test fixture.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use beamr::atom::{Atom, AtomTable};
use beamr::loader::Instruction;
use beamr::loader::decode::TypeTestOp;
use beamr::loader::decode::compact::Operand;
use beamr::module::{Module, ModuleOrigin, ModuleRegistry};
use beamr::process::ExitReason;
use beamr::scheduler::{NativeBifs, Scheduler, SchedulerConfig};
use beamr::term::Term;

const DEADLINE: Duration = Duration::from_secs(30);
const WAIT_BUDGET: Duration = Duration::from_secs(20);
const HEAT_DRIVES: usize = 8;

/// Selector atoms the driver builds its subject from.
const TRAPPING: &str = "full";
const HAPPY: &str = "exact";

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

struct Names {
    module: Atom,
    driver: Atom,
    specimen: Atom,
    a: Atom,
    b: Atom,
    c: Atom,
    deep_tail: Atom,
    walk_end: Atom,
    full: Atom,
    exact: Atom,
}

impl Names {
    fn intern(atoms: &AtomTable) -> Self {
        Self {
            module: atoms.intern("awl_burst"),
            driver: atoms.intern("driver"),
            specimen: atoms.intern("assert_targets"),
            a: atoms.intern("fmt_target"),
            b: atoms.intern("clippy_workspace_target"),
            c: atoms.intern("test_target"),
            deep_tail: atoms.intern("embed_target"),
            walk_end: atoms.intern("targets_end"),
            full: atoms.intern(TRAPPING),
            exact: atoms.intern(HAPPY),
        }
    }
}

/// AWL's `assert_list` burst (`ablative-io/aion`
/// `crates/aion-awl/src/mir/select/emit/burst.rs:344-396`), instruction for
/// instruction, with the list walk transposed to a 2-tuple chain so every op is
/// one this tier lowers. `driver/1` builds the subject from a selector; the
/// specimen `assert_targets/1` is the function under JIT.
fn burst_module(names: &Names) -> Module {
    let a = Operand::Atom(Some(names.a));
    let b = Operand::Atom(Some(names.b));
    let c = Operand::Atom(Some(names.c));
    let deep_tail = Operand::Atom(Some(names.deep_tail));
    let walk_end = Operand::Atom(Some(names.walk_end));

    // A single unrolled bind: AWL's `burst.rs:357-373` — shape-test the walked
    // value, split head/tail, home the head, and CLOBBER X0 with the tail.
    let bind = |fail: u32| {
        vec![
            Instruction::TypeTest {
                op: TypeTestOp::IsTuple,
                fail: Operand::Label(fail),
                value: Operand::X(0),
            },
            Instruction::TestArity {
                fail: Operand::Label(fail),
                tuple: Operand::X(0),
                arity: Operand::Unsigned(2),
            },
            Instruction::GetTupleElement {
                source: Operand::X(0),
                index: Operand::Unsigned(0),
                destination: Operand::X(1),
            },
            Instruction::GetTupleElement {
                source: Operand::X(0),
                index: Operand::Unsigned(1),
                destination: Operand::X(2),
            },
            // burst.rs:370-373 — the walked tail REPLACES the subject in X0.
            Instruction::Move {
                source: Operand::X(2),
                destination: Operand::X(0),
            },
        ]
    };

    let mut code = vec![
        // ---- driver/1 ----
        Instruction::Label { label: 1 },
        Instruction::FuncInfo {
            module: Operand::Atom(Some(names.module)),
            function: Operand::Atom(Some(names.driver)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 2 },
        Instruction::TestHeap {
            heap_need: Operand::Unsigned(12),
            live: Operand::Unsigned(1),
        },
        Instruction::SelectVal {
            value: Operand::X(0),
            fail: Operand::Label(5),
            list: Operand::List(vec![
                Operand::Atom(Some(names.full)),
                Operand::Label(3),
                Operand::Atom(Some(names.exact)),
                Operand::Label(4),
            ]),
        },
        // `full`: one bind too many for the walk — the subject that TRAPS.
        Instruction::Label { label: 3 },
        Instruction::PutTuple2 {
            destination: Operand::X(1),
            elements: Operand::List(vec![c.clone(), deep_tail.clone()]),
        },
        Instruction::PutTuple2 {
            destination: Operand::X(1),
            elements: Operand::List(vec![b.clone(), Operand::X(1)]),
        },
        Instruction::PutTuple2 {
            destination: Operand::X(0),
            elements: Operand::List(vec![a.clone(), Operand::X(1)]),
        },
        Instruction::Jump {
            target: Operand::Label(6),
        },
        // `exact`: the subject the walk accepts — the R3b null arm's input.
        Instruction::Label { label: 4 },
        Instruction::PutTuple2 {
            destination: Operand::X(1),
            elements: Operand::List(vec![b.clone(), walk_end.clone()]),
        },
        Instruction::PutTuple2 {
            destination: Operand::X(0),
            elements: Operand::List(vec![a.clone(), Operand::X(1)]),
        },
        Instruction::Jump {
            target: Operand::Label(6),
        },
        // No other selector is driven; the arm exists so the select is total.
        Instruction::Label { label: 5 },
        Instruction::Return,
        Instruction::Label { label: 6 },
        Instruction::Call {
            arity: Operand::Unsigned(1),
            label: Operand::Label(8),
        },
        Instruction::Return,
        // ---- assert_targets/1 — THE SPECIMEN ----
        Instruction::Label { label: 7 },
        Instruction::FuncInfo {
            module: Operand::Atom(Some(names.module)),
            function: Operand::Atom(Some(names.specimen)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 8 },
        // AWL homes the subject (`self.home(list)`) in a Y slot; X3 here, per
        // the transposition declared in the header — F7, not a shortcut.
        Instruction::Move {
            source: Operand::X(0),
            destination: Operand::X(3),
        },
        // burst.rs:350-353 — load the subject into X0 to start the walk.
        Instruction::Move {
            source: Operand::X(3),
            destination: Operand::X(0),
        },
    ];
    // Two unrolled binds. The first homes its head (AWL's `Some(var)` arm,
    // burst.rs:365-369); the second discards it (`None`).
    code.extend(bind(9));
    code.push(Instruction::Move {
        source: Operand::X(1),
        destination: Operand::X(4),
    });
    code.extend(bind(9));
    code.extend([
        // burst.rs:375-379 — the exact-length terminator. AWL uses IsNil on the
        // walked list; the tuple-chain transposition ends on an atom.
        Instruction::TypeTest {
            op: TypeTestOp::IsAtom,
            fail: Operand::Label(9),
            value: Operand::X(0),
        },
        Instruction::Jump {
            target: Operand::Label(10),
        },
        Instruction::Label { label: 9 },
        // burst.rs:385-392, THE BC-2b-5 CARRIED FIX: trap on the SUBJECT, not
        // the walked tail X0 happens to hold. This `Move` is the whole fixture.
        Instruction::Move {
            source: Operand::X(3),
            destination: Operand::X(0),
        },
        Instruction::Badmatch {
            value: Operand::X(0),
        },
        // burst.rs:396 — the `done` label. The bound head is returned so the
        // happy walk has an observable result for the R3b null arm.
        Instruction::Label { label: 10 },
        Instruction::Move {
            source: Operand::X(4),
            destination: Operand::X(0),
        },
        Instruction::Return,
    ]);

    let mut exports = HashMap::new();
    exports.insert((names.driver, 1), 2);
    exports.insert((names.specimen, 1), 8);
    finish_module(names.module, code, exports)
}

fn finish_module(name: Atom, code: Vec<Instruction>, exports: HashMap<(Atom, u8), u32>) -> Module {
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            _ => None,
        })
        .collect();
    let function_table = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::FuncInfo {
                function: Operand::Atom(Some(function)),
                arity: Operand::Unsigned(arity),
                ..
            } => Some((ip, *function, u8::try_from(*arity).ok()?)),
            _ => None,
        })
        .collect();
    Module {
        name,
        generation: 0,
        origin: ModuleOrigin::Preloaded,
        exports,
        label_index,
        code,
        function_table,
        line_table: Vec::new(),
        literals: Vec::new(),
        constant_pool: Default::default(),
        resolved_imports: Vec::new(),
        lambdas: Vec::new(),
        string_table: Vec::new(),
        line_info: Vec::new(),
    }
}

struct Composition {
    scheduler: Arc<Scheduler>,
    names: Names,
    generation: u64,
}

impl Composition {
    fn start(atoms: &AtomTable, jit: bool) -> Self {
        let names = Names::intern(atoms);
        let registry = Arc::new(ModuleRegistry::new());
        let module = registry.insert(burst_module(&names));
        let generation = module.generation;
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
                NativeBifs::none(),
            )
            .expect("scheduler starts"),
        );
        if !jit {
            scheduler.set_jit_enabled(false);
        }
        Self {
            scheduler,
            names,
            generation,
        }
    }

    /// `atoms` is the table the module's own atoms were interned into. The
    /// scheduler builds its own common-atom table, which does not know these
    /// names, so rendering through it would print unrelated common atoms.
    fn drive(&self, atoms: &AtomTable, selector: Atom) -> Outcome {
        let pid = self
            .scheduler
            .spawn(
                self.names.module,
                self.names.driver,
                vec![Term::atom(selector)],
            )
            .expect("spawn driver");
        let (tx, rx) = mpsc::channel();
        let sched = Arc::clone(&self.scheduler);
        std::thread::spawn(move || {
            let _ = tx.send(sched.run_until_exit(pid));
        });
        let (exit, value) = rx
            .recv_timeout(DEADLINE)
            .unwrap_or_else(|_| panic!("drive did not terminate within {DEADLINE:?}"));
        let exception = self.scheduler.take_exit_exception(pid);
        let host_error = self
            .scheduler
            .take_exit_error(pid)
            .map(|error| error.to_string());
        Outcome {
            exit,
            rendered_value: beamr::term::format::format_term(value.root(), atoms),
            rendered_exception: exception.map(|e| e.format_with_atoms(atoms)),
            host_error,
        }
    }

    fn specimen_cached(&self) -> bool {
        self.scheduler
            .jit_cache()
            .lookup(self.names.module, self.names.specimen, 1, self.generation)
            .is_some()
    }
}

struct Outcome {
    exit: ExitReason,
    /// RENDERED AT CAPTURE, not stored as a raw `Term`. The exit result's terms
    /// live in the `OwnedTerm`'s heap, so a `Term` copied out of it dangles the
    /// moment that `OwnedTerm` drops — rendering here keeps the borrow honest.
    rendered_value: String,
    rendered_exception: Option<String>,
    host_error: Option<String>,
}

impl Outcome {
    fn rendered(&self) -> String {
        format!(
            "exit={:?} value={} exception={:?} host_error={:?}",
            self.exit, self.rendered_value, self.rendered_exception, self.host_error
        )
    }
}

struct Arm {
    specimen_cached: bool,
    happy: String,
    trapped: String,
}

fn measure(atoms: &AtomTable, composition: &Composition, expect_cached: bool) -> Arm {
    let happy_selector = composition.names.exact;
    let trapping_selector = composition.names.full;

    for _ in 0..HEAT_DRIVES {
        let heat = composition.drive(atoms, happy_selector);
        assert_eq!(
            heat.exit,
            ExitReason::Normal,
            "heat drive must exit normally (host error: {:?})",
            heat.host_error
        );
    }
    if expect_cached {
        assert!(
            wait_until(|| composition.specimen_cached()),
            "assert_targets/1 — the AWL burst shape carrying the Badmatch trap — \
             must be ADMITTED and cached native. beamr's pre-pass admission is \
             whole-function, so a cache entry here IS the proof the terminal was \
             lowered instead of rejecting its container"
        );
    }
    let specimen_cached = composition.specimen_cached();

    // R3b null arm, in the same run: the walk that never traps.
    let happy = composition.drive(atoms, happy_selector);
    assert_eq!(
        happy.exit,
        ExitReason::Normal,
        "the cured function's happy walk must still exit normally (host error: {:?})",
        happy.host_error
    );

    // R3 deopt arm: the fail edge, TAKEN.
    let trapped = composition.drive(atoms, trapping_selector);

    Arm {
        specimen_cached,
        happy: happy.rendered(),
        trapped: trapped.rendered(),
    }
}

#[test]
fn badmatch_reports_the_subject_not_the_walked_tail_across_the_deopt_boundary() {
    let atoms = AtomTable::with_common_atoms();
    let names = Names::intern(&atoms);

    // The two terms the assertion discriminates between, rendered from the same
    // atoms the fixture builds them from. The subject is the whole 2-tuple
    // chain; the walked tail at the point of failure is a PROPER SUB-TERM of it.
    let subject = format!(
        "{{{}, {{{}, {{{}, {}}}}}}}",
        atoms.resolve(names.a).unwrap(),
        atoms.resolve(names.b).unwrap(),
        atoms.resolve(names.c).unwrap(),
        atoms.resolve(names.deep_tail).unwrap()
    );
    let walked_tail = format!(
        "{{{}, {}}}",
        atoms.resolve(names.c).unwrap(),
        atoms.resolve(names.deep_tail).unwrap()
    );
    assert_ne!(
        subject, walked_tail,
        "the fixture is vacuous unless the subject and the walked tail at the \
         point of failure are DISTINGUISHABLE terms"
    );

    let jit = Composition::start(&atoms, true);
    let native = measure(&atoms, &jit, true);
    jit.scheduler.shutdown();

    let plain = Composition::start(&atoms, false);
    let interpreted = measure(&atoms, &plain, false);
    assert!(
        !interpreted.specimen_cached,
        "the never-JIT'd arm must have NO cached native body for the specimen"
    );
    plain.scheduler.shutdown();

    println!("---- BEAMR-R8-DEOPT R3 Badmatch RESTART-FIDELITY transcript ----");
    println!("  subject (must be reported)        : {subject}");
    println!("  walked tail at failure (must not) : {walked_tail}");
    println!(
        "  specimen cached native? native-arm={} never-JIT'd-arm={}",
        native.specimen_cached, interpreted.specimen_cached
    );
    println!("  R3b null arm (happy walk, SAME RUN)");
    println!("    native-with-deopt : {}", native.happy);
    println!("    never-JIT'd       : {}", interpreted.happy);
    println!("  R3 deopt arm (fail edge TAKEN)");
    println!("    native-with-deopt : {}", native.trapped);
    println!("    never-JIT'd       : {}", interpreted.trapped);

    assert!(
        native.specimen_cached,
        "the specimen must be cached native for its fail edge to be taken in \
         native code"
    );
    assert_eq!(
        native.happy, interpreted.happy,
        "R3b — the cure must not perturb the walk that never traps"
    );
    assert_eq!(
        native.trapped, interpreted.trapped,
        "R3 — ERROR IDENTITY. Class, reason term and stacktrace must be \
         byte-identical between the native-with-deopt run and the never-JIT'd run"
    );

    let subject_reason = format!("{{badmatch, {subject}}}");
    let tail_reason = format!("{{badmatch, {walked_tail}}}");
    assert!(
        native.trapped.contains(&subject_reason),
        "RESTART FIDELITY: the reason must be {subject_reason} — the WHOLE \
         SUBJECT. It reaches X0 only because the deopt restarts the function from \
         its bytecode ENTRY, so the restarted interpreter re-executes the \
         BC-2b-5 `Move` (burst.rs:389-392) itself. A lowering that resumed \
         mid-function, or marshalled native register state across the deopt, \
         would leave the walked tail in X0. Got: {}",
        native.trapped
    );
    assert!(
        !native.trapped.contains(&tail_reason),
        "RESTART FIDELITY: the reason must NOT be {tail_reason} — the walked \
         tail. That is the exact BC-2b-5 misattribution this fixture exists to \
         catch. Got: {}",
        native.trapped
    );
}
