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
    assert_eq!(counters.stale_completion_noops, 0);
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
    assert_eq!(counters.stale_completion_noops, 0);
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
