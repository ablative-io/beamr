//! WPORT-7 R1/R2 wall battery: the typed `SchedulerFailureError` surface
//! (all five legs), the D1 manual-drain wedge fix (OQ-C ruled LATCH), the two
//! observability surfaces, and the reporting-only panic hook.
//!
//! Drain-failure walls inject throwing host-primitive doubles installed
//! BEFORE construction (`HostPrimitives::probe` captures globals at
//! construction; the doubles delegate to the real primitive until toggled) —
//! zero production seams. They exercise the deadline-reconcile path ONLY:
//! send_message's stranded-edge doctrine (WPORT-4 tear Ruling 6,
//! `lib.rs` Ruling 6 comment) is exercised-adjacent here, NOT reopened;
//! transient-throw evidence, if ever observed, reopens per the ruling's own
//! clause as its own finding.

use std::cell::RefCell;
use std::rc::Rc;

use beamr::ets::OwnedTerm;
use beamr::loader::Instruction;
use beamr::loader::decode::compact::Operand;
use beamr::native::Capability;
use beamr::term::Term;
use js_sys::{Function, Reflect};
use serde_json::{Value, json};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test;

use super::*;
use crate::tests::{
    build_module, host_macrotask, host_microtask, promise_returning_nif, receive_after_module,
    registered_bif_import, spawn_bytecode,
};
use crate::{ArbiterState, WasmVm};

fn terminal_error_json(vm: &WasmVm) -> Value {
    serde_json::from_str(
        &vm.terminal_error()
            .as_string()
            .expect("terminal_error returns a JSON string post-failure"),
    )
    .expect("terminal_error JSON parses")
}

/// Banked `(name, data)` pairs from failure-callback invocations.
type FailureCaptureBank = Rc<RefCell<Vec<Value>>>;
/// The live JS closure backing a registered capture callback.
type FailureCaptureGuard = Closure<dyn FnMut(JsValue)>;

/// Capture surface for `register_failure_callback`: banks each invocation's
/// (name, data) pair.
fn capture_failure_callback(vm: &mut WasmVm) -> (FailureCaptureBank, FailureCaptureGuard) {
    let captured: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(Vec::new()));
    let bank = Rc::clone(&captured);
    let callback = Closure::<dyn FnMut(JsValue)>::new(move |error: JsValue| {
        bank.borrow_mut()
            .push(json!({"name": error_name(&error), "data": error_data(&error)}));
    });
    vm.register_failure_callback(callback.as_ref().unchecked_ref::<Function>().clone());
    (captured, callback)
}

fn spawn_parked_receive_after(vm: &WasmVm, name: &str, milliseconds: u64) -> u64 {
    let (module, function, definition) = receive_after_module(&vm.atom_table, name, milliseconds);
    vm.module_registry.insert(definition);
    vm.scheduler
        .borrow_mut()
        .spawn_owned(module, function, Vec::new())
        .expect("bytecode process spawns")
}

/// `run/0`: park in a PLAIN receive (no timer); on the first message,
/// re-enter a receive with a fresh 60s timeout. The re-arm forces the NEXT
/// drain's completion-seam reconcile to call the host `setTimeout` — the
/// injection point for the queued-leg drain-failure wall (the send-time
/// reconcile touches nothing, keeping Ruling 6's send path exercised-adjacent
/// only).
fn recv_then_rearm_module(
    atoms: &beamr::atom::AtomTable,
    name: &str,
) -> (beamr::atom::Atom, beamr::atom::Atom, beamr::module::Module) {
    let timed_out = atoms.intern("timed_out");
    let code = vec![
        Instruction::Label { label: 1 },
        // First receive: plain wait, no deadline.
        Instruction::Label { label: 10 },
        Instruction::LoopRec {
            fail: Operand::Label(20),
            destination: Operand::X(0),
        },
        Instruction::RemoveMessage,
        // Second receive: 60s timeout — a NEW deadline minted mid-drain.
        Instruction::Label { label: 11 },
        Instruction::LoopRec {
            fail: Operand::Label(21),
            destination: Operand::X(0),
        },
        Instruction::RemoveMessage,
        Instruction::Return,
        Instruction::Label { label: 21 },
        Instruction::WaitTimeout {
            fail: Operand::Label(11),
            timeout: Operand::Unsigned(60_000),
        },
        Instruction::Timeout,
        Instruction::Move {
            source: Operand::Atom(Some(timed_out)),
            destination: Operand::X(0),
        },
        Instruction::Return,
        Instruction::Label { label: 20 },
        Instruction::Wait {
            fail: Operand::Label(10),
        },
    ];
    let module = build_module(atoms, name, &[("run", 0, 1)], code, Vec::new());
    (module.name, atoms.intern("run"), module)
}

