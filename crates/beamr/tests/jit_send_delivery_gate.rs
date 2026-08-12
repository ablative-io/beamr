//! Gate test for cross-process send under COMPILED execution (lane #88).
//!
//! The defect: `jit_send_message` (jit/runtime_message.rs) delivers only when
//! the destination pid is the sending process itself; every other destination
//! is silently dropped while the helper returns the message term — the correct
//! `!` result — so compiled code continues as if delivery succeeded. The
//! interpreter's `Send` path delivers through the scheduler's
//! `LocalSendFacility` (see `local_send_gate.rs`, which gated the interpreter's
//! own historical version of this same silent-drop shape).
//!
//! Two arms:
//! * `jit_compiled_cross_process_send_delivers` — heats `fire/1` over the JIT
//!   threshold, proves native dispatch via the record-on-miss-only counter,
//!   then drives ONE compiled send at a fresh receiver and asserts delivery.
//!   EXPECTED RED while the defect is live: the receiver never gets the
//!   message and the bounded watchdog fails the test.
//! * `interpreted_cross_process_send_delivers_control` — the identical
//!   program with the JIT threshold at `u32::MAX` (never fires). GREEN,
//!   proving the harness and pinning the divergence to compiled execution.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use beamr::atom::{Atom, AtomTable};
use beamr::loader::Instruction;
use beamr::loader::decode::compact::Operand;
use beamr::module::{Module, ModuleOrigin, ModuleRegistry};
use beamr::process::ExitReason;
use beamr::scheduler::{Scheduler, SchedulerConfig};
use beamr::term::Term;

const WAIT_BUDGET: Duration = Duration::from_secs(10);
const DELIVERY_BUDGET: Duration = Duration::from_secs(8);
const THRESHOLD: u32 = 3;

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

