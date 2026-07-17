use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::atom::{Atom, AtomTable};
use crate::constant_pool::ConstantPool;
use crate::ets::copy_term_to_ets;
use crate::loader::decode::compact::Operand;
use crate::loader::{Instruction, LambdaEntry, LineInfo, Literal};
use crate::module::{Module, ModuleOrigin, ResolvedImport, ResolvedImportTarget};
use crate::native::bifs::register_gate1_bifs;
use crate::native::process_bifs::register_gate2_bifs;
use crate::native::{BifRegistryImpl, ProcessContext};
use crate::process::heap::DEFAULT_HEAP_SIZE;
use crate::process::{CodePosition, ExitReason, Process, ProcessStatus, ReceiveTimeout};
use crate::term::Term;
use crate::term::boxed::Tuple;
use crate::timer::TimerKind;

#[test]
fn wasm_scheduler_starts_empty_and_runs_idle_round() {
    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let modules = Arc::new(ModuleRegistry::new());
    let bifs = Arc::new(BifRegistryImpl::new());
    let mut scheduler = WasmScheduler::new(atom_table, modules, bifs);

    let summary = scheduler.run_until_idle();

    assert_eq!(summary.executed, 0);
    assert_eq!(
        summary.state,
        WasmRunState::Idle {
            next_native_deadline: None
        }
    );
    assert!(summary.exited.is_empty());
}

#[test]
fn run_until_idle_reports_true_idle_and_earliest_native_deadline() {
    let (mut empty, module) = scheduler_with_test_module();
    assert_eq!(
        empty.run_until_idle().state,
        WasmRunState::Idle {
            next_native_deadline: None
        }
    );

    let mut receive_only = scheduler_with_test_module().0;
    let mut waiting = waiting_process(88, module);
    waiting.set_receive_timeout(Some(ReceiveTimeout {
        timeout_position: CodePosition {
            module: Atom::NIL,
            instruction_pointer: 3,
        },
        milliseconds: 25,
    }));
    receive_only.register_receive_timer(&mut waiting);
    assert_eq!(
        receive_only.run_until_idle().state,
        WasmRunState::Idle {
            next_native_deadline: None
        },
        "host receive-after records do not enter the native wheel"
    );

    let (mut native_deadline, _module) = scheduler_with_test_module();
    let now = web_time::Instant::now();
    let delay = Duration::from_secs(60);
    let expected = now + delay;
    lock_timers(&native_deadline.native_timers).schedule_at(
        now,
        delay,
        999,
        Term::small_int(1),
        TimerKind::Deliver,
    );
    assert_eq!(
        native_deadline.run_until_idle().state,
        WasmRunState::Idle {
            next_native_deadline: Some(expected)
        }
    );
}

#[test]
fn run_until_idle_preserves_errored_pid_identity() {
    let (mut scheduler, _module) = scheduler_with_test_module();
    let pid = 91;
    let mut process = Process::new(pid, DEFAULT_HEAP_SIZE);
    process
        .transition_to(ProcessStatus::Running)
        .expect("new process can run");
    scheduler.processes.insert(pid, process);
    scheduler.ready.push(pid, Priority::Normal);

    let summary = scheduler.run_until_idle();

    assert_eq!(summary.errored, vec![pid]);
    assert_eq!(
        summary.state,
        WasmRunState::Idle {
            next_native_deadline: None
        }
    );
    assert!(scheduler.has_exit_error(pid));
}

#[test]
fn receive_after_wait_schedules_and_fires_matching_timer() {
    let (mut scheduler, module) = scheduler_with_test_module();
    let pid = 42;
    let timeout_position = CodePosition {
        module: module.name,
        instruction_pointer: 7,
    };
    let mut process = waiting_process(pid, Arc::clone(&module));
    process.set_receive_timeout(Some(ReceiveTimeout {
        timeout_position,
        milliseconds: 25,
    }));

    scheduler.register_receive_timer(&mut process);
    assert_eq!(process.receive_timer_ref(), Some(1));
    assert_eq!(
        scheduler.take_pending_timer_schedules(),
        vec![WasmScheduledTimer {
            pid,
            timer_id: 1,
            milliseconds: 25,
        }]
    );
    scheduler.processes.insert(pid, process);
    scheduler.waiting.insert(pid);

    assert!(scheduler.timer_fired(pid, 1));
    let resumed = scheduler.processes.get(&pid).expect("process is retained");
    assert_eq!(resumed.receive_timer_ref(), None);
    assert_eq!(resumed.code_position(), Some(timeout_position));
    assert_eq!(resumed.status(), ProcessStatus::Running);
    assert_eq!(scheduler.ready.pop(), Some(pid));
}

