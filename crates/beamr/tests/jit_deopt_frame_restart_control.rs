//! BEAMR-R8-DEOPT — FINDING F7, with its control: a deopt that happens AFTER a
//! frame push leaves the native frame on the stack, and the restart pushes a
//! second one, so the raise-time stacktrace carries a duplicated frame.
//!
//! This is NOT introduced by this brief, and this file is the measurement that
//! proves it. Two specimens are built with IDENTICAL structure — `Allocate` a
//! frame, take a fail edge — differing ONLY in the terminal the fail edge
//! reaches:
//!
//!   * `funcinfo_probe/1` reaches `Instruction::FuncInfo`, the LEG 1c A2
//!     function-clause landing pad. That lowering predates this brief
//!     (`jit/compiler/dispatch_core.rs`, the arm above the four) and is the
//!     exact template R2 tells this brief to copy.
//!   * `badmatch_probe/1` reaches `Instruction::Badmatch`, lowered by THIS
//!     brief with that template's body verbatim.
//!
//! Both deopt, both restart interpreted, both raise. If the stacktrace
//! divergence were something R8's lowering introduced, only the second would
//! show it. The assertion below is that BOTH show it, IDENTICALLY — the defect
//! belongs to the deopt-restart seam (a native `Allocate` is not unwound when
//! `JIT_STATUS_DEOPT` returns `Ok(None)` and the caller re-enters the callee at
//! its bytecode entry, `interpreter/opcodes/core.rs`), and it is reachable today
//! by every `is_runtime_deopt_capable` instruction that can follow a frame push:
//! an `{f,0}` arithmetic `Bif`, `CallExt*`, `PutList`, `PutTuple2`, `MakeFun`,
//! `TestHeap`, the recv-markers. Those are all already `Supported`.
//!
//! CLASS AND REASON ARE STILL IDENTICAL in both arms — this touches the
//! stacktrace SHAPE only, and only for a specimen that allocates a frame before
//! its trap. The flight report carries it as a finding with a recommendation; it
//! is not fixed here, because fixing it means changing the deopt seam for every
//! deopt-capable instruction, which is a different brief and outside this one's
//! Walls ("No new mechanism — the func_info path is the template").
//!
//! First-party fixture note: beamr is our own product and this box is our own
//! fleet venue; every estate and fixture here is a first-party local fixture.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use beamr::atom::{Atom, AtomTable};
use beamr::loader::Instruction;
use beamr::loader::decode::compact::Operand;
use beamr::module::{Module, ModuleOrigin, ModuleRegistry};
use beamr::process::ExitReason;
use beamr::scheduler::{NativeBifs, Scheduler, SchedulerConfig};
use beamr::term::Term;

const DEADLINE: Duration = Duration::from_secs(30);
const WAIT_BUDGET: Duration = Duration::from_secs(20);
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

struct Names {
    module: Atom,
    drive_funcinfo: Atom,
    drive_badmatch: Atom,
    funcinfo_probe: Atom,
    badmatch_probe: Atom,
    trap: Atom,
    pass: Atom,
}

impl Names {
    fn intern(atoms: &AtomTable) -> Self {
        Self {
            module: atoms.intern("deopt_frame_control"),
            drive_funcinfo: atoms.intern("drive_funcinfo"),
            drive_badmatch: atoms.intern("drive_badmatch"),
            funcinfo_probe: atoms.intern("funcinfo_probe"),
            badmatch_probe: atoms.intern("badmatch_probe"),
            trap: atoms.intern("trap"),
            pass: atoms.intern("pass"),
        }
    }
}

/// Which terminal a specimen's fail edge reaches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Terminal {
    /// The LEG 1c A2 precedent, unchanged by this brief.
    FuncInfo,
    /// This brief's lowering, with the precedent's body verbatim.
    Badmatch,
}

