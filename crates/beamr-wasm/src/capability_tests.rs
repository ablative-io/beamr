//! WPORT-8 wall battery — capability adapters (fetch + KV).
//!
//! Commit-1 scope: the synchronous surface — closed vocabulary (R4), the
//! `CapabilityError` minter mold, refuse-before-suspend with zero counter
//! motion (R5), refusal-precedes-arg-validation ordering, module-scoped
//! injection (R1). Async completion walls (R2/R3/R6) ride later commits.

use beamr::loader::Instruction;
use beamr::loader::decode::compact::Operand;
use js_sys::{Object, Reflect};
use serde_json::json;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::wasm_bindgen_test;

use super::*;
use crate::WasmVm;
use crate::tests::{await_exit_json, build_module, registered_bif_import};

/// A one-function caller module: `call/Arity` forwards its spawn arguments
/// straight into the imported capability MFA and exits with the returned
/// value (or the async-resumed x0).
fn capability_call_module(
    vm: &WasmVm,
    name: &str,
    target_module: &str,
    target_function: &str,
    arity: u8,
) -> String {
    let import = registered_bif_import(vm, target_module, target_function, arity);
    let code = vec![
        Instruction::Label { label: 1 },
        Instruction::CallExt {
            arity: Operand::Unsigned(u64::from(arity)),
            import: Operand::Unsigned(0),
        },
        Instruction::Return,
    ];
    let module = build_module(
        &vm.atom_table,
        name,
        &[("call", arity, 1)],
        code,
        vec![import],
    );
    vm.module_registry.insert(module);
    name.to_owned()
}

/// R4 (Rust side): the kind set is closed and exact, and variant order IS
/// slug order.
#[wasm_bindgen_test]
fn capability_kind_slug_set_is_closed_and_exact() {
    assert_eq!(
        CAPABILITY_KIND_SLUGS,
        [
            "capability_missing",
            "refused",
            "malformed_response",
            "rejected",
            "cancelled",
        ]
    );
    let kinds = [
        CapabilityKind::CapabilityMissing,
        CapabilityKind::Refused,
        CapabilityKind::MalformedResponse,
        CapabilityKind::Rejected,
        CapabilityKind::Cancelled,
    ];
    for (index, kind) in kinds.into_iter().enumerate() {
        assert_eq!(kind.slug(), CAPABILITY_KIND_SLUGS[index]);
    }
}

/// R4: the minter follows the `ArtifactLoadError` mold exactly — named
/// error, `"{kind}: {detail}"` message, ONE `data` property holding a JSON
/// string with the adapter named inside the payload.
#[wasm_bindgen_test]
fn capability_error_minter_follows_the_artifact_load_error_mold() {
    let error = capability_error(
        CapabilityAdapter::Fetch,
        CapabilityKind::MalformedResponse,
        "status is not a number",
    );
    let error: &js_sys::Error = error.dyn_ref().expect("minter returns an Error");
    assert_eq!(String::from(error.name()), "CapabilityError");
    assert_eq!(
        String::from(error.message()),
        "malformed_response: status is not a number"
    );
    let data = Reflect::get(error.as_ref(), &JsValue::from_str("data"))
        .expect("data property present")
        .as_string()
        .expect("data is a JSON string");
    let data: serde_json::Value = serde_json::from_str(&data).expect("data parses");
    assert_eq!(
        data,
        json!({
            "adapter": "fetch",
            "kind": "malformed_response",
            "detail": "status is not a number",
        })
    );
}