#[test]
fn message_before_receive_after_cancels_pending_timer() {
    let (mut scheduler, module) = scheduler_with_test_module();
    let pid = 43;
    let mut process = waiting_process(pid, module);
    process.set_receive_timer_ref(Some(9));
    scheduler.processes.insert(pid, process);
    scheduler.waiting.insert(pid);

    assert!(scheduler.send(pid, Term::small_int(123)));

    assert_eq!(scheduler.take_pending_timer_cancellations(), vec![9]);
    let resumed = scheduler.processes.get(&pid).expect("process is retained");
    assert_eq!(resumed.receive_timer_ref(), None);
    assert_eq!(resumed.status(), ProcessStatus::Running);
    assert_eq!(scheduler.ready.pop(), Some(pid));
}

#[test]
fn host_send_owned_copies_message_into_receiver_heap_and_wakes() {
    let (mut scheduler, module) = scheduler_with_test_module();
    let pid = 47;
    let mut source = Process::new(900, DEFAULT_HEAP_SIZE);
    let mut context = ProcessContext::new();
    context.attach_process(&mut source, 0);
    let tuple = context
        .alloc_tuple(&[Term::small_int(1), Term::small_int(2)])
        .expect("source tuple allocation succeeds");
    context.detach_process();
    let owned = copy_term_to_ets(tuple).expect("tuple copies into host-owned storage");
    let process = waiting_process(pid, module);
    scheduler.processes.insert(pid, process);
    scheduler.waiting.insert(pid);

    scheduler
        .send_owned(pid, &owned)
        .expect("host-owned term sends to local pid");

    let Some(resumed) = scheduler.processes.get_mut(&pid) else {
        panic!("process is retained");
    };
    let Some(delivered) = resumed.mailbox_mut().current_message() else {
        panic!("message is visible through normal receive scan");
    };
    let delivered_tuple = Tuple::new(delivered).expect("delivered message is tuple-shaped");
    assert_eq!(delivered_tuple.get(0), Some(Term::small_int(1)));
    assert_eq!(delivered_tuple.get(1), Some(Term::small_int(2)));
    assert_eq!(resumed.status(), ProcessStatus::Running);
    assert_eq!(scheduler.ready.pop(), Some(pid));
}

#[test]
fn host_send_owned_rejects_missing_pid() {
    let (mut scheduler, _module) = scheduler_with_test_module();
    let owned = copy_term_to_ets(Term::small_int(5)).expect("immediate copies into owned storage");

    assert_eq!(scheduler.send_owned(99, &owned), Err(ExecError::Badarg));
}

#[test]
fn stale_timer_callback_is_ignored() {
    let (mut scheduler, module) = scheduler_with_test_module();
    let pid = 44;
    let mut process = waiting_process(pid, module);
    process.set_receive_timer_ref(Some(10));
    process.set_code_position(Some(CodePosition {
        module: Atom::NIL,
        instruction_pointer: 3,
    }));
    scheduler.processes.insert(pid, process);
    scheduler.waiting.insert(pid);

    assert!(!scheduler.timer_fired(pid, 11));

    let still_waiting = scheduler.processes.get(&pid).expect("process is retained");
    assert_eq!(still_waiting.receive_timer_ref(), Some(10));
    assert_eq!(still_waiting.status(), ProcessStatus::Waiting);
    assert!(scheduler.ready.pop().is_none());
}

#[test]
fn async_completion_rejects_missing_pid_without_recording_result() {
    let (mut scheduler, _module) = scheduler_with_test_module();

    assert!(!scheduler.complete_async(
        404,
        WasmAsyncCompletion::Ok(crate::ets::OwnedTerm::immediate(Term::small_int(1)))
    ));
    assert!(scheduler.async_results.is_empty());
}

#[test]
fn async_completion_injects_result_and_advances_call() {
    let (mut scheduler, module) = scheduler_with_test_module();
    let mut process = running_process(45, module);
    process.set_code_position(Some(CodePosition {
        module: Atom::NIL,
        instruction_pointer: 12,
    }));
    scheduler.async_results.insert(
        process.pid(),
        WasmAsyncCompletion::Ok(crate::ets::OwnedTerm::immediate(Term::small_int(987))),
    );

    assert_eq!(scheduler.apply_async_completion(&mut process), None);

    assert_eq!(process.x_reg(0), Term::small_int(987));
    assert_eq!(
        process.code_position(),
        Some(CodePosition {
            module: Atom::NIL,
            instruction_pointer: 13,
        })
    );
}

