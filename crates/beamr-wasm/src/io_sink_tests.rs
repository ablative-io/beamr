//! WPORT-7 R3 wall battery + io_sink's first unit tests (zero existed at the
//! pin): the promoted ordering contract — flush-before-waiter-resolution on
//! both paths, the OQ-B HOLD-DRAINING re-entrancy refusal, the one-split-point
//! fallback, cross-process/multi-turn total FIFO, the `io:format/2`
//! facility-absent refusal — plus buffer FIFO and lossy-UTF-8 units.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use beamr::io_sink::IoStream;
use beamr::loader::decode::compact::Operand;
use beamr::loader::{Instruction, Literal};
use js_sys::{Array, Function, Reflect};
use serde_json::json;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test;

use super::HostIoSinkBridge;
use crate::WasmVm;
use crate::failure::{assert_typed, error_name, install_throwing_double};
use crate::tests::{
    await_exit_json, build_module_with_literals, host_macrotask, host_microtask, parse_json,
    registered_bif_import,
};

type SinkBank = Rc<RefCell<Vec<(String, String)>>>;
type SinkGuard = Closure<dyn FnMut(JsValue, JsValue)>;

fn capture_closure(bank: &SinkBank) -> SinkGuard {
    let bank = Rc::clone(bank);
    Closure::<dyn FnMut(JsValue, JsValue)>::new(move |stream: JsValue, text: JsValue| {
        bank.borrow_mut().push((
            stream.as_string().unwrap_or_default(),
            text.as_string().unwrap_or_default(),
        ));
    })
}

/// `run/0`: `io:put_chars/1` each literal line in order, then exit.
fn print_lines_module(vm: &WasmVm, name: &str, lines: &[&str]) -> beamr::module::Module {
    let literals: Vec<Literal> = lines
        .iter()
        .map(|line| Literal::Binary(line.as_bytes().to_vec()))
        .collect();
    let imports = vec![registered_bif_import(vm, "io", "put_chars", 1)];
    let mut code = vec![Instruction::Label { label: 1 }];
    for index in 0..lines.len() {
        code.push(Instruction::Move {
            source: Operand::Literal(index),
            destination: Operand::X(0),
        });
        code.push(Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        });
    }
    code.push(Instruction::Return);
    build_module_with_literals(
        &vm.atom_table,
        name,
        &[("run", 0, 1)],
        code,
        imports,
        literals,
    )
}

fn spawn_run(vm: &WasmVm, module: beamr::module::Module) -> u64 {
    let module_name = module.name;
    let run = vm.atom_table.intern("run");
    vm.module_registry.insert(module);
    vm.scheduler
        .borrow_mut()
        .spawn_owned(module_name, run, Vec::new())
        .expect("bytecode process spawns")
}

/// Unit: the scheduler-wide buffer is ONE FIFO across both stream tags —
/// untagged writes are stdout-flavoured, order is write order.
#[wasm_bindgen_test]
fn buffer_preserves_write_order_across_stream_tags() {
    let bridge = HostIoSinkBridge::new();
    let sink = bridge.scheduler_sink();
    sink.write(b"a");
    sink.write_stream(IoStream::Err, b"b");
    sink.write_stream(IoStream::Out, b"c");
    sink.write_stream(IoStream::Err, b"d");
    let bank: SinkBank = Rc::new(RefCell::new(Vec::new()));
    let guard = capture_closure(&bank);
    bridge.register(guard.as_ref().unchecked_ref::<Function>().clone());
    bridge.flush();
    assert_eq!(
        *bank.borrow(),
        vec![
            ("out".to_owned(), "a".to_owned()),
            ("err".to_owned(), "b".to_owned()),
            ("out".to_owned(), "c".to_owned()),
            ("err".to_owned(), "d".to_owned()),
        ],
        "one FIFO, write order, correct tags"
    );
    bridge.flush();
    assert_eq!(bank.borrow().len(), 4, "an empty flush delivers nothing");
    drop(guard);
}