/// Wall 1 (R4): the drain-failure QUEUED leg. A queued turn whose completed-
/// drain deadline reconcile throws (injected `setTimeout` double at the
/// mid-drain re-arm) rejects parked waiters with the typed class, latches
/// it, fires the one-shot callback, and leaves counters coherent — the exact
/// symmetric twin of the manual-leg wall below.
#[wasm_bindgen_test]
async fn queued_drain_failure_rejects_await_exit_with_typed_scheduler_failure() {
    let set_double = install_throwing_double("setTimeout");
    let mut vm = WasmVm::new().expect("VM constructs");
    let (module, function, definition) =
        recv_then_rearm_module(&vm.atom_table, "wport7_queued_leg");
    vm.module_registry.insert(definition);
    let pid = vm
        .scheduler
        .borrow_mut()
        .spawn_owned(module, function, Vec::new())
        .expect("bytecode process spawns");
    let first = vm.run_step().expect("parking drain succeeds");
    assert!(first.as_string().expect("summary JSON").contains("idle"));
    let (captured, callback) = capture_failure_callback(&mut vm);
    let before = vm.arbiter_counters();

    set_double.set_throwing(true);
    vm.send_message(pid, JsValue::from_f64(1.0))
        .expect("send delivers; the wake rides the real microtask primitive");
    // Park the waiter while the turn is QUEUED (post-park the VM is settled
    // idle, so an earlier await would have resolved "idle" immediately).
    let waiter = vm.await_exit(pid);
    host_microtask().await;
    set_double.set_throwing(false);

    let rejection = JsFuture::from(waiter)
        .await
        .expect_err("parked waiter rejects at the latch");
    assert_typed(&rejection, "queued", "reconcile");
    assert_eq!(captured.borrow().len(), 1, "one-shot callback fired once");
    assert_eq!(captured.borrow()[0]["data"]["leg"], json!("queued"));
    assert_eq!(terminal_error_json(&vm)["leg"], json!("queued"));
    let after = vm.arbiter_counters();
    assert_eq!(after.arbiter.queued_now, 0, "queued_now is not stuck");
    assert_eq!(after.arbiter.executions, before.arbiter.executions + 1);
    assert_eq!(vm.arbiter.state.get(), ArbiterState::Idle);
    let late = JsFuture::from(vm.await_exit(pid))
        .await
        .expect_err("post-latch await_exit rejects immediately");
    assert_typed(&late, "queued", "reconcile");
    drop(callback);
    drop(vm);
    set_double.restore();
}

/// Wall 2 (R4): the drain-failure MANUAL leg — the D1 wedge fix, OQ-C ruled
/// LATCH. A failed `run_step` returns the typed class to the caller AND
/// resets to Idle AND latches (waiters reject with the same value): full
/// symmetric routing with wall 1. Pre-fix this walked the silent wedge —
/// state stuck Draining, `last_error` unset, every later `run_step` minting
/// "arbiter is already draining".
#[wasm_bindgen_test]
async fn manual_drain_failure_returns_typed_error_resets_state_and_latches() {
    let set_double = install_throwing_double("setTimeout");
    let clear_double = install_throwing_double("clearTimeout");
    let mut vm = WasmVm::new().expect("VM constructs");
    let pid = spawn_parked_receive_after(&vm, "wport7_manual_leg", 60_000);
    let (captured, callback) = capture_failure_callback(&mut vm);
    let waiter = vm.await_exit(pid);

    set_double.set_throwing(true);
    let error = vm
        .run_step()
        .expect_err("manual drain fails at the reconcile seam");
    set_double.set_throwing(false);

    assert_typed(&error, "manual", "reconcile");
    assert_eq!(vm.arbiter.state.get(), ArbiterState::Idle, "state reset");
    let rejection = JsFuture::from(waiter)
        .await
        .expect_err("parked waiter rejects per the OQ-C LATCH ruling");
    assert_typed(&rejection, "manual", "reconcile");
    assert_eq!(captured.borrow().len(), 1);
    assert_eq!(terminal_error_json(&vm)["leg"], json!("manual"));
    let recovered = vm
        .run_step()
        .expect("post-failure manual drain is NOT refused as already-draining");
    assert!(
        recovered
            .as_string()
            .expect("summary JSON")
            .contains("idle"),
        "the arbiter is unwedged: a later drain completes"
    );
    let counters = vm.arbiter_counters();
    assert_eq!(counters.arbiter.queued_now, 0, "queued_now is not stuck");
    drop(callback);
    drop(vm);
    set_double.restore();
    clear_double.restore();
}