#[test]
fn async_rejection_maps_to_error_exit() {
    let (mut scheduler, module) = scheduler_with_test_module();
    let mut process = running_process(46, module);
    scheduler.async_results.insert(
        process.pid(),
        WasmAsyncCompletion::Error(crate::ets::OwnedTerm::immediate(Term::atom(Atom::BADARG))),
    );

    assert_eq!(
        scheduler.apply_async_completion(&mut process),
        Some(ExitReason::Error)
    );
    assert_eq!(process.x_reg(0), Term::atom(Atom::BADARG));
}

/// WPORT-3 R2: real bytecode `erlang:send_after/3`, `start_timer/3`, and
/// `cancel_timer/1` execute through `run_with_native_services` under the
/// cooperative scheduler now that `WasmScheduler::native_services` injects the
/// shared native timer wheel — the missing-service `badarg`/`false` refusal is
/// gone, and the timers land in the wheel whose earliest deadline the drain
/// result exposes.
#[test]
fn cooperative_bytecode_timer_bifs_round_trip_through_native_services() {
    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let modules = Arc::new(ModuleRegistry::new());
    let bifs = Arc::new(BifRegistryImpl::new());
    register_gate1_bifs(&bifs, &atom_table).expect("gate-1 BIFs register");
    register_gate2_bifs(&bifs, &atom_table).expect("gate-2 BIFs register");
    let mut scheduler = WasmScheduler::new(
        Arc::clone(&atom_table),
        Arc::clone(&modules),
        Arc::clone(&bifs),
    );

    let bif_import = |function: &str, arity: u8| -> ResolvedImport {
        let erlang = atom_table.intern("erlang");
        let function_atom = atom_table.intern(function);
        let entry = bifs
            .lookup(erlang, function_atom, arity)
            .expect("gate-1 timer BIF is registered");
        ResolvedImport {
            module: erlang,
            function: function_atom,
            arity,
            target: ResolvedImportTarget::Native(entry),
        }
    };
    // Imports: 0 = self/0, 1 = send_after/3, 2 = start_timer/3,
    // 3 = cancel_timer/1.
    let imports = vec![
        bif_import("self", 0),
        bif_import("send_after", 3),
        bif_import("start_timer", 3),
        bif_import("cancel_timer", 1),
    ];
    let code = vec![
        // recv_one/0: park until one message arrives, exit with it.
        Instruction::Label { label: 1 },
        Instruction::Label { label: 10 },
        Instruction::LoopRec {
            fail: Operand::Label(11),
            destination: Operand::X(0),
        },
        Instruction::RemoveMessage,
        Instruction::Return,
        Instruction::Label { label: 11 },
        Instruction::Wait {
            fail: Operand::Label(10),
        },
        // arm_send/3 (Pid, DelayMs, Msg): send_after, exit with the reference.
        Instruction::Label { label: 2 },
        Instruction::Move {
            source: Operand::X(0),
            destination: Operand::X(3),
        },
        Instruction::Move {
            source: Operand::X(1),
            destination: Operand::X(0),
        },
        Instruction::Move {
            source: Operand::X(3),
            destination: Operand::X(1),
        },
        Instruction::CallExt {
            arity: Operand::Unsigned(3),
            import: Operand::Unsigned(1),
        },
        Instruction::Return,
        // start_wait/0: start_timer(90_000, self(), 77), park, exit with the
        // delivered {timeout, Ref, Msg}.
        Instruction::Label { label: 3 },
        Instruction::CallExt {
            arity: Operand::Unsigned(0),
            import: Operand::Unsigned(0),
        },
        Instruction::Move {
            source: Operand::X(0),
            destination: Operand::X(1),
        },
        Instruction::Move {
            source: Operand::Integer(90_000),
            destination: Operand::X(0),
        },
        Instruction::Move {
            source: Operand::Integer(77),
            destination: Operand::X(2),
        },
        Instruction::CallExt {
            arity: Operand::Unsigned(3),
            import: Operand::Unsigned(2),
        },
        Instruction::Label { label: 30 },
        Instruction::LoopRec {
            fail: Operand::Label(31),
            destination: Operand::X(0),
        },
        Instruction::RemoveMessage,
        Instruction::Return,
        Instruction::Label { label: 31 },
        Instruction::Wait {
            fail: Operand::Label(30),
        },
        // cancel_ref/1 (RefId): cancel_timer, exit with remaining ms or false.
        Instruction::Label { label: 4 },
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(3),
        },
        Instruction::Return,
    ];
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            _ => None,
        })
        .collect();
    let name = atom_table.intern("wport3_timer_bifs");
    let recv_one = atom_table.intern("recv_one");
    let arm_send = atom_table.intern("arm_send");
    let start_wait = atom_table.intern("start_wait");
    let cancel_ref = atom_table.intern("cancel_ref");
    let mut exports = HashMap::new();
    exports.insert((recv_one, 0), 1);
    exports.insert((arm_send, 3), 2);
    exports.insert((start_wait, 0), 3);
    exports.insert((cancel_ref, 1), 4);
    let mut definition = dummy_module(name);
    definition.exports = exports;
    definition.label_index = label_index;
    definition.code = code;
    definition.resolved_imports = imports;
    modules.insert(definition);

    let owned = |term: Term| crate::ets::OwnedTerm::immediate(term);

    // Target parked in a plain receive.
    let target = scheduler
        .spawn_owned(name, recv_one, Vec::new())
        .expect("receive target spawns");
    let parked = scheduler.run_until_idle();
    assert_eq!(parked.waiting, vec![target]);

    // send_after/3 returns a reference and arms a native wheel deadline that
    // the settled drain result reports.
    let armer = scheduler
        .spawn_owned(
            name,
            arm_send,
            vec![
                owned(Term::pid(target)),
                owned(Term::small_int(120_000)),
                owned(Term::atom(Atom::OK)),
            ],
        )
        .expect("send_after armer spawns");
    let armed = scheduler.run_until_idle();
    assert_eq!(armed.exited, vec![armer], "send_after must not refuse");
    let reference_a = scheduler
        .take_exit_result(armer)
        .expect("armer retains its exit result")
        .root()
        .as_small_int()
        .expect("send_after returns a reference id");
    assert!(reference_a >= 1);
    assert!(
        matches!(
            armed.state,
            WasmRunState::Idle {
                next_native_deadline: Some(_)
            }
        ),
        "the bytecode-scheduled timer is reported by the settled drain"
    );

    // start_timer/3 to self: schedules the earlier (90s) deadline and parks.
    let waiter = scheduler
        .spawn_owned(name, start_wait, Vec::new())
        .expect("start_wait spawns");
    let waiting = scheduler.run_until_idle();
    assert!(waiting.waiting.contains(&waiter));

    // cancel_timer/1: remaining milliseconds for the pending reference, then
    // false on the second cancel.
    let cancel_one = scheduler
        .spawn_owned(name, cancel_ref, vec![owned(Term::small_int(reference_a))])
        .expect("first cancel spawns");
    let _run = scheduler.run_until_idle();
    let remaining = scheduler
        .take_exit_result(cancel_one)
        .expect("first cancel retains its exit result")
        .root()
        .as_small_int()
        .expect("first cancel returns remaining milliseconds");
    assert!(remaining > 0 && remaining <= 120_000);
    let cancel_two = scheduler
        .spawn_owned(name, cancel_ref, vec![owned(Term::small_int(reference_a))])
        .expect("second cancel spawns");
    let _run = scheduler.run_until_idle();
    assert_eq!(
        scheduler
            .take_exit_result(cancel_two)
            .expect("second cancel retains its exit result")
            .root(),
        Term::atom(Atom::FALSE),
        "cancel-after-cancel returns false"
    );

    // A deliberately late deterministic tick delivers the due start_timer
    // deadline exactly once; the cancelled send_after delivers nothing.
    let woken =
        scheduler.tick_native_timers_at(web_time::Instant::now() + Duration::from_millis(150_000));
    assert_eq!(woken, vec![waiter], "only the live due timer fires");
    let resumed = scheduler.run_until_idle();
    assert_eq!(resumed.exited, vec![waiter]);
    let delivered = scheduler
        .take_exit_result(waiter)
        .expect("waiter retains its exit result");
    let tuple = Tuple::new(delivered.root()).expect("delivered {timeout, Ref, Msg} tuple");
    assert_eq!(tuple.get(0), Some(Term::atom(Atom::TIMEOUT)));
    let reference_w = tuple
        .get(1)
        .and_then(|term| term.as_small_int())
        .expect("delivered tuple carries the timer reference");
    assert!(reference_w >= 1);
    assert_eq!(tuple.get(2), Some(Term::small_int(77)));
    assert!(
        scheduler.waiting.contains(&target),
        "cancel-before-fire suppressed the send_after delivery"
    );

    // cancel-after-fire returns false and cannot retract the delivery.
    let cancel_fired = scheduler
        .spawn_owned(name, cancel_ref, vec![owned(Term::small_int(reference_w))])
        .expect("cancel-after-fire spawns");
    let _run = scheduler.run_until_idle();
    assert_eq!(
        scheduler
            .take_exit_result(cancel_fired)
            .expect("cancel-after-fire retains its exit result")
            .root(),
        Term::atom(Atom::FALSE)
    );
}