/// Unit: bytes are decoded as LOSSY UTF-8 at delivery — invalid sequences
/// become replacement characters, valid tails survive.
#[wasm_bindgen_test]
fn flush_decodes_bytes_as_lossy_utf8() {
    let bridge = HostIoSinkBridge::new();
    let sink = bridge.scheduler_sink();
    sink.write_stream(IoStream::Out, b"\xff\xfeok");
    let bank: SinkBank = Rc::new(RefCell::new(Vec::new()));
    let guard = capture_closure(&bank);
    bridge.register(guard.as_ref().unchecked_ref::<Function>().clone());
    bridge.flush();
    let delivered = bank.borrow()[0].1.clone();
    assert_eq!(delivered, "\u{FFFD}\u{FFFD}ok", "lossy decode, tail intact");
    drop(guard);
}

/// Wall 9 (R4, D10a): a throwing callback produces exactly ONE split point
/// per flush — pre-throw writes via the callback, every remaining write via
/// the console, order preserved within each channel, and the callback is
/// retried at the next flush.
#[wasm_bindgen_test]
fn throwing_callback_switches_flush_remainder_to_console_at_one_split_point() {
    let bridge = HostIoSinkBridge::new();
    let sink = bridge.scheduler_sink();
    let global = js_sys::global();
    let js_bank = Array::new();
    let _set = Reflect::set(&global, &JsValue::from_str("__wport7_sink_bank"), &js_bank);
    bridge.register(Function::new_with_args(
        "stream, text",
        "if (text === 'w2') { throw new Error('sink boom'); } \
         globalThis.__wport7_sink_bank.push(stream + ':' + text);",
    ));

    let console = Reflect::get(&global, &JsValue::from_str("console")).expect("console exists");
    let original_log = Reflect::get(&console, &JsValue::from_str("log")).expect("log exists");
    let original_error = Reflect::get(&console, &JsValue::from_str("error")).expect("error exists");
    let console_bank: SinkBank = Rc::new(RefCell::new(Vec::new()));
    let log_bank = Rc::clone(&console_bank);
    let log_spy = Closure::<dyn FnMut(JsValue)>::new(move |text: JsValue| {
        log_bank
            .borrow_mut()
            .push(("log".to_owned(), text.as_string().unwrap_or_default()));
    });
    let error_bank = Rc::clone(&console_bank);
    let error_spy = Closure::<dyn FnMut(JsValue)>::new(move |text: JsValue| {
        error_bank
            .borrow_mut()
            .push(("error".to_owned(), text.as_string().unwrap_or_default()));
    });
    let _log = Reflect::set(&console, &JsValue::from_str("log"), log_spy.as_ref());
    let _error = Reflect::set(&console, &JsValue::from_str("error"), error_spy.as_ref());

    sink.write_stream(IoStream::Out, b"w1");
    sink.write_stream(IoStream::Out, b"w2");
    sink.write_stream(IoStream::Err, b"w3");
    sink.write_stream(IoStream::Out, b"w4");
    bridge.flush();
    // Retry happens no earlier than the NEXT flush: w5 rides the callback.
    sink.write_stream(IoStream::Out, b"w5");
    bridge.flush();

    let _restore_log = Reflect::set(&console, &JsValue::from_str("log"), &original_log);
    let _restore_error = Reflect::set(&console, &JsValue::from_str("error"), &original_error);
    drop(log_spy);
    drop(error_spy);

    let callback_seen: Vec<String> = js_bank.iter().filter_map(|v| v.as_string()).collect();
    assert_eq!(
        callback_seen,
        vec!["out:w1".to_owned(), "out:w5".to_owned()],
        "pre-throw writes via the callback; the NEXT flush retries it"
    );
    assert_eq!(
        *console_bank.borrow(),
        vec![
            ("log".to_owned(), "w2".to_owned()),
            ("error".to_owned(), "w3".to_owned()),
            ("log".to_owned(), "w4".to_owned()),
        ],
        "one split point: the whole remainder lands on the console, in order"
    );
}