/// Wall 3 (R4): `terminal_error()` — null pre-failure, the data JSON string
/// post-failure, non-consuming and repeatable.
#[wasm_bindgen_test]
fn terminal_error_returns_null_then_repeatable_json_after_failure() {
    let set_double = install_throwing_double("setTimeout");
    let mut vm = WasmVm::new().expect("VM constructs");
    assert!(vm.terminal_error().is_null(), "null before any failure");
    let _pid = spawn_parked_receive_after(&vm, "wport7_terminal_error", 60_000);
    set_double.set_throwing(true);
    let _error = vm.run_step().expect_err("manual drain fails");
    set_double.set_throwing(false);
    let first = vm.terminal_error().as_string().expect("JSON string");
    let second = vm.terminal_error().as_string().expect("JSON string");
    assert_eq!(first, second, "non-consuming: repeat reads answer alike");
    let data: Value = serde_json::from_str(&first).expect("data parses");
    assert_eq!(data["leg"], json!("manual"));
    assert_eq!(data["phase"], json!("reconcile"));
    assert_eq!(data["terminal"], json!(true));
    drop(vm);
    set_double.restore();
}

/// Wall 4 (R4): `register_failure_callback` is one-shot push — exactly one
/// invocation at the first `fail()`, none at later failures.
#[wasm_bindgen_test]
fn failure_callback_fires_exactly_once_at_first_failure() {
    let set_double = install_throwing_double("setTimeout");
    let mut vm = WasmVm::new().expect("VM constructs");
    let _pid = spawn_parked_receive_after(&vm, "wport7_one_shot", 60_000);
    let (captured, callback) = capture_failure_callback(&mut vm);
    set_double.set_throwing(true);
    let first = vm.run_step().expect_err("first manual failure");
    let second = vm
        .run_step()
        .expect_err("second failure re-attempts the arm and fails again");
    set_throwing_off_and_assert(&set_double, &first, &second);
    assert_eq!(
        captured.borrow().len(),
        1,
        "the push surface fired exactly once, at the first latch"
    );
    drop(callback);
    drop(vm);
    set_double.restore();
}

fn set_throwing_off_and_assert(double: &PrimitiveDouble, first: &JsValue, second: &JsValue) {
    double.set_throwing(false);
    assert_typed(first, "manual", "reconcile");
    assert_typed(second, "manual", "reconcile");
}

/// Wall 5 (R4): the leg-slug set is closed and exact, and every leg mints the
/// full typed shape with its own slug in both the message kind position and
/// the data `leg` position.
#[wasm_bindgen_test]
fn failure_leg_slug_set_is_closed_and_exact() {
    assert_eq!(
        LEG_SLUGS,
        ["queued", "manual", "deadline", "promise", "spawn_edge"]
    );
    let legs = [
        FailureLeg::Queued,
        FailureLeg::Manual,
        FailureLeg::Deadline,
        FailureLeg::Promise,
        FailureLeg::SpawnEdge,
    ];
    let minted: Vec<String> = legs
        .iter()
        .map(|leg| {
            let error = scheduler_failure_error(*leg, PHASE_RECONCILE, &JsValue::from_str("probe"));
            assert_typed(&error, leg.slug(), PHASE_RECONCILE);
            leg.slug().to_owned()
        })
        .collect();
    assert_eq!(minted, LEG_SLUGS, "variant order IS the pinned slug order");
}

/// R1 acceptance: the spawn-edge leg — `spawn_actor`'s wake failure has no
/// per-call surface and is swallowed into the latch, observable through both
/// surfaces with the `spawn_edge` slug.
#[wasm_bindgen_test]
async fn spawn_edge_wake_failure_swallows_into_latch_with_spawn_edge_leg() {
    let queue_double = install_targeted_queue_microtask_double();
    let mut vm = WasmVm::new().expect("VM constructs");
    target_arbiter_callback(&vm);
    let (captured, callback) = capture_failure_callback(&mut vm);
    queue_double.set_throwing(true);
    let pid = vm.spawn_actor(Function::new_no_args("return null;"));
    queue_double.set_throwing(false);
    assert!(
        pid > 0,
        "spawn_actor still returns the pid: error swallowed"
    );
    assert_eq!(terminal_error_json(&vm)["leg"], json!("spawn_edge"));
    assert_eq!(
        terminal_error_json(&vm)["phase"],
        json!("queue_microtask"),
        "phase names the failing turn-queue primitive"
    );
    assert_eq!(
        captured.borrow().len(),
        1,
        "push surface saw the latch-only leg"
    );
    let rejection = JsFuture::from(vm.await_exit(pid))
        .await
        .expect_err("await_exit after the latch rejects with the typed value");
    assert_typed(&rejection, "spawn_edge", "queue_microtask");
    drop(callback);
    drop(vm);
    queue_double.restore();
}