/// WPORT-5 P11 pin: on a context with NO timer facility, `cancel_timer/1`
/// answers atom `false` — a missing-service answer indistinguishable from
/// "timer already fired/cancelled". With the facility wired (WPORT-3) the
/// same `false` means already-fired; this pin freezes the bare-context shape
/// so a wiring regression cannot silently reintroduce it unclassified.
#[test]
fn bare_context_cancel_timer_answers_false_indistinguishable_from_fired() {
    let mut context = ProcessContext::new();
    let result = crate::native::bifs::cancel_timer(&[Term::small_int(41)], &mut context)
        .expect("bare-context cancel_timer answers rather than raises");
    assert_eq!(
        result,
        Term::atom(Atom::FALSE),
        "the missing-service answer is the same atom `false` a fired timer produces"
    );
}

/// WPORT-5 P7/P10 pin (scheduler-level): a dirty-registered native reached
/// from cooperative bytecode never runs its body — dispatch returns
/// `DirtyCall` and the wasm scheduler converts it to the process-fatal
/// `ExecError::UnsupportedOpcode { name: "dirty native call on wasm" }`,
/// consumable via the cooperative `take_exit_error` (WPORT-5 R2 item 7).
/// Uses the REAL `timer:sleep/1` registration (the only dirty entry on the
/// browser surface).
#[test]
fn dirty_native_call_errors_the_process_with_unsupported_opcode() {
    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let modules = Arc::new(ModuleRegistry::new());
    let bifs = Arc::new(BifRegistryImpl::new());
    crate::native::stdlib_stubs::register_stdlib_stubs(&bifs, &atom_table)
        .expect("stdlib stubs register");
    let mut scheduler = WasmScheduler::new(
        Arc::clone(&atom_table),
        Arc::clone(&modules),
        Arc::clone(&bifs),
    );

    let timer = atom_table.intern("timer");
    let sleep = atom_table.intern("sleep");
    let entry = bifs
        .lookup(timer, sleep, 1)
        .expect("timer:sleep/1 is registered");
    assert!(entry.dirty_kind.is_some(), "timer:sleep/1 is dirty-marked");
    let imports = vec![ResolvedImport {
        module: timer,
        function: sleep,
        arity: 1,
        target: ResolvedImportTarget::Native(entry),
    }];
    let code = vec![
        Instruction::Label { label: 1 },
        Instruction::Move {
            source: Operand::Integer(1),
            destination: Operand::X(0),
        },
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
        Instruction::Return,
    ];
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            _ => None,
        })
        .collect();
    let name = atom_table.intern("wport5_dirty_pin");
    let sleepy = atom_table.intern("sleepy");
    let mut exports = HashMap::new();
    exports.insert((sleepy, 0), 1);
    let mut definition = dummy_module(name);
    definition.exports = exports;
    definition.label_index = label_index;
    definition.code = code;
    definition.resolved_imports = imports;
    modules.insert(definition);

    let pid = scheduler
        .spawn_owned(name, sleepy, Vec::new())
        .expect("sleepy spawns");
    let summary = scheduler.run_until_idle();
    assert_eq!(summary.errored, vec![pid], "the dirty call errors the pid");
    assert!(scheduler.has_exit_error(pid));
    assert_eq!(
        scheduler.take_exit_error(pid),
        Some(crate::error::ExecError::UnsupportedOpcode {
            name: "dirty native call on wasm",
        }),
    );
    assert_eq!(
        scheduler.take_exit_error(pid),
        None,
        "take_exit_error consumes the record"
    );
}