/// `driver/1` calls the specimen; the specimen allocates a frame, then routes on
/// its argument: `pass` returns, `trap` takes the fail edge to the terminal.
/// The two specimens differ ONLY in that terminal.
fn control_module(names: &Names) -> Module {
    // The specimens are entered through a local `Call`, which is where the tier
    // dispatches to compiled code — spawning a function directly never consults
    // the JIT cache, so a driver is not decoration, it is the seam.
    let mut code = driver(names, names.drive_funcinfo, 1, 2, 4);
    code.extend(driver(names, names.drive_badmatch, 9, 10, 6));
    // `funcinfo_probe/1`: labels 3 (landing pad) / 4 (entry). Its fail edge
    // targets the pad, which is exactly how a dispatch fail edge reaches
    // func_info today.
    code.extend(probe(names, Terminal::FuncInfo, 3, 4, names.funcinfo_probe));
    // `badmatch_probe/1`: labels 5 (landing pad) / 6 (entry), 7 the fail edge.
    code.extend(probe(names, Terminal::Badmatch, 5, 6, names.badmatch_probe));

    let mut exports = HashMap::new();
    exports.insert((names.drive_funcinfo, 1), 2);
    exports.insert((names.drive_badmatch, 1), 10);
    exports.insert((names.funcinfo_probe, 1), 4);
    exports.insert((names.badmatch_probe, 1), 6);
    finish_module(names.module, code, exports)
}