fn config(threshold: u32) -> SchedulerConfig {
    SchedulerConfig {
        thread_count: Some(1),
        dirty_cpu_threads: Some(1),
        dirty_io_threads: Some(1),
        jit_threshold: Some(threshold),
        ..SchedulerConfig::default()
    }
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

/// The sender module:
/// * `fire/1` (label 2): x0 = destination pid; moves the `ping` atom into
///   x1 and runs the real `Send` opcode — the JIT-supported tail-send shape.
/// * `warm/1` (label 1): x0 = destination pid, preserved in y0; makes
///   `THRESHOLD` local calls to `fire/1` so its call edges heat the profile.
/// * `drive/1` (label 3): x0 = destination pid; exactly ONE call to `fire/1`
///   — the post-compile probe, so no send ever targets a dead pid.
fn sender_module(atoms: &AtomTable, name: Atom, ping: Atom) -> Module {
    let fire = atoms.intern("fire");
    let mut code = vec![
        Instruction::Label { label: 1 },
        Instruction::Allocate {
            stack_need: Operand::Unsigned(1),
            live: Operand::Unsigned(1),
        },
        Instruction::Move {
            source: Operand::X(0),
            destination: Operand::Y(0),
        },
    ];
    for _ in 0..THRESHOLD {
        code.push(Instruction::Move {
            source: Operand::Y(0),
            destination: Operand::X(0),
        });
        code.push(Instruction::Call {
            arity: Operand::Unsigned(1),
            label: Operand::Label(2),
        });
    }
    code.push(Instruction::Deallocate {
        words: Operand::Unsigned(1),
    });
    code.push(Instruction::Return);
    code.extend([
        Instruction::FuncInfo {
            module: Operand::Atom(Some(name)),
            function: Operand::Atom(Some(fire)),
            arity: Operand::Unsigned(1),
        },
        Instruction::Label { label: 2 },
        Instruction::Move {
            source: Operand::Atom(Some(ping)),
            destination: Operand::X(1),
        },
        Instruction::Send,
        Instruction::Return,
        Instruction::Label { label: 3 },
        Instruction::Call {
            arity: Operand::Unsigned(1),
            label: Operand::Label(2),
        },
        Instruction::Return,
    ]);
    let mut exports = HashMap::new();
    exports.insert((atoms.intern("warm"), 1), 1);
    exports.insert((atoms.intern("drive"), 1), 3);
    exports.insert((fire, 1), 2);
    finish_module(name, code, exports)
}

/// `loop/0`: `receive Msg -> Msg end` — parks until a message arrives, then
/// returns it in x0 (exits normally). From `local_send_gate.rs`.
fn receiver_module(atoms: &AtomTable, name: Atom) -> Module {
    let function = atoms.intern("loop");
    let mut exports = HashMap::new();
    exports.insert((function, 0), 1);
    finish_module(
        name,
        vec![
            Instruction::Label { label: 1 },
            Instruction::Label { label: 10 },
            Instruction::LoopRec {
                fail: Operand::Label(20),
                destination: Operand::X(0),
            },
            Instruction::RemoveMessage,
            Instruction::Return,
            Instruction::Label { label: 20 },
            Instruction::Wait {
                fail: Operand::Label(10),
            },
        ],
        exports,
    )
}

/// `sink/0`: receives forever, discarding every message — the warm-up target,
/// so warm-up deliveries never hit a dead pid.
fn sink_module(atoms: &AtomTable, name: Atom) -> Module {
    let function = atoms.intern("sink");
    let mut exports = HashMap::new();
    exports.insert((function, 0), 1);
    finish_module(
        name,
        vec![
            Instruction::Label { label: 1 },
            Instruction::Label { label: 10 },
            Instruction::LoopRec {
                fail: Operand::Label(20),
                destination: Operand::X(0),
            },
            Instruction::RemoveMessage,
            Instruction::Jump {
                target: Operand::Label(10),
            },
            Instruction::Label { label: 20 },
            Instruction::Wait {
                fail: Operand::Label(10),
            },
        ],
        exports,
    )
}

struct Setup {
    scheduler: Arc<Scheduler>,
    atoms: Arc<AtomTable>,
    sender_name: Atom,
    receiver_name: Atom,
    ping: Atom,
}

fn setup(threshold: u32) -> Setup {
    let atoms = Arc::new(AtomTable::with_common_atoms());
    let sender_name = atoms.intern("jit_send_gate_sender");
    let receiver_name = atoms.intern("jit_send_gate_receiver");
    let sink_name = atoms.intern("jit_send_gate_sink");
    let ping = atoms.intern("ping");

    let registry = Arc::new(ModuleRegistry::new());
    registry.insert(sender_module(&atoms, sender_name, ping));
    registry.insert(receiver_module(&atoms, receiver_name));
    registry.insert(sink_module(&atoms, sink_name));

    let scheduler = Arc::new(
        // 0.17.0 backport: `Scheduler::new` takes no native-BIF argument at this
        // base and constructs an empty `BifRegistryImpl` internally, which is
        // exactly what `NativeBifs::none()` names explicitly on the 0.18.x line.
        // The gate's native surface is therefore unchanged by the adaptation.
        Scheduler::new(config(threshold), registry)
            .unwrap_or_else(|error| panic!("scheduler starts: {error}")),
    );
    Setup {
        scheduler,
        atoms,
        sender_name,
        receiver_name,
        ping,
    }
}

fn run_to_exit(scheduler: &Arc<Scheduler>, pid: u64, what: &str) -> (ExitReason, Term) {
    let (tx, rx) = std::sync::mpsc::channel();
    let scheduler = Arc::clone(scheduler);
    std::thread::spawn(move || {
        let _ = tx.send(scheduler.run_until_exit(pid));
    });
    let (reason, result) = rx
        .recv_timeout(DELIVERY_BUDGET)
        .unwrap_or_else(|_| panic!("{what}: never exited within the delivery budget"));
    (reason, result.root())
}

/// Common body: warm `fire/1` to `threshold` at a sink, then drive exactly one
/// send at a fresh receiver and assert the receiver gets it. `expect_compiled`
/// selects whether the drive-phase send must run native (arm 1) or stay
/// interpreted (arm 2).
fn send_delivery_arm(threshold: u32, expect_compiled: bool) {
    let s = setup(threshold);
    let scheduler = &s.scheduler;
    let fire = s.atoms.intern("fire");
    let warm = s.atoms.intern("warm");
    let drive = s.atoms.intern("drive");
    let looper = s.atoms.intern("loop");
    let sink_name = s.atoms.intern("jit_send_gate_sink");
    let sink_fn = s.atoms.intern("sink");

    // Warm-up: heat fire/1's call edge at a sink that never dies.
    let sink_pid = scheduler
        .spawn(sink_name, sink_fn, Vec::new())
        .expect("spawn sink");
    std::thread::sleep(Duration::from_millis(50));
    let warm_pid = scheduler
        .spawn(
            s.sender_name,
            warm,
            vec![Term::try_pid(sink_pid).expect("sink pid fits")],
        )
        .expect("spawn warm");
    let (warm_reason, _) = run_to_exit(scheduler, warm_pid, "warm-up sender");
    assert_eq!(
        warm_reason,
        ExitReason::Normal,
        "warm-up must exit normally"
    );

    if expect_compiled {
        assert_eq!(
            scheduler
                .jit_profiler()
                .compile_outcome_counters()
                .submissions,
            1,
            "the threshold-crossing call must submit fire/1"
        );
        assert!(
            wait_until(|| {
                scheduler
                    .jit_profiler()
                    .compile_outcome_counters()
                    .successes
                    == 1
            }),
            "fire/1 must compile within the wait budget"
        );
        assert!(
            scheduler.jit_profiler().is_compiled(s.sender_name, fire, 1),
            "fire/1 must be COMPILED before the drive phase"
        );
    } else {
        assert_eq!(
            scheduler
                .jit_profiler()
                .compile_outcome_counters()
                .submissions,
            0,
            "the control arm must never submit"
        );
    }
    let heat_before_drive = scheduler
        .jit_profiler()
        .recorded_call_count(s.sender_name, fire, 1);

    // Drive: ONE send at a fresh receiver.
    let receiver_pid = scheduler
        .spawn(s.receiver_name, looper, Vec::new())
        .expect("spawn receiver");
    std::thread::sleep(Duration::from_millis(50));
    let drive_pid = scheduler
        .spawn(
            s.sender_name,
            drive,
            vec![Term::try_pid(receiver_pid).expect("receiver pid fits")],
        )
        .expect("spawn drive");
    let (drive_reason, _) = run_to_exit(scheduler, drive_pid, "drive sender");
    assert_eq!(drive_reason, ExitReason::Normal, "drive must exit normally");

    if expect_compiled {
        // Record-on-miss-only: an unmoved counter proves the drive call
        // dispatched NATIVE — the send under test executed from compiled code.
        assert_eq!(
            scheduler
                .jit_profiler()
                .recorded_call_count(s.sender_name, fire, 1),
            heat_before_drive,
            "the drive call must dispatch native (counter unmoved), \
             otherwise this arm proved nothing about compiled sends"
        );
    }

    // The assertion under test: the receiver must get the message.
    let (tx, rx) = std::sync::mpsc::channel();
    let scheduler_for_wait = Arc::clone(scheduler);
    std::thread::spawn(move || {
        let _ = tx.send(scheduler_for_wait.run_until_exit(receiver_pid));
    });
    let (reason, result) = rx.recv_timeout(DELIVERY_BUDGET).unwrap_or_else(|_| {
        panic!(
            "receiver never got the cross-process message \
             (silent drop under {} execution)",
            if expect_compiled {
                "COMPILED"
            } else {
                "interpreted"
            }
        )
    });
    assert_eq!(reason, ExitReason::Normal);
    assert_eq!(
        result.root(),
        Term::atom(s.ping),
        "the receiver must return the exact message sent via the Send opcode"
    );

    scheduler.shutdown();
}

/// Arm 1 (RED while the defect is live): a compiled `B ! Msg` must deliver.
#[test]
fn jit_compiled_cross_process_send_delivers() {
    send_delivery_arm(THRESHOLD, true);
}

/// Arm 2 (GREEN control): the identical program interpreted end-to-end
/// delivers — pinning the divergence to compiled execution.
#[test]
fn interpreted_cross_process_send_delivers_control() {
    send_delivery_arm(u32::MAX, false);
}