/// WPORT-5 send-drop TOMBSTONE, named for the retired behaviour: before R2
/// item 1, cross-process bytecode `Pid ! Msg` on the cooperative scheduler
/// silently dropped the message while reporting success (its only spec was
/// the messaging.rs fall-through comment). With `local_send` injected into
/// bytecode `NativeServices`, delivery now occurs: the receiver observes the
/// payload and the sender's x0 carries the message.
#[test]
fn retired_silent_send_drop_cross_process_bytecode_send_now_delivers() {
    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let modules = Arc::new(ModuleRegistry::new());
    let bifs = Arc::new(BifRegistryImpl::new());
    let mut scheduler = WasmScheduler::new(
        Arc::clone(&atom_table),
        Arc::clone(&modules),
        Arc::clone(&bifs),
    );

    let code = vec![
        // recv_one/0: park until one message arrives, exit with it.
        Instruction::Label { label: 1 },
        Instruction::Label { label: 10 },
        Instruction::LoopRec {
            fail: Operand::Label(11),
            destination: Operand::X(0),
        },
        Instruction::RemoveMessage,
        Instruction::Return,
        Instruction::Label { label: 11 },
        Instruction::Wait {
            fail: Operand::Label(10),
        },
        // send_to/2 (Pid, Msg): the Send opcode (x0 = pid, x1 = msg), then
        // exit with x0 (which Send sets to the message).
        Instruction::Label { label: 2 },
        Instruction::Send,
        Instruction::Return,
    ];
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            _ => None,
        })
        .collect();
    let name = atom_table.intern("wport5_send_tombstone");
    let recv_one = atom_table.intern("recv_one");
    let send_to = atom_table.intern("send_to");
    let mut exports = HashMap::new();
    exports.insert((recv_one, 0), 1);
    exports.insert((send_to, 2), 2);
    let mut definition = dummy_module(name);
    definition.exports = exports;
    definition.label_index = label_index;
    definition.code = code;
    modules.insert(definition);

    let owned = |term: Term| crate::ets::OwnedTerm::immediate(term);
    let receiver = scheduler
        .spawn_owned(name, recv_one, Vec::new())
        .expect("receiver spawns");
    let parked = scheduler.run_until_idle();
    assert_eq!(parked.waiting, vec![receiver]);

    let payload = atom_table.intern("wport5_payload");
    let sender = scheduler
        .spawn_owned(
            name,
            send_to,
            vec![owned(Term::pid(receiver)), owned(Term::atom(payload))],
        )
        .expect("sender spawns");
    let summary = scheduler.run_until_idle();
    assert!(summary.exited.contains(&sender));
    assert!(
        summary.exited.contains(&receiver),
        "the delivered message wakes the receiver within the same drain"
    );
    assert_eq!(
        scheduler
            .take_exit_result(sender)
            .expect("sender retains its exit result")
            .root(),
        Term::atom(payload),
        "the sender's x0 carries the message"
    );
    assert_eq!(
        scheduler
            .take_exit_result(receiver)
            .expect("receiver retains its exit result")
            .root(),
        Term::atom(payload),
        "nothing is dropped: the receiver observed the exact payload"
    );
}