/// R1 acceptance: the deadline latch-only leg (`deadline_callback_fired`) —
/// a wake failure inside the unified late-fire callback has no JS caller and
/// is observable through both surfaces, driven by the deterministic fire seam.
#[wasm_bindgen_test]
async fn deadline_late_fire_wake_failure_reaches_both_surfaces() {
    let queue_double = install_targeted_queue_microtask_double();
    let mut vm = WasmVm::new().expect("VM constructs");
    target_arbiter_callback(&vm);
    let pid = spawn_parked_receive_after(&vm, "wport7_deadline_leg", 30);
    let parked = vm.run_step().expect("parking drain arms the deadline");
    assert!(parked.as_string().expect("summary JSON").contains("idle"));
    let (captured, callback) = capture_failure_callback(&mut vm);

    queue_double.set_throwing(true);
    vm.fire_unified_deadline_at(web_time::Instant::now() + std::time::Duration::from_millis(50));
    queue_double.set_throwing(false);

    assert_eq!(terminal_error_json(&vm)["leg"], json!("deadline"));
    assert_eq!(terminal_error_json(&vm)["phase"], json!("queue_microtask"));
    assert_eq!(
        captured.borrow().len(),
        1,
        "push surface saw the latch-only leg"
    );
    assert_eq!(captured.borrow()[0]["name"], json!("SchedulerFailureError"));
    let rejection = JsFuture::from(vm.await_exit(pid))
        .await
        .expect_err("await_exit rejects with the latched typed value");
    assert_typed(&rejection, "deadline", "queue_microtask");
    drop(callback);
    drop(vm);
    queue_double.restore();
}

/// R1 acceptance: the promise latch-only leg (async completion) — a wake
/// failure after `complete_async` has no JS caller and is observable through
/// both surfaces.
#[wasm_bindgen_test]
async fn promise_completion_wake_failure_reaches_both_surfaces() {
    let queue_double = install_targeted_queue_microtask_double();
    let mut vm = WasmVm::new().expect("VM constructs");
    target_arbiter_callback(&vm);
    let resolvers: Rc<RefCell<Vec<(Function, Function)>>> = Rc::new(RefCell::new(Vec::new()));
    vm.register_async_nif(
        "wport4_async",
        "fetch",
        1,
        promise_returning_nif(&resolvers),
    )
    .expect("async NIF registers");
    let module = crate::tests::async_caller_module(&vm, "wport7_promise_leg");
    let module_name = module.name;
    vm.module_registry.insert(module);
    let run = vm.atom_table.intern("run");
    let pid = spawn_bytecode(
        &mut vm,
        module_name,
        run,
        vec![OwnedTerm::immediate(Term::small_int(1))],
    );
    host_microtask().await;
    assert_eq!(resolvers.borrow().len(), 1, "the NIF banked its promise");
    let (captured, callback) = capture_failure_callback(&mut vm);

    queue_double.set_throwing(true);
    let (resolve, _reject) = resolvers.borrow()[0].clone();
    resolve
        .call1(&JsValue::UNDEFINED, &JsValue::from_f64(5.0))
        .expect("banked promise resolves");
    host_macrotask().await;
    queue_double.set_throwing(false);

    assert_eq!(terminal_error_json(&vm)["leg"], json!("promise"));
    assert_eq!(terminal_error_json(&vm)["phase"], json!("queue_microtask"));
    assert_eq!(
        captured.borrow().len(),
        1,
        "push surface saw the latch-only leg"
    );
    let rejection = JsFuture::from(vm.await_exit(pid))
        .await
        .expect_err("await_exit rejects with the latched typed value");
    assert_typed(&rejection, "promise", "queue_microtask");
    drop(callback);
    drop(vm);
    queue_double.restore();
}