fn driver(names: &Names, function: Atom, pad: u32, entry: u32, target: u32) -> Vec<Instruction> {
    vec![
        Instruction::Label { label: pad },
        Instruction::FuncInfo {
            module: Operand::Atom(Some(names.module)),
            function: Operand::Atom(Some(function)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: entry },
        Instruction::Call {
            arity: Operand::Unsigned(1),
            label: Operand::Label(target),
        },
        Instruction::Return,
    ]
}

fn probe(
    names: &Names,
    terminal: Terminal,
    pad_label: u32,
    entry_label: u32,
    function: Atom,
) -> Vec<Instruction> {
    // The fail edge's target: for the precedent it is the func_info landing pad
    // itself (exactly how a dispatch fail edge reaches func_info today); for
    // this brief's terminal it is a dedicated fail block.
    let fail_label = match terminal {
        Terminal::FuncInfo => pad_label,
        Terminal::Badmatch => entry_label + 1,
    };
    let mut code = vec![
        Instruction::Label { label: pad_label },
        Instruction::FuncInfo {
            module: Operand::Atom(Some(names.module)),
            function: Operand::Atom(Some(function)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: entry_label },
        // THE FRAME PUSH. Everything this file measures hangs off this line.
        Instruction::Allocate {
            stack_need: Operand::Unsigned(2),
            live: Operand::Unsigned(1),
        },
        Instruction::InitYregs {
            registers: Operand::List(vec![Operand::Y(0), Operand::Y(1)]),
        },
        Instruction::Move {
            source: Operand::X(0),
            destination: Operand::Y(0),
        },
        // `pass` continues; anything else takes the fail edge to the terminal.
        Instruction::Comparison {
            op: beamr::loader::decode::ComparisonOp::EqExact,
            fail: Operand::Label(fail_label),
            left: Operand::X(0),
            right: Operand::Atom(Some(names.pass)),
        },
        Instruction::Move {
            source: Operand::Y(0),
            destination: Operand::X(0),
        },
        Instruction::Deallocate {
            words: Operand::Unsigned(2),
        },
        Instruction::Return,
    ];
    if terminal == Terminal::Badmatch {
        code.extend([
            Instruction::Label { label: fail_label },
            Instruction::Move {
                source: Operand::Y(0),
                destination: Operand::X(0),
            },
            Instruction::Badmatch {
                value: Operand::X(0),
            },
        ]);
    }
    code
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
        let module = registry.insert(control_module(&names));
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

    fn drive(&self, entry: Atom, selector: Atom) -> Raise {
        let pid = self
            .scheduler
            .spawn(self.names.module, entry, vec![Term::atom(selector)])
            .expect("spawn driver");
        let (tx, rx) = mpsc::channel();
        let sched = Arc::clone(&self.scheduler);
        std::thread::spawn(move || {
            let _ = tx.send(sched.run_until_exit(pid));
        });
        let (exit, _value) = rx
            .recv_timeout(DEADLINE)
            .unwrap_or_else(|_| panic!("drive did not terminate within {DEADLINE:?}"));
        let exception = self.scheduler.take_exit_exception(pid);
        let frames = exception
            .as_ref()
            .map_or(0, |exception| exception.frames().len());
        let arities: Vec<u8> = exception.as_ref().map_or_else(Vec::new, |exception| {
            exception.frames().iter().map(|frame| frame.arity).collect()
        });
        Raise {
            exit,
            frames,
            arities,
        }
    }

    fn cached(&self, probe: Atom) -> bool {
        self.scheduler
            .jit_cache()
            .lookup(self.names.module, probe, 1, self.generation)
            .is_some()
    }
}

struct Raise {
    exit: ExitReason,
    frames: usize,
    arities: Vec<u8>,
}

/// Heat the probe on its passing path, confirm it is cached, then take the fail
/// edge and report the raise-time frame count.
fn measure(
    composition: &Composition,
    entry: Atom,
    probe: Atom,
    expect_cached: bool,
) -> (bool, Raise) {
    for _ in 0..HEAT_DRIVES {
        let heat = composition.drive(entry, composition.names.pass);
        assert_eq!(
            heat.exit,
            ExitReason::Normal,
            "the passing path must exit normally"
        );
    }
    if expect_cached {
        assert!(
            wait_until(|| composition.cached(probe)),
            "the probe must be admitted and cached native for its fail edge to be \
             taken in native code"
        );
    }
    let cached = composition.cached(probe);
    let raise = composition.drive(entry, composition.names.trap);
    (cached, raise)
}

/// F7. The frame-duplication on deopt-restart is the TEMPLATE'S behaviour, not
/// this brief's: `FuncInfo` — the LEG 1c A2 precedent, untouched here — and
/// `Badmatch` — lowered by this brief with that template's body verbatim —
/// diverge from their never-JIT'd runs by the SAME amount.
#[test]
fn deopt_after_a_frame_push_duplicates_the_frame_for_the_template_and_the_new_terminal_alike() {
    let atoms = AtomTable::with_common_atoms();
    let names = Names::intern(&atoms);

    let jit = Composition::start(&atoms, true);
    let (funcinfo_cached, funcinfo_native) =
        measure(&jit, names.drive_funcinfo, names.funcinfo_probe, true);
    let (badmatch_cached, badmatch_native) =
        measure(&jit, names.drive_badmatch, names.badmatch_probe, true);
    jit.scheduler.shutdown();

    let plain = Composition::start(&atoms, false);
    let (_, funcinfo_interpreted) =
        measure(&plain, names.drive_funcinfo, names.funcinfo_probe, false);
    let (_, badmatch_interpreted) =
        measure(&plain, names.drive_badmatch, names.badmatch_probe, false);
    plain.scheduler.shutdown();

    println!("---- BEAMR-R8-DEOPT F7: deopt after a frame push ----");
    println!(
        "  FuncInfo  (the LEG 1c A2 TEMPLATE, unchanged by this brief): \
         native frames={} arities={:?} | never-JIT'd frames={} arities={:?}",
        funcinfo_native.frames,
        funcinfo_native.arities,
        funcinfo_interpreted.frames,
        funcinfo_interpreted.arities
    );
    println!(
        "  Badmatch  (THIS BRIEF, template body verbatim):              \
         native frames={} arities={:?} | never-JIT'd frames={} arities={:?}",
        badmatch_native.frames,
        badmatch_native.arities,
        badmatch_interpreted.frames,
        badmatch_interpreted.arities
    );

    assert!(
        funcinfo_cached && badmatch_cached,
        "both probes must be cached native for the comparison to be about the \
         deopt path at all"
    );
    assert_eq!(
        funcinfo_native.exit, funcinfo_interpreted.exit,
        "the template's exit reason must not diverge"
    );
    assert_eq!(
        badmatch_native.exit, badmatch_interpreted.exit,
        "this brief's terminal must not diverge in exit reason"
    );

    let template_delta = funcinfo_native.frames as i64 - funcinfo_interpreted.frames as i64;
    let terminal_delta = badmatch_native.frames as i64 - badmatch_interpreted.frames as i64;
    println!("  frame delta: template={template_delta} this-brief={terminal_delta}");
    assert_eq!(
        terminal_delta, template_delta,
        "F7 — the stacktrace-frame divergence after a deopt that follows a frame \
         push must be EXACTLY the template's. A larger delta for Badmatch would \
         mean this brief's lowering added something the func_info arm does not \
         do; it does not — the two arms are byte-identical bodies. This asserts \
         the defect belongs to the deopt-restart seam (a native `Allocate` is not \
         unwound when JIT_STATUS_DEOPT returns Ok(None)), which is pre-existing \
         and reachable today by every Supported deopt-capable instruction that \
         can follow a frame push."
    );
}