fn scheduler_with_test_module() -> (WasmScheduler, Arc<Module>) {
    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let modules = Arc::new(ModuleRegistry::new());
    let bifs = Arc::new(BifRegistryImpl::new());
    let module = Arc::new(dummy_module(Atom::NIL));
    (WasmScheduler::new(atom_table, modules, bifs), module)
}

fn waiting_process(pid: u64, module: Arc<Module>) -> Process {
    let mut process = running_process(pid, module);
    process
        .transition_to(ProcessStatus::Waiting)
        .expect("running process can wait");
    process
}

fn running_process(pid: u64, module: Arc<Module>) -> Process {
    let mut process = Process::new(pid, DEFAULT_HEAP_SIZE);
    process.set_current_module(module);
    process
        .transition_to(ProcessStatus::Running)
        .expect("new process can run");
    process
}

fn dummy_module(name: Atom) -> Module {
    Module {
        name,
        generation: 0,
        origin: ModuleOrigin::Preloaded,
        exports: HashMap::new(),
        label_index: HashMap::new(),
        code: Vec::<Instruction>::new(),
        function_table: Vec::new(),
        line_table: Vec::new(),
        literals: Vec::<Literal>::new(),
        constant_pool: ConstantPool::new(),
        resolved_imports: Vec::<ResolvedImport>::new(),
        lambdas: Vec::<LambdaEntry>::new(),
        string_table: Vec::new(),
        line_info: Vec::<LineInfo>::new(),
    }
}