/// R3 acceptance (D9a, success path): waiter continuations observe ALL of the
/// turn's sink output already delivered — and at flush time the waiters are
/// provably UNRESOLVED (the exit result is still unconsumed), so
/// flush-before-waiter-resolution is construction, not microtask luck.
#[wasm_bindgen_test]
async fn flush_completes_before_waiter_resolution_on_success_path() {
    let mut vm = WasmVm::new().expect("VM constructs");
    let bank: Rc<RefCell<Vec<(String, bool)>>> = Rc::new(RefCell::new(Vec::new()));
    let probe_bank = Rc::clone(&bank);
    let scheduler = Rc::clone(&vm.scheduler);
    let pid_cell: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    let probe_pid = Rc::clone(&pid_cell);
    let sink =
        Closure::<dyn FnMut(JsValue, JsValue)>::new(move |_stream: JsValue, text: JsValue| {
            // Probe: is the printing process's exit result still unconsumed at
            // delivery time? Waiter resolution consumes it — so `true` here
            // proves the flush ran BEFORE `resolve_waiters`.
            let unresolved = scheduler
                .borrow()
                .exit_results()
                .iter()
                .any(|(pid, _)| *pid == probe_pid.get());
            probe_bank
                .borrow_mut()
                .push((text.as_string().unwrap_or_default(), unresolved));
        });
    vm.register_io_sink(sink.as_ref().unchecked_ref::<Function>().clone());
    let module = print_lines_module(&vm, "wport7_flush_order", &["l1\n", "l2\n"]);
    let pid = spawn_run(&vm, module);
    pid_cell.set(pid);
    let waiter = vm.await_exit(pid);
    let summary = vm.run_step().expect("the drain succeeds");
    assert!(summary.as_string().expect("summary JSON").contains("idle"));

    let exited = parse_json(
        JsFuture::from(waiter)
            .await
            .expect("waiter resolves exited"),
    );
    assert_eq!(exited["state"], "exited");
    // The continuation observes the full output, already delivered...
    assert_eq!(
        *bank.borrow(),
        vec![("l1\n".to_owned(), true), ("l2\n".to_owned(), true)],
        "both lines delivered before waiter resolution (probe true at both)"
    );
    drop(sink);
}

/// Wall 12 (R4): flush-on-failed-drain, now constructional AND ordered — a
/// drain that fails at the reconcile seam still delivers everything the turn
/// captured, BEFORE the typed error reaches the caller or any waiter.
#[wasm_bindgen_test]
async fn flush_on_failed_drain_delivers_output_before_typed_rejection() {
    let set_double = install_throwing_double("setTimeout");
    let mut vm = WasmVm::new().expect("VM constructs");
    let bank: SinkBank = Rc::new(RefCell::new(Vec::new()));
    let sink = capture_closure(&bank);
    vm.register_io_sink(sink.as_ref().unchecked_ref::<Function>().clone());
    // `run/0`: print one line, then park in a 60s receive-after — the re-arm
    // forces the failed reconcile while the turn holds captured output.
    let literals = vec![Literal::Binary(b"mid\n".to_vec())];
    let imports = vec![registered_bif_import(&vm, "io", "put_chars", 1)];
    let code = vec![
        Instruction::Label { label: 1 },
        Instruction::Move {
            source: Operand::Literal(0),
            destination: Operand::X(0),
        },
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
        Instruction::Label { label: 10 },
        Instruction::LoopRec {
            fail: Operand::Label(20),
            destination: Operand::X(0),
        },
        Instruction::RemoveMessage,
        Instruction::Return,
        Instruction::Label { label: 20 },
        Instruction::WaitTimeout {
            fail: Operand::Label(10),
            timeout: Operand::Unsigned(60_000),
        },
        Instruction::Timeout,
        Instruction::Return,
    ];
    let module = build_module_with_literals(
        &vm.atom_table,
        "wport7_failed_flush",
        &[("run", 0, 1)],
        code,
        imports,
        literals,
    );
    let pid = spawn_run(&vm, module);
    let waiter = vm.await_exit(pid);

    set_double.set_throwing(true);
    let error = vm
        .run_step()
        .expect_err("the parking drain fails at the arm");
    set_double.set_throwing(false);

    assert_typed(&error, "manual", "reconcile");
    assert_eq!(
        *bank.borrow(),
        vec![("out".to_owned(), "mid\n".to_owned())],
        "the failed turn's output was delivered before the Err returned"
    );
    let rejection = JsFuture::from(waiter)
        .await
        .expect_err("parked waiter rejects after the flush");
    assert_typed(&rejection, "manual", "reconcile");
    drop(sink);
    drop(vm);
    set_double.restore();
}