/// R5 + A2: an uninjected module refuses `{error, {capability_missing,
/// CapabilityAtom}}` synchronously — the caller exits with the refusal as a
/// VALUE in its very first slice, and nothing moves: no deadline arm, no
/// capability counter, no in-flight entry.
#[wasm_bindgen_test]
async fn uninjected_modules_refuse_typed_with_zero_counter_motion() {
    let mut vm = WasmVm::new().expect("VM constructs");
    capability_call_module(&vm, "wport8_refuse_fetch", "wasm_fetch", "request", 1);
    capability_call_module(&vm, "wport8_refuse_kv", "wasm_kv", "get", 1);
    let deadline_before = vm.unified_deadline_snapshot();

    let fetch_pid = vm
        .spawn(
            "wport8_refuse_fetch",
            "call",
            r#"[{"url":"https://example.test/"}]"#,
        )
        .expect("fetch caller spawns");
    let fetch_exit = await_exit_json(&mut vm, fetch_pid).await;
    assert_eq!(fetch_exit["state"], "exited");
    assert_eq!(
        fetch_exit["result"],
        json!(["error", ["capability_missing", "fetch"]])
    );

    let kv_pid = vm
        .spawn("wport8_refuse_kv", "call", r#"["some_key"]"#)
        .expect("kv caller spawns");
    let kv_exit = await_exit_json(&mut vm, kv_pid).await;
    assert_eq!(kv_exit["state"], "exited");
    assert_eq!(
        kv_exit["result"],
        json!(["error", ["capability_missing", "kv"]])
    );

    assert_eq!(
        vm.unified_deadline_snapshot(),
        deadline_before,
        "a refusal arms nothing and cancels nothing"
    );
    let counters = vm.capability_counters();
    assert_eq!(counters.dead_pid_completions, 0);
}

/// R5: the capability check precedes EVERY other effect — malformed args on
/// an UNINJECTED module still get the typed refusal; the same malformed args
/// on an injected module are badarg-class (the caller ERRORS), keeping the
/// refusal and arg-shape vocabularies distinct.
#[wasm_bindgen_test]
async fn refusal_precedes_arg_validation_and_badarg_stays_distinct() {
    let mut vm = WasmVm::new().expect("VM constructs");
    capability_call_module(&vm, "wport8_order_put", "wasm_kv", "put", 2);

    // Uninjected: integer key, still the typed refusal.
    let refused_pid = vm
        .spawn("wport8_order_put", "call", r#"[123, "value"]"#)
        .expect("uninjected put caller spawns");
    let refused = await_exit_json(&mut vm, refused_pid).await;
    assert_eq!(refused["state"], "exited");
    assert_eq!(
        refused["result"],
        json!(["error", ["capability_missing", "kv"]])
    );

    // Injected: the same integer key is an arg-shape error — badarg, caller
    // errors; NOT a capability slug.
    vm.register_kv_capability(Object::new());
    let badarg_pid = vm
        .spawn("wport8_order_put", "call", r#"[123, "value"]"#)
        .expect("injected put caller spawns");
    let errored = await_exit_json(&mut vm, badarg_pid).await;
    // Uncaught native-BIF raises classify as "exited" with x0 preserved —
    // the WPORT-7 board finding (2026-07-18, tear-graded); the exited/errored
    // vocabulary belongs to a future brief and is NOT asserted as such here.
    // The load-bearing distinction: a badarg outcome preserves x0 and is
    // NEVER a capability slug tuple.
    assert_eq!(errored["state"], "exited");
    assert_eq!(
        errored["result"], 123,
        "badarg preserves x0; no capability slug tuple is minted"
    );
    let counters = vm.capability_counters();
    assert_eq!(counters.dead_pid_completions, 0);
}

/// R1: injection is module-scoped and last-wins — a VM with only the fetch
/// capability has a LIVE `wasm_fetch` module (a method-less capability
/// object produces the operational `refused` leg, not `capability_missing`)
/// while the whole `wasm_kv` module still refuses `capability_missing`.
#[wasm_bindgen_test]
async fn capability_injection_is_module_scoped_last_wins() {
    let mut vm = WasmVm::new().expect("VM constructs");
    capability_call_module(&vm, "wport8_scope_fetch", "wasm_fetch", "request", 1);
    capability_call_module(&vm, "wport8_scope_kv", "wasm_kv", "get", 1);

    // A capability object with NO request method: the module is live, the
    // host is refusing — the operational leg, with a DetailBinary (A2).
    vm.register_fetch_capability(Object::new());
    let fetch_pid = vm
        .spawn(
            "wport8_scope_fetch",
            "call",
            r#"[{"url":"https://example.test/"}]"#,
        )
        .expect("fetch caller spawns");
    let fetch_exit = await_exit_json(&mut vm, fetch_pid).await;
    assert_eq!(fetch_exit["state"], "exited");
    assert_eq!(
        fetch_exit["result"],
        json!(["error", ["refused", "capability method missing"]])
    );

    let kv_pid = vm
        .spawn("wport8_scope_kv", "call", r#"["k"]"#)
        .expect("kv caller spawns");
    let kv_exit = await_exit_json(&mut vm, kv_pid).await;
    assert_eq!(
        kv_exit["result"],
        json!(["error", ["capability_missing", "kv"]])
    );

    // Last-wins: replacing the fetch capability with one whose request
    // method throws synchronously flips the detail — same slug family, new
    // object serving.
    let throwing = Object::new();
    let thrower = js_sys::Function::new_no_args("throw new Error('host said no')");
    Reflect::set(
        throwing.as_ref(),
        &JsValue::from_str("request"),
        thrower.as_ref(),
    )
    .expect("request method installs");
    vm.register_fetch_capability(throwing);
    let again_pid = vm
        .spawn(
            "wport8_scope_fetch",
            "call",
            r#"[{"url":"https://example.test/"}]"#,
        )
        .expect("fetch caller respawns");
    let again = await_exit_json(&mut vm, again_pid).await;
    assert_eq!(
        again["result"],
        json!(["error", ["refused", "host said no"]])
    );
}

// ---------------------------------------------------------------------------
// Async completion walls (R2/R3, ruling walls W1/W2/W3).
// ---------------------------------------------------------------------------

use std::rc::Rc;
use std::sync::{Arc as StdArc, Mutex};

use beamr::native::native_process::{NativeContext, NativeHandler, NativeOutcome};
use beamr::process::ExitReason;
use beamr::term::json::term_to_value;
use js_sys::{Function, Promise};
use wasm_bindgen::closure::Closure;

use crate::tests::{assert_true_idle, host_macrotask, host_microtask, spawn_native_root_edge};

/// A fetch capability whose `request` defers settlement: each call stores
/// its `(resolve, reject)` pair and, when the host attaches one, records the
/// abort slot for the test to inspect.
fn deferred_fetch_capability(
    resolvers: &Rc<RefCell<Vec<(Function, Function)>>>,
    abort_calls: &Rc<Cell<u32>>,
) -> Object {
    let capability = Object::new();
    let resolvers = Rc::clone(resolvers);
    let abort_calls = Rc::clone(abort_calls);
    let request = Closure::<dyn FnMut(JsValue, JsValue) -> JsValue>::new(
        move |_request: JsValue, slot: JsValue| {
            let abort_calls = Rc::clone(&abort_calls);
            let recorder = Closure::<dyn FnMut()>::new(move || {
                abort_calls.set(abort_calls.get() + 1);
            })
            .into_js_value();
            Reflect::set(&slot, &JsValue::from_str("abort"), &recorder)
                .expect("abort hook attaches to the slot");
            let resolvers = Rc::clone(&resolvers);
            Promise::new(&mut move |resolve, reject| {
                resolvers.borrow_mut().push((resolve, reject));
            })
            .into()
        },
    )
    .into_js_value()
    .unchecked_into::<Function>();
    Reflect::set(
        capability.as_ref(),
        &JsValue::from_str("request"),
        request.as_ref(),
    )
    .expect("request method installs");
    capability
}

/// An in-memory KV capability over a Rust BTreeMap — get/put/delete/
/// list_by_prefix, every method returning an already-settled thenable.
/// BTreeMap iteration order IS the contract's lexicographic listing order.
fn in_memory_kv_capability() -> Object {
    let store = Rc::new(RefCell::new(
        std::collections::BTreeMap::<String, String>::new(),
    ));
    let capability = Object::new();
    let install = |name: &str, function: Function| {
        Reflect::set(
            capability.as_ref(),
            &JsValue::from_str(name),
            function.as_ref(),
        )
        .expect("kv method installs");
    };
    {
        let store = Rc::clone(&store);
        install(
            "get",
            Closure::<dyn FnMut(JsValue) -> JsValue>::new(move |key: JsValue| {
                let key = key.as_string().expect("kv key is a string");
                match store.borrow().get(&key) {
                    Some(value) => Promise::resolve(&JsValue::from_str(value)).into(),
                    None => Promise::resolve(&JsValue::UNDEFINED).into(),
                }
            })
            .into_js_value()
            .unchecked_into(),
        );
    }
    {
        let store = Rc::clone(&store);
        install(
            "put",
            Closure::<dyn FnMut(JsValue, JsValue) -> JsValue>::new(
                move |key: JsValue, value: JsValue| {
                    let key = key.as_string().expect("kv key is a string");
                    let value = value.as_string().expect("kv value arrives as a string");
                    store.borrow_mut().insert(key, value);
                    Promise::resolve(&JsValue::UNDEFINED).into()
                },
            )
            .into_js_value()
            .unchecked_into(),
        );
    }
    {
        let store = Rc::clone(&store);
        install(
            "delete",
            Closure::<dyn FnMut(JsValue) -> JsValue>::new(move |key: JsValue| {
                let key = key.as_string().expect("kv key is a string");
                store.borrow_mut().remove(&key);
                Promise::resolve(&JsValue::UNDEFINED).into()
            })
            .into_js_value()
            .unchecked_into(),
        );
    }
    {
        let store = Rc::clone(&store);
        install(
            "list_by_prefix",
            Closure::<dyn FnMut(JsValue) -> JsValue>::new(move |prefix: JsValue| {
                let prefix = prefix.as_string().expect("kv prefix is a string");
                let keys = js_sys::Array::new();
                for key in store.borrow().keys().filter(|key| key.starts_with(&prefix)) {
                    keys.push(&JsValue::from_str(key));
                }
                Promise::resolve(&JsValue::from(keys)).into()
            })
            .into_js_value()
            .unchecked_into(),
        );
    }
    capability
}

/// R2 + the one-turn law: a suspended fetch caller completes from TRUE IDLE
/// with exactly one coalesced arbiter turn, delivering the D8 codec map
/// verbatim, and the deferred-capability round trip moves no no-op counter.
#[wasm_bindgen_test]
async fn fetch_success_from_true_idle_delivers_the_codec_map_in_one_turn() {
    let mut vm = WasmVm::new().expect("VM constructs");
    capability_call_module(&vm, "wport8_fetch_ok", "wasm_fetch", "request", 1);
    let resolvers = Rc::new(RefCell::new(Vec::new()));
    let abort_calls = Rc::new(Cell::new(0));
    vm.register_fetch_capability(deferred_fetch_capability(&resolvers, &abort_calls));

    let pid = vm
        .spawn(
            "wport8_fetch_ok",
            "call",
            r#"[{"url":"https://example.test/data","method":"POST","headers":{"x-req":"1"},"body":"ping"}]"#,
        )
        .expect("fetch caller spawns");
    host_macrotask().await;
    let before = assert_true_idle(&vm).await;
    assert_eq!(
        resolvers.borrow().len(),
        1,
        "the caller parked on the host op"
    );

    let response = Object::new();
    Reflect::set(
        response.as_ref(),
        &JsValue::from_str("status"),
        &JsValue::from_f64(200.0),
    )
    .expect("status sets");
    let headers = Object::new();
    Reflect::set(
        headers.as_ref(),
        &JsValue::from_str("x-test"),
        &JsValue::from_str("yes"),
    )
    .expect("header sets");
    Reflect::set(
        response.as_ref(),
        &JsValue::from_str("headers"),
        headers.as_ref(),
    )
    .expect("headers set");
    Reflect::set(
        response.as_ref(),
        &JsValue::from_str("body"),
        &JsValue::from_str("hello"),
    )
    .expect("body sets");
    resolvers.borrow()[0]
        .0
        .call1(&JsValue::UNDEFINED, response.as_ref())
        .expect("host resolves the request");

    host_microtask().await;
    let queued = vm.arbiter_counters();
    assert_eq!(
        queued.arbiter.requests,
        before.arbiter.requests + 1,
        "one completion queues exactly one arbiter turn"
    );

    host_macrotask().await;
    let settled = await_exit_json(&mut vm, pid).await;
    assert_eq!(settled["state"], "exited");
    assert_eq!(
        settled["result"],
        json!(["ok", {"body": "hello", "headers": {"x-test": "yes"}, "status": 200}])
    );
    assert_eq!(vm.capability_counters().dead_pid_completions, 0);
    assert_eq!(abort_calls.get(), 0, "no abort fired on a live completion");
}

/// W1 + R4 (BEAM side): a bytecode caller SURVIVES every error leg — each
/// failure resumes it with `{error, {Slug, Detail}}` in x0 as a VALUE, and
/// the slug atoms collected across all five legs are EXACTLY the closed JS
/// kind set.
#[wasm_bindgen_test]
async fn bytecode_caller_survives_every_error_leg_and_the_slug_set_is_closed() {
    let mut vm = WasmVm::new().expect("VM constructs");
    capability_call_module(&vm, "wport8_legs", "wasm_fetch", "request", 1);
    let request_json = r#"[{"url":"https://example.test/"}]"#;
    let mut seen = Vec::new();

    // Leg 1: capability_missing (nothing injected yet).
    let pid = vm
        .spawn("wport8_legs", "call", request_json)
        .expect("spawns");
    let exit = await_exit_json(&mut vm, pid).await;
    assert_eq!(exit["state"], "exited");
    assert_eq!(exit["result"][0], "error");
    seen.push(
        exit["result"][1][0]
            .as_str()
            .expect("slug string")
            .to_owned(),
    );
    assert_eq!(exit["result"][1][1], "fetch");

    // Leg 2: refused (request method throws synchronously).
    let throwing = Object::new();
    let thrower = Function::new_no_args("throw new Error('nope')");
    Reflect::set(
        throwing.as_ref(),
        &JsValue::from_str("request"),
        thrower.as_ref(),
    )
    .expect("method installs");
    vm.register_fetch_capability(throwing);
    let pid = vm
        .spawn("wport8_legs", "call", request_json)
        .expect("spawns");
    let exit = await_exit_json(&mut vm, pid).await;
    assert_eq!(exit["state"], "exited");
    assert_eq!(exit["result"], json!(["error", ["refused", "nope"]]));
    seen.push("refused".to_owned());

    // Legs 3-5 share the deferred capability: rejected, malformed_response,
    // cancelled (an AbortError-named rejection).
    let resolvers = Rc::new(RefCell::new(Vec::new()));
    let abort_calls = Rc::new(Cell::new(0));
    vm.register_fetch_capability(deferred_fetch_capability(&resolvers, &abort_calls));

    let pid = vm
        .spawn("wport8_legs", "call", request_json)
        .expect("spawns");
    host_macrotask().await;
    resolvers.borrow()[0]
        .1
        .call1(&JsValue::UNDEFINED, js_sys::Error::new("boom").as_ref())
        .expect("host rejects");
    host_macrotask().await;
    let exit = await_exit_json(&mut vm, pid).await;
    assert_eq!(exit["state"], "exited");
    assert_eq!(exit["result"], json!(["error", ["rejected", "boom"]]));
    seen.push("rejected".to_owned());

    let pid = vm
        .spawn("wport8_legs", "call", request_json)
        .expect("spawns");
    host_macrotask().await;
    let malformed = Object::new();
    Reflect::set(
        malformed.as_ref(),
        &JsValue::from_str("status"),
        &JsValue::from_str("nope"),
    )
    .expect("bad status sets");
    resolvers.borrow()[1]
        .0
        .call1(&JsValue::UNDEFINED, malformed.as_ref())
        .expect("host resolves malformed");
    host_macrotask().await;
    let exit = await_exit_json(&mut vm, pid).await;
    assert_eq!(exit["state"], "exited");
    assert_eq!(
        exit["result"],
        json!([
            "error",
            [
                "malformed_response",
                "fetch response status is not a number"
            ]
        ])
    );
    seen.push("malformed_response".to_owned());

    let pid = vm
        .spawn("wport8_legs", "call", request_json)
        .expect("spawns");
    host_macrotask().await;
    let abort_error = js_sys::Error::new("aborted mid-flight");
    abort_error.set_name("AbortError");
    resolvers.borrow()[2]
        .1
        .call1(&JsValue::UNDEFINED, abort_error.as_ref())
        .expect("host rejects with AbortError");
    host_macrotask().await;
    let exit = await_exit_json(&mut vm, pid).await;
    assert_eq!(exit["state"], "exited");
    assert_eq!(
        exit["result"],
        json!(["error", ["cancelled", "aborted mid-flight"]])
    );
    seen.push("cancelled".to_owned());

    seen.sort();
    let mut expected: Vec<String> = CAPABILITY_KIND_SLUGS
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    expected.sort();
    assert_eq!(seen, expected, "BEAM slug atoms == the closed JS kind set");
    assert_eq!(vm.capability_counters().dead_pid_completions, 0);
}

/// W2 + W3: a NATIVE caller reaches the capability MFA through the same
/// registration path (caller-type visibility pinned end-to-end) and receives
/// the contract VERBATIM on both arms — `{ok, Value}` / `{error, {Slug,
/// Detail}}` mailbox envelopes with no double wrap.
#[wasm_bindgen_test]
async fn native_caller_receives_verbatim_contract_on_both_arms() {
    struct CapabilityCaller {
        mfa: NativeKey,
        key: Term,
        issued: bool,
        atom_table: StdArc<AtomTable>,
        envelopes: StdArc<Mutex<Vec<serde_json::Value>>>,
    }
    impl NativeHandler for CapabilityCaller {
        fn handle(&mut self, ctx: &mut NativeContext<'_>) -> NativeOutcome {
            if !self.issued {
                self.issued = true;
                return match ctx.start_async(self.mfa, &[self.key]) {
                    Ok(()) => NativeOutcome::Wait,
                    Err(_) => NativeOutcome::Stop(ExitReason::Error),
                };
            }
            while let Some(message) = ctx.recv() {
                if let Ok(value) = term_to_value(message, self.atom_table.as_ref()) {
                    self.envelopes.lock().expect("envelope sink").push(value);
                    return NativeOutcome::Stop(ExitReason::Normal);
                }
            }
            NativeOutcome::Wait
        }
    }

    let mut vm = WasmVm::new().expect("VM constructs");
    vm.register_kv_capability(in_memory_kv_capability());
    let envelopes = StdArc::new(Mutex::new(Vec::new()));
    let get_mfa: NativeKey = (
        vm.atom_table.intern("wasm_kv"),
        vm.atom_table.intern("get"),
        1,
    );
    let present = Term::atom(vm.atom_table.intern("native_key"));
    let atom_table = StdArc::clone(&vm.atom_table);

    // Seed the store through a bytecode put so the native get finds a value.
    capability_call_module(&vm, "wport8_native_seed", "wasm_kv", "put", 2);
    let seeded = run_kv(
        &mut vm,
        "wport8_native_seed",
        r#"["native_key", "native_value"]"#,
    )
    .await;
    assert_eq!(seeded, json!(["ok", true]));

    // Ok arm: the envelope is {ok, <<"native_value">>} — the VALUE directly,
    // never {ok, {ok, ...}}.
    {
        let envelopes = StdArc::clone(&envelopes);
        let atom_table = StdArc::clone(&atom_table);
        let _pid = spawn_native_root_edge(
            &mut vm,
            Box::new(move || {
                Box::new(CapabilityCaller {
                    mfa: get_mfa,
                    key: present,
                    issued: false,
                    atom_table: StdArc::clone(&atom_table),
                    envelopes: StdArc::clone(&envelopes),
                })
            }),
        );
    }
    host_macrotask().await;
    host_macrotask().await;
    assert_eq!(
        envelopes.lock().expect("envelope sink").as_slice(),
        &[json!(["ok", "native_value"])],
        "native ok arm is the verbatim {{ok, Value}} envelope"
    );

    // Error arm: a rejecting fetch capability drives {error, {Slug, Detail}}
    // — the slug tuple directly under the error tag, no double wrap.
    let rejecting = Object::new();
    let reject_fn = Function::new_with_args(
        "req,slot",
        "return Promise.reject(new Error('native boom'))",
    );
    Reflect::set(
        rejecting.as_ref(),
        &JsValue::from_str("request"),
        reject_fn.as_ref(),
    )
    .expect("request installs");
    vm.register_fetch_capability(rejecting);
    let request_mfa: NativeKey = (
        vm.atom_table.intern("wasm_fetch"),
        vm.atom_table.intern("request"),
        1,
    );
    struct FetchCaller {
        mfa: NativeKey,
        request: beamr::ets::OwnedTerm,
        issued: bool,
        atom_table: StdArc<AtomTable>,
        envelopes: StdArc<Mutex<Vec<serde_json::Value>>>,
    }
    impl NativeHandler for FetchCaller {
        fn handle(&mut self, ctx: &mut NativeContext<'_>) -> NativeOutcome {
            if !self.issued {
                self.issued = true;
                let Some(request) = ctx.alloc_owned_term(&self.request) else {
                    return NativeOutcome::Stop(ExitReason::Error);
                };
                return match ctx.start_async(self.mfa, &[request]) {
                    Ok(()) => NativeOutcome::Wait,
                    Err(_) => NativeOutcome::Stop(ExitReason::Error),
                };
            }
            while let Some(message) = ctx.recv() {
                if let Ok(value) = term_to_value(message, self.atom_table.as_ref()) {
                    self.envelopes.lock().expect("envelope sink").push(value);
                    return NativeOutcome::Stop(ExitReason::Normal);
                }
            }
            NativeOutcome::Wait
        }
    }
    let request_owned = {
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(StdArc::clone(&vm.atom_table)));
        let key = context.alloc_binary(b"url").expect("url key allocates");
        let value = context
            .alloc_binary(b"https://example.test/native")
            .expect("url value allocates");
        let map = context
            .alloc_map(&[key], &[value])
            .expect("request map allocates");
        context.take_detached_result(map).expect("owned request")
    };
    let request_slot = StdArc::new(Mutex::new(Some(request_owned)));
    {
        let envelopes = StdArc::clone(&envelopes);
        let atom_table = StdArc::clone(&atom_table);
        let request_slot = StdArc::clone(&request_slot);
        let _pid = spawn_native_root_edge(
            &mut vm,
            Box::new(move || {
                let request = request_slot
                    .lock()
                    .expect("request slot")
                    .take()
                    .expect("the fetch caller spawns once");
                Box::new(FetchCaller {
                    mfa: request_mfa,
                    request,
                    issued: false,
                    atom_table: StdArc::clone(&atom_table),
                    envelopes: StdArc::clone(&envelopes),
                })
            }),
        );
    }
    host_macrotask().await;
    host_macrotask().await;
    let recorded = envelopes.lock().expect("envelope sink").clone();
    assert_eq!(
        recorded.last(),
        Some(&json!(["error", ["rejected", "native boom"]])),
        "native error arm is the verbatim {{error, {{Slug, Detail}}}} envelope"
    );
    assert_eq!(vm.capability_counters().dead_pid_completions, 0);
}

/// R3: the KV surface round-trips against the in-memory host — byte-exact
/// get after put, `{ok, undefined}` for absent, idempotent delete, and
/// lexicographic list_by_prefix (empty prefix = full listing).
#[wasm_bindgen_test]
async fn kv_adapter_round_trips_get_put_delete_and_lexicographic_list() {
    let mut vm = WasmVm::new().expect("VM constructs");
    vm.register_kv_capability(in_memory_kv_capability());
    capability_call_module(&vm, "wport8_kv_put", "wasm_kv", "put", 2);
    capability_call_module(&vm, "wport8_kv_get", "wasm_kv", "get", 1);
    capability_call_module(&vm, "wport8_kv_delete", "wasm_kv", "delete", 1);
    capability_call_module(&vm, "wport8_kv_list", "wasm_kv", "list_by_prefix", 1);

    assert_eq!(
        run_kv(&mut vm, "wport8_kv_put", r#"["b2", "beta"]"#).await,
        json!(["ok", true])
    );
    assert_eq!(
        run_kv(&mut vm, "wport8_kv_put", r#"["a1", "alpha"]"#).await,
        json!(["ok", true])
    );
    assert_eq!(
        run_kv(&mut vm, "wport8_kv_put", r#"["a2", "gamma"]"#).await,
        json!(["ok", true])
    );
    assert_eq!(
        run_kv(&mut vm, "wport8_kv_get", r#"["a1"]"#).await,
        json!(["ok", "alpha"])
    );
    assert_eq!(
        run_kv(&mut vm, "wport8_kv_get", r#"["missing"]"#).await,
        json!(["ok", null]) // atom `undefined` renders as JSON null in exit results
    );
    assert_eq!(
        run_kv(&mut vm, "wport8_kv_list", r#"["a"]"#).await,
        json!(["ok", ["a1", "a2"]])
    );
    assert_eq!(
        run_kv(&mut vm, "wport8_kv_list", r#"[""]"#).await,
        json!(["ok", ["a1", "a2", "b2"]])
    );
    assert_eq!(
        run_kv(&mut vm, "wport8_kv_delete", r#"["a1"]"#).await,
        json!(["ok", true])
    );
    assert_eq!(
        run_kv(&mut vm, "wport8_kv_delete", r#"["a1"]"#).await,
        json!(["ok", true])
    );
    assert_eq!(
        run_kv(&mut vm, "wport8_kv_get", r#"["a1"]"#).await,
        json!(["ok", null]) // atom `undefined` renders as JSON null in exit results
    );
    assert_eq!(vm.capability_counters().dead_pid_completions, 0);
}

/// Drive one bytecode capability call end to end: spawn, let the caller
/// park, let the completion turn run, then read the retained exit result
/// (awaiting exit BEFORE settlement would resolve settled-idle instead —
/// the WPORT-2 idle contract).
async fn run_kv(vm: &mut WasmVm, module: &str, args: &str) -> serde_json::Value {
    let pid = vm.spawn(module, "call", args).expect("kv caller spawns");
    host_macrotask().await;
    host_macrotask().await;
    await_exit_json(vm, pid).await["result"].clone()
}

// ---------------------------------------------------------------------------
// R6 walls: process-death auto-abort (A1 seam), counted dead-pid completion,
// late-abort harmlessness.
// ---------------------------------------------------------------------------

/// R6 + A1: a native caller dying with a fetch in flight fires the abort
/// hook at the arbiter's exit-observation sweep (the ExitWaiter settling
/// point), and the completion later arriving for the drained entry is a
/// COUNTED no-op: dead_pid_completions increments by exactly one and NO
/// arbiter turn is requested for it.
#[wasm_bindgen_test]
async fn dying_caller_auto_aborts_in_flight_and_late_completion_is_counted() {
    struct DieOnWake {
        mfa: NativeKey,
        request: Option<beamr::ets::OwnedTerm>,
        issued: bool,
    }
    impl NativeHandler for DieOnWake {
        fn handle(&mut self, ctx: &mut NativeContext<'_>) -> NativeOutcome {
            if !self.issued {
                self.issued = true;
                let Some(request) = self
                    .request
                    .take()
                    .and_then(|owned| ctx.alloc_owned_term(&owned))
                else {
                    return NativeOutcome::Stop(ExitReason::Error);
                };
                return match ctx.start_async(self.mfa, &[request]) {
                    Ok(()) => NativeOutcome::Wait,
                    Err(_) => NativeOutcome::Stop(ExitReason::Error),
                };
            }
            // Any wake (the test casts a message) kills the caller with the
            // request still in flight.
            while ctx.recv().is_some() {}
            NativeOutcome::Stop(ExitReason::Normal)
        }
    }

    let mut vm = WasmVm::new().expect("VM constructs");
    let resolvers = Rc::new(RefCell::new(Vec::new()));
    let abort_calls = Rc::new(Cell::new(0));
    vm.register_fetch_capability(deferred_fetch_capability(&resolvers, &abort_calls));
    let request_mfa: NativeKey = (
        vm.atom_table.intern("wasm_fetch"),
        vm.atom_table.intern("request"),
        1,
    );
    let request_owned = {
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(StdArc::clone(&vm.atom_table)));
        let key = context.alloc_binary(b"url").expect("url key allocates");
        let value = context
            .alloc_binary(b"https://example.test/doomed")
            .expect("url value allocates");
        let map = context
            .alloc_map(&[key], &[value])
            .expect("request map allocates");
        context.take_detached_result(map).expect("owned request")
    };
    let request_slot = StdArc::new(Mutex::new(Some(request_owned)));
    let pid = {
        let request_slot = StdArc::clone(&request_slot);
        spawn_native_root_edge(
            &mut vm,
            Box::new(move || {
                Box::new(DieOnWake {
                    mfa: request_mfa,
                    request: request_slot.lock().expect("request slot").take(),
                    issued: false,
                })
            }),
        )
    };
    host_macrotask().await;
    assert_eq!(resolvers.borrow().len(), 1, "the request is in flight");
    assert_eq!(abort_calls.get(), 0, "no abort while the caller lives");

    // Kill the caller: the cast wakes it, it stops Normal, and the exit
    // sweep fires the abort hook for its in-flight request.
    vm.cast(pid, JsValue::from_f64(1.0)).expect("cast delivers");
    host_macrotask().await;
    assert_eq!(
        abort_calls.get(),
        1,
        "the death sweep fired the abort hook exactly once"
    );
    assert_eq!(vm.capability_counters().dead_pid_completions, 0);

    // The host settles AFTER the death: the completion finds the entry
    // drained — counted, undelivered, and no arbiter turn requested.
    let before = vm.arbiter_counters();
    resolvers.borrow()[0]
        .1
        .call1(
            &JsValue::UNDEFINED,
            js_sys::Error::new("too late for anyone").as_ref(),
        )
        .expect("host rejects post-mortem");
    host_macrotask().await;
    assert_eq!(vm.capability_counters().dead_pid_completions, 1);
    let after = vm.arbiter_counters();
    assert_eq!(
        after.arbiter.requests, before.arbiter.requests,
        "a dead-pid completion requests no turn"
    );
}

/// R6: an abort arriving AFTER successful completion is harmless — the
/// promise-settles-once platform law makes a second completion structurally
/// impossible; the result stands, counters do not move, and the abort hook
/// firing late disturbs nothing.
#[wasm_bindgen_test]
async fn late_abort_after_completion_is_a_harmless_noop() {
    let mut vm = WasmVm::new().expect("VM constructs");
    capability_call_module(&vm, "wport8_late_abort", "wasm_fetch", "request", 1);
    let resolvers = Rc::new(RefCell::new(Vec::new()));
    let abort_calls = Rc::new(Cell::new(0));
    vm.register_fetch_capability(deferred_fetch_capability(&resolvers, &abort_calls));

    let pid = vm
        .spawn(
            "wport8_late_abort",
            "call",
            r#"[{"url":"https://example.test/settled"}]"#,
        )
        .expect("caller spawns");
    host_macrotask().await;
    let response = Object::new();
    Reflect::set(
        response.as_ref(),
        &JsValue::from_str("status"),
        &JsValue::from_f64(204.0),
    )
    .expect("status sets");
    resolvers.borrow()[0]
        .0
        .call1(&JsValue::UNDEFINED, response.as_ref())
        .expect("host resolves");
    host_macrotask().await;
    let settled = await_exit_json(&mut vm, pid).await;
    assert_eq!(
        settled["result"],
        json!(["ok", {"body": "", "headers": {}, "status": 204}])
    );
    let counters_before = vm.capability_counters();
    let arbiter_before = vm.arbiter_counters();

    // The host aborts and rejects AFTER settlement: the promise is already
    // resolved, so the rejection is a platform no-op; nothing moves.
    resolvers.borrow()[0]
        .1
        .call1(
            &JsValue::UNDEFINED,
            js_sys::Error::new("late abort").as_ref(),
        )
        .expect("post-settle rejection is callable");
    host_macrotask().await;
    assert_eq!(vm.capability_counters(), counters_before);
    let arbiter_after = vm.arbiter_counters();
    assert_eq!(
        arbiter_after.arbiter.requests,
        arbiter_before.arbiter.requests
    );
    assert_eq!(
        abort_calls.get(),
        0,
        "the bridge fired no abort for a live completion"
    );
}