/// R2 acceptance: the panic hook installs exactly once per process across any
/// number of constructions (Once-guarded); construction multiplicity does not
/// double-install.
#[wasm_bindgen_test]
fn panic_hook_installs_exactly_once_across_vm_constructions() {
    let _first = WasmVm::new().expect("VM constructs");
    let _second = WasmVm::new().expect("VM constructs");
    let _third = WasmVm::new().expect("VM constructs");
    assert_eq!(
        panic_hook_install_count(),
        1,
        "one process-global install, ever"
    );
}

/// The cfg(test) panicking BIF (D8): a plain fn pointer through the ordinary
/// registration seam — zero production change.
fn panicking_test_bif(
    _args: &[Term],
    _context: &mut beamr::native::ProcessContext<'_>,
) -> Result<Term, Term> {
    panic!("wport7 intentional panic wall probe");
}

/// Wall 6 (R4, D8): the panic wall — sync-entry trap caught as a JS
/// exception via a JS trampoline; the registered panic callback receives
/// message + location BEFORE the trap and `console.error` fires regardless.
///
/// One panic per test; instance-terminal; deliberately the LAST act in this
/// test's lifecycle (post-panic the VM is bricked per the recovery contract
/// and the process's panic bookkeeping stays elevated — later diagnostics in
/// this binary would degrade, so nothing after this test may panic again by
/// design of the suite being green). The arbiter-callback ASYNC-entry abort
/// case is probe territory (`WPORT-7-PROBE-FAILURE.md`), not CI.
#[wasm_bindgen_test]
fn panic_reaches_console_and_registered_callback_before_the_trap() {
    let mut vm = WasmVm::new().expect("VM constructs");
    let module_atom = vm.atom_table.intern("wport7_panic");
    let function_atom = vm.atom_table.intern("boom");
    vm.bif_registry
        .register(
            module_atom,
            function_atom,
            0,
            panicking_test_bif,
            Capability::ProcessLocal,
        )
        .expect("cfg(test) panicking BIF registers through the plain seam");
    let imports = vec![registered_bif_import(&vm, "wport7_panic", "boom", 0)];
    let code = vec![
        Instruction::Label { label: 1 },
        Instruction::CallExt {
            arity: Operand::Unsigned(0),
            import: Operand::Unsigned(0),
        },
        Instruction::Return,
    ];
    let module = build_module(
        &vm.atom_table,
        "wport7_panic_wall",
        &[("run", 0, 1)],
        code,
        imports,
    );
    let module_name = module.name;
    vm.module_registry.insert(module);
    let run = vm.atom_table.intern("run");
    let _pid = vm
        .scheduler
        .borrow_mut()
        .spawn_owned(module_name, run, Vec::new())
        .expect("panicking process spawns");

    let global = js_sys::global();
    register_panic_callback(Function::new_with_args(
        "payload",
        "globalThis.__wport7_panic_payload = payload;",
    ));
    let console = Reflect::get(&global, &JsValue::from_str("console")).expect("console exists");
    let original_error =
        Reflect::get(&console, &JsValue::from_str("error")).expect("console.error exists");
    Reflect::set(
        &console,
        &JsValue::from_str("error"),
        Function::new_with_args("text", "globalThis.__wport7_panic_console = String(text);")
            .as_ref(),
    )
    .expect("console.error spy installs");

    let trampoline =
        Function::new_with_args("f", "try { f(); return null; } catch (e) { return e; }");
    let entry = Closure::once(move || {
        let _ignored = vm.run_step();
    });
    let caught = trampoline
        .call1(
            &JsValue::UNDEFINED,
            entry.as_ref().unchecked_ref::<Function>(),
        )
        .expect("trampoline returns normally");
    let _restore = Reflect::set(&console, &JsValue::from_str("error"), &original_error);

    assert!(
        !caught.is_null(),
        "the sync entry trapped as a JS exception"
    );
    assert_eq!(error_name(&caught), "RuntimeError", "the wasm trap class");
    let payload = Reflect::get(&global, &JsValue::from_str("__wport7_panic_payload"))
        .expect("payload slot reads")
        .as_string()
        .expect("the registered callback received the payload BEFORE the trap");
    assert!(
        payload.contains("wport7 intentional panic wall probe"),
        "payload carries the message: {payload}"
    );
    assert!(
        payload.contains("failure_tests.rs"),
        "payload carries the location: {payload}"
    );
    let console_line = Reflect::get(&global, &JsValue::from_str("__wport7_panic_console"))
        .expect("console slot reads")
        .as_string()
        .expect("console.error fired regardless of registration");
    assert!(console_line.contains("wport7 intentional panic wall probe"));
}