/// R3 acceptance (D10b, OQ-B HOLD-DRAINING): a sink callback synchronously
/// re-entering `run_step` mid-flush receives the EXISTING already-draining
/// refusal — a plain string, deliberately NOT a `SchedulerFailureError` (the
/// sync caller-misuse class stays distinct) — no nested drain runs, and the
/// flush order is untouched.
#[wasm_bindgen_test]
fn reentrant_run_step_mid_flush_receives_already_draining_refusal() {
    let mut vm = WasmVm::new().expect("VM constructs");
    let arbiter = Rc::clone(&vm.arbiter);
    let banked: Rc<RefCell<Vec<(bool, String, String)>>> = Rc::new(RefCell::new(Vec::new()));
    let reentry_bank = Rc::clone(&banked);
    let order_bank: SinkBank = Rc::new(RefCell::new(Vec::new()));
    let order = Rc::clone(&order_bank);
    let attempted = Cell::new(false);
    let sink =
        Closure::<dyn FnMut(JsValue, JsValue)>::new(move |stream: JsValue, text: JsValue| {
            if !attempted.get() {
                attempted.set(true);
                match arbiter.run_manual_drain() {
                    Ok(_) => reentry_bank.borrow_mut().push((
                        false,
                        String::from("nested drain ran"),
                        String::new(),
                    )),
                    Err(error) => reentry_bank.borrow_mut().push((
                        true,
                        error.as_string().unwrap_or_default(),
                        error_name(&error),
                    )),
                }
            }
            order.borrow_mut().push((
                stream.as_string().unwrap_or_default(),
                text.as_string().unwrap_or_default(),
            ));
        });
    vm.register_io_sink(sink.as_ref().unchecked_ref::<Function>().clone());
    let module = print_lines_module(&vm, "wport7_reentry", &["r1\n", "r2\n"]);
    let _pid = spawn_run(&vm, module);
    let summary = vm.run_step().expect("the outer manual drain succeeds");
    assert!(summary.as_string().expect("summary JSON").contains("idle"));

    assert_eq!(
        *banked.borrow(),
        vec![(
            true,
            "arbiter is already draining".to_owned(),
            String::new(),
        )],
        "the re-entrant call was refused with the existing string, not typed"
    );
    assert_eq!(
        *order_bank.borrow(),
        vec![
            ("out".to_owned(), "r1\n".to_owned()),
            ("out".to_owned(), "r2\n".to_owned()),
        ],
        "no newer-before-older delivery: FIFO order intact through the refusal"
    );
    drop(sink);
}

/// Wall 7 (R4, D9b): cross-process interleave equals slice order — two
/// processes' writes stream through the one FIFO exactly as the scheduler
/// sliced them (spawn order, run-to-exit slices at this workload).
#[wasm_bindgen_test]
fn cross_process_interleave_is_slice_order() {
    let mut vm = WasmVm::new().expect("VM constructs");
    let bank: SinkBank = Rc::new(RefCell::new(Vec::new()));
    let sink = capture_closure(&bank);
    vm.register_io_sink(sink.as_ref().unchecked_ref::<Function>().clone());
    let module_a = print_lines_module(&vm, "wport7_interleave_a", &["a1\n", "a2\n"]);
    let module_b = print_lines_module(&vm, "wport7_interleave_b", &["b1\n", "b2\n"]);
    let _pid_a = spawn_run(&vm, module_a);
    let _pid_b = spawn_run(&vm, module_b);
    let summary = vm.run_step().expect("one drain runs both processes");
    assert!(summary.as_string().expect("summary JSON").contains("idle"));
    let texts: Vec<String> = bank.borrow().iter().map(|(_, text)| text.clone()).collect();
    assert_eq!(
        texts,
        vec!["a1\n", "a2\n", "b1\n", "b2\n"],
        "slice order preserved faithfully across processes"
    );
    drop(sink);
}

/// Wall 8 (R4, D9b): multi-turn FairnessYield ordering — writes stream per
/// turn and the order is TOTAL across turns; the fairness re-queue from the
/// held Draining state (the existing `queue_turn` edge) still works.
#[wasm_bindgen_test]
async fn multi_turn_fairness_output_order_is_total_across_turns() {
    let mut vm = WasmVm::new().expect("VM constructs");
    let bank: SinkBank = Rc::new(RefCell::new(Vec::new()));
    let sink = capture_closure(&bank);
    vm.register_io_sink(sink.as_ref().unchecked_ref::<Function>().clone());
    let module_a = print_lines_module(&vm, "wport7_fair_a", &["a1\n", "a2\n"]);
    let module_b = print_lines_module(&vm, "wport7_fair_b", &["b1\n", "b2\n"]);
    let _pid_a = spawn_run(&vm, module_a);
    let filler = Function::new_no_args("return null;");
    for _ in 0..1023 {
        let _actor = vm.spawn_actor(filler.clone());
    }
    let _pid_b = spawn_run(&vm, module_b);
    host_microtask().await;

    let mid_summary = vm.arbiter.last_summary.borrow().clone();
    assert_eq!(
        mid_summary["state"], "fairness_yield",
        "turn 1 exhausted the slice budget with the second printer remaining"
    );
    let after_turn_one: Vec<String> = bank.borrow().iter().map(|(_, t)| t.clone()).collect();
    assert_eq!(
        after_turn_one,
        vec!["a1\n", "a2\n"],
        "turn 1's writes flushed at turn 1's tail — streaming per turn"
    );

    host_macrotask().await;
    let final_texts: Vec<String> = bank.borrow().iter().map(|(_, t)| t.clone()).collect();
    assert_eq!(
        final_texts,
        vec!["a1\n", "a2\n", "b1\n", "b2\n"],
        "total order across turns; the fairness re-queue from Draining ran"
    );
    assert_eq!(vm.arbiter.last_summary.borrow()["state"], "idle");
    drop(sink);
}

/// Wall 11 (R4, D11): the `io:format/2` facility-absent badarg is the pinned
/// CONTRACT — only the threaded `supervision_integration` installs an
/// `io_message_facility`; the cooperative path refuses with a CATCHABLE
/// badarg (the sealed profile's classification) rather than wiring a naive
/// facility (H3 FORBIDS it; the IO-server brief is recorded future work).
/// Pinned exactly as the WPORT-5 zeros wall pins its refusals: a real
/// bytecode `catch` whose caught value reaches JS.
#[wasm_bindgen_test]
async fn io_format_2_facility_absent_badarg_is_pinned() {
    let mut vm = WasmVm::new().expect("VM constructs");
    let literals = vec![Literal::Binary(b"hi~n".to_vec())];
    let imports = vec![registered_bif_import(&vm, "io", "format", 2)];
    let code = vec![
        Instruction::Label { label: 1 },
        Instruction::Allocate {
            stack_need: Operand::Unsigned(1),
            live: Operand::Unsigned(0),
        },
        Instruction::Catch {
            destination: Operand::Y(0),
            label: Operand::Label(2),
        },
        Instruction::Move {
            source: Operand::Literal(0),
            destination: Operand::X(0),
        },
        Instruction::Move {
            source: Operand::Atom(None),
            destination: Operand::X(1),
        },
        Instruction::CallExt {
            arity: Operand::Unsigned(2),
            import: Operand::Unsigned(0),
        },
        Instruction::Label { label: 2 },
        Instruction::CatchEnd {
            source: Operand::Y(0),
        },
        Instruction::Deallocate {
            words: Operand::Unsigned(1),
        },
        Instruction::Return,
    ];
    let module = build_module_with_literals(
        &vm.atom_table,
        "wport7_format2",
        &[("run", 0, 1)],
        code,
        imports,
        literals,
    );
    let pid = spawn_run(&vm, module);
    let summary = parse_json(vm.run_step().expect("the drain itself succeeds"));
    assert_eq!(
        summary["exited"],
        json!([pid]),
        "the refusal is CATCHABLE — the process caught it and exited normally"
    );
    let completion = await_exit_json(&mut vm, pid).await;
    assert_eq!(completion["state"], "exited");
    let caught = serde_json::to_string(&completion["result"]).expect("caught value serializes");
    assert!(
        caught.contains("badarg"),
        "the facility-absent refusal carries badarg: {caught}"
    );
    assert!(
        caught.contains("EXIT"),
        "the caught value is a bytecode catch tuple: {caught}"
    );
}
