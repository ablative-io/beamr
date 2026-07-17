//! WPORT-6 R3: the artifact-loader wall battery — the repo's FIRST wasm-side
//! executions of the real `.beam` decode path and FIRST real-VM
//! JS-orchestration tests (at the brief's pin no wasm test exercised
//! `load_module`; the decoder was bypassed entirely on wasm — ground pack §6).
//! Every wall drives the loader through the exported `load_artifacts` surface
//! with an injected fetch double; no wall pumps manually and no wall polls —
//! completion re-enters via each fetch Promise's own microtask continuation.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Function, Promise, Uint8Array};
use serde_json::{Value, json};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test;

use super::KIND_SLUGS;
use crate::WasmVm;

const CHAIN_A: &[u8] = include_bytes!("../fixtures/fetch_chain_a.beam");
const CHAIN_B: &[u8] = include_bytes!("../fixtures/fetch_chain_b.beam");
const CHAIN_C: &[u8] = include_bytes!("../fixtures/fetch_chain_c.beam");
const CYCLE_PING: &[u8] = include_bytes!("../fixtures/fetch_cycle_ping.beam");
const CYCLE_PONG: &[u8] = include_bytes!("../fixtures/fetch_cycle_pong.beam");
const UNRESOLVED_CALLER: &[u8] = include_bytes!("../fixtures/fetch_unresolved_caller.beam");
/// Deliberately NOT a FOR1/BEAM container (the bytes are a prose note saying
/// so): loading must produce `LoadError::InvalidFormat`, surfaced as the
/// `artifact_invalid_format` kind slug.
const MALFORMED_NOT_BEAM: &[u8] = include_bytes!("../fixtures/malformed_not_beam.beam");

const BASE: &str = "https://fixtures.test/mods/manifest.json";

/// One scripted reply of the injected-fetch double.
enum Reply {
    Bytes(&'static [u8]),
    Crafted(Vec<u8>),
    Text(String),
    Reject(&'static str),
    NonPromise,
}

/// Build an injected-fetch double serving scripted replies by absolute URL,
/// plus the shared call log (order-observing). Unknown URLs reject loudly.
fn fetch_double(replies: Vec<(String, Reply)>) -> (Function, Rc<RefCell<Vec<String>>>) {
    let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let calls = Rc::clone(&log);
    let closure = Closure::<dyn FnMut(JsValue) -> JsValue>::new(move |url: JsValue| {
        let url = url.as_string().expect("fetch double receives a URL string");
        calls.borrow_mut().push(url.clone());
        match replies.iter().find(|(key, _)| *key == url).map(|(_, r)| r) {
            Some(Reply::Bytes(bytes)) => resolve_bytes(bytes),
            Some(Reply::Crafted(bytes)) => resolve_bytes(bytes),
            Some(Reply::Text(text)) => resolve_bytes(text.as_bytes()),
            Some(Reply::Reject(reason)) => Promise::reject(&JsValue::from_str(reason)).into(),
            Some(Reply::NonPromise) => JsValue::from_f64(42.0),
            None => Promise::reject(&JsValue::from_str("unexpected URL")).into(),
        }
    });
    (closure.into_js_value().unchecked_into(), log)
}

fn resolve_bytes(bytes: &[u8]) -> JsValue {
    let bytes: JsValue = Uint8Array::from(bytes).into();
    Promise::resolve(&bytes).into()
}

fn url(name: &str) -> String {
    format!("https://fixtures.test/mods/{name}")
}

/// Manifest v1 text from `(name, url, deps)` rows.
fn manifest(modules: &[(&str, &str, &[&str])]) -> String {
    let rows: Vec<Value> = modules
        .iter()
        .map(|(name, url, deps)| json!({"name": name, "url": url, "deps": deps}))
        .collect();
    json!({"format": "beamr-fetch-manifest", "version": 1, "modules": rows}).to_string()
}

async fn load(vm: &WasmVm, fetch: &Function) -> Result<Value, JsValue> {
    let value = JsFuture::from(vm.load_artifacts(BASE.to_string(), fetch.clone())).await?;
    let text = value
        .as_string()
        .expect("loader resolves with a JSON string");
    Ok(serde_json::from_str(&text).expect("loader report JSON parses"))
}

/// Await a pid's settled completion through the existing export surface and
/// parse the completion JSON string — zero manual pump calls.
async fn await_completion(vm: &mut WasmVm, pid: u64) -> Value {
    let value = JsFuture::from(vm.await_exit(pid))
        .await
        .expect("await_exit resolves");
    let text = value.as_string().expect("completion is a JSON string");
    serde_json::from_str(&text).expect("completion JSON parses")
}

/// Assert an `ArtifactLoadError` rejection with the expected kind slug and
/// return the parsed `data` payload (OQ-D RATIFIED: one `data` property
/// holding the JSON string `{"artifact","url","stage","loaded"}`).
fn assert_rejects(outcome: Result<Value, JsValue>, kind: &str) -> Value {
    let rejection = outcome.expect_err("the batch must reject");
    let error = rejection
        .dyn_ref::<js_sys::Error>()
        .expect("rejection is an Error");
    assert_eq!(String::from(error.name()), "ArtifactLoadError");
    let message = String::from(error.message());
    assert!(
        message.starts_with(&format!("{kind}: ")),
        "message {message:?} must start with the {kind} slug"
    );
    let data = js_sys::Reflect::get(&rejection, &JsValue::from_str("data"))
        .expect("rejection carries a data property")
        .as_string()
        .expect("data is a JSON string");
    serde_json::from_str(&data).expect("data JSON parses")
}

fn loaded_modules(report: &Value) -> Vec<String> {
    report["loaded"]
        .as_array()
        .expect("loaded is an array")
        .iter()
        .map(|module| {
            module["module"]
                .as_str()
                .expect("module name is a string")
                .to_owned()
        })
        .collect()
}

// ---- crafted malformed blobs (wall 5) -------------------------------------

fn chunk(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(tag);
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(payload);
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    bytes
}

fn beam_container(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut body = b"BEAM".to_vec();
    for chunk in chunks {
        body.extend_from_slice(chunk);
    }
    let mut bytes = b"FOR1".to_vec();
    bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
    bytes.extend(body);
    bytes
}

fn atom_chunk(names: &[&str]) -> Vec<u8> {
    let mut payload = (names.len() as u32).to_be_bytes().to_vec();
    for name in names {
        payload.push(name.len() as u8);
        payload.extend_from_slice(name.as_bytes());
    }
    chunk(b"AtU8", &payload)
}

/// Valid container, valid atoms, no `Code` chunk:
/// `LoadError::MissingChunk("Code")` -> `artifact_invalid_format`.
fn missing_code_chunk_blob() -> Vec<u8> {
    beam_container(&[atom_chunk(&["wport6_missing_code"])])
}

/// `Code` chunk shorter than its 20-byte header:
/// `LoadError::DecodeError` -> `artifact_decode_failed`.
fn truncated_code_header_blob() -> Vec<u8> {
    beam_container(&[
        atom_chunk(&["wport6_bad_code"]),
        chunk(b"Code", &[0, 0, 0, 0]),
    ])
}

/// Decodes cleanly (empty instruction stream) but exports label 1, which
/// does not exist: `LoadError::ValidationError` -> `artifact_validation_failed`.
fn dangling_export_label_blob() -> Vec<u8> {
    let mut code = Vec::new();
    for header_word in [16u32, 0, 0, 0, 0] {
        code.extend_from_slice(&header_word.to_be_bytes());
    }
    let mut exports = 1u32.to_be_bytes().to_vec();
    for export_word in [1u32, 0, 1] {
        exports.extend_from_slice(&export_word.to_be_bytes());
    }
    beam_container(&[
        atom_chunk(&["wport6_bad_export"]),
        chunk(b"Code", &code),
        chunk(b"ExpT", &exports),
    ])
}

// ---- the walls ------------------------------------------------------------

#[wasm_bindgen_test]
async fn manifest_chain_loads_dependencies_before_dependants_with_clean_report() {
    let vm = WasmVm::new().expect("VM constructs");
    // Dependants listed FIRST: Kahn order must invert the manifest order.
    let (fetch, log) = fetch_double(vec![
        (
            BASE.to_string(),
            Reply::Text(manifest(&[
                ("fetch_chain_a", "fetch_chain_a.beam", &["fetch_chain_b"]),
                ("fetch_chain_b", "fetch_chain_b.beam", &["fetch_chain_c"]),
                ("fetch_chain_c", "fetch_chain_c.beam", &[]),
            ])),
        ),
        (url("fetch_chain_a.beam"), Reply::Bytes(CHAIN_A)),
        (url("fetch_chain_b.beam"), Reply::Bytes(CHAIN_B)),
        (url("fetch_chain_c.beam"), Reply::Bytes(CHAIN_C)),
    ]);
    let report = load(&vm, &fetch).await.expect("batch resolves");
    assert_eq!(report["ok"], json!(true));
    assert_eq!(
        loaded_modules(&report),
        ["fetch_chain_c", "fetch_chain_b", "fetch_chain_a"]
    );
    for module in report["loaded"].as_array().expect("loaded is an array") {
        assert_eq!(
            module["unresolved"],
            json!([]),
            "preserved-empty unresolved"
        );
        assert_eq!(module["deferred"], json!([]));
        assert_eq!(module["denied"], json!([]));
    }
    assert_eq!(report["cycles"], json!([]));
    assert_eq!(report["missing_dependencies"], json!([]));
    let calls = log.borrow();
    assert_eq!(
        calls[..],
        [
            BASE.to_string(),
            url("fetch_chain_c.beam"),
            url("fetch_chain_b.beam"),
            url("fetch_chain_a.beam"),
        ],
        "fetches follow Kahn order, dependencies first"
    );
}

#[wasm_bindgen_test]
async fn mutual_cycle_pair_loads_in_manifest_order_and_reports_the_scc() {
    let vm = WasmVm::new().expect("VM constructs");
    let (fetch, log) = fetch_double(vec![
        (
            BASE.to_string(),
            Reply::Text(manifest(&[
                (
                    "fetch_cycle_ping",
                    "fetch_cycle_ping.beam",
                    &["fetch_cycle_pong"],
                ),
                (
                    "fetch_cycle_pong",
                    "fetch_cycle_pong.beam",
                    &["fetch_cycle_ping"],
                ),
            ])),
        ),
        (url("fetch_cycle_ping.beam"), Reply::Bytes(CYCLE_PING)),
        (url("fetch_cycle_pong.beam"), Reply::Bytes(CYCLE_PONG)),
    ]);
    let report = load(&vm, &fetch).await.expect("cycle is non-fatal (OQ-B)");
    assert_eq!(
        loaded_modules(&report),
        ["fetch_cycle_ping", "fetch_cycle_pong"],
        "SCC members load in manifest order"
    );
    assert_eq!(
        report["cycles"],
        json!([["fetch_cycle_ping", "fetch_cycle_pong"]]),
        "the SCC is a structured, named report entry"
    );
    assert_eq!(
        report["missing_dependencies"],
        json!([]),
        "both deferred halves healed post-batch"
    );
    assert_eq!(log.borrow().len(), 3);
}

#[wasm_bindgen_test]
async fn dep_edge_naming_module_absent_from_manifest_rejects_dependency_missing() {
    let vm = WasmVm::new().expect("VM constructs");
    let (fetch, log) = fetch_double(vec![(
        BASE.to_string(),
        Reply::Text(manifest(&[(
            "fetch_chain_a",
            "fetch_chain_a.beam",
            &["fetch_ghost"],
        )])),
    )]);
    let data = assert_rejects(load(&vm, &fetch).await, "dependency_missing");
    assert_eq!(data["artifact"], json!("fetch_ghost"));
    assert_eq!(data["stage"], json!("order"));
    assert_eq!(data["loaded"], json!([]));
    assert_eq!(
        log.borrow()[..],
        [BASE.to_string()],
        "fatal pre-fetch: no artifact fetch happens"
    );
}

#[wasm_bindgen_test]
async fn rejecting_fetch_double_rejects_batch_with_artifact_fetch_failed_and_loaded_list() {
    let vm = WasmVm::new().expect("VM constructs");
    let (fetch, _log) = fetch_double(vec![
        (
            BASE.to_string(),
            Reply::Text(manifest(&[
                ("fetch_chain_c", "fetch_chain_c.beam", &[]),
                ("fetch_chain_b", "fetch_chain_b.beam", &["fetch_chain_c"]),
            ])),
        ),
        (url("fetch_chain_c.beam"), Reply::Bytes(CHAIN_C)),
        (url("fetch_chain_b.beam"), Reply::Reject("status 500")),
    ]);
    let data = assert_rejects(load(&vm, &fetch).await, "artifact_fetch_failed");
    assert_eq!(data["artifact"], json!("fetch_chain_b"));
    assert_eq!(data["url"], json!(url("fetch_chain_b.beam")));
    assert_eq!(data["stage"], json!("fetch"));
    assert_eq!(
        data["loaded"],
        json!(["fetch_chain_c"]),
        "the rejection is honest about what already loaded (no unload)"
    );
}

#[wasm_bindgen_test]
async fn malformed_artifact_bytes_reject_with_exact_decode_stage_kind_slugs() {
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        // LoadError::InvalidFormat (not FOR1/BEAM) -> artifact_invalid_format
        (
            "not_beam",
            MALFORMED_NOT_BEAM.to_vec(),
            "artifact_invalid_format",
        ),
        // LoadError::MissingChunk("Code") -> artifact_invalid_format
        (
            "missing_code",
            missing_code_chunk_blob(),
            "artifact_invalid_format",
        ),
        // LoadError::DecodeError (truncated Code header) -> artifact_decode_failed
        (
            "truncated_code",
            truncated_code_header_blob(),
            "artifact_decode_failed",
        ),
        // LoadError::ValidationError (export label absent) -> artifact_validation_failed
        (
            "dangling_export",
            dangling_export_label_blob(),
            "artifact_validation_failed",
        ),
    ];
    for (label, bytes, expected_slug) in cases {
        let vm = WasmVm::new().expect("VM constructs");
        let (fetch, _log) = fetch_double(vec![
            (
                BASE.to_string(),
                Reply::Text(manifest(&[("bad_mod", "bad_mod.beam", &[])])),
            ),
            (url("bad_mod.beam"), Reply::Crafted(bytes)),
        ]);
        let data = assert_rejects(load(&vm, &fetch).await, expected_slug);
        assert_eq!(data["artifact"], json!("bad_mod"), "case {label}");
        assert_eq!(data["stage"], json!("load"), "case {label}");
    }
}

#[wasm_bindgen_test]
async fn non_promise_fetch_return_rejects_with_fetch_protocol() {
    let vm = WasmVm::new().expect("VM constructs");
    let (fetch, _log) = fetch_double(vec![
        (
            BASE.to_string(),
            Reply::Text(manifest(&[("fetch_chain_c", "fetch_chain_c.beam", &[])])),
        ),
        (url("fetch_chain_c.beam"), Reply::NonPromise),
    ]);
    let data = assert_rejects(load(&vm, &fetch).await, "fetch_protocol");
    assert_eq!(data["artifact"], json!("fetch_chain_c"));
    assert_eq!(data["stage"], json!("fetch"));
}

#[wasm_bindgen_test]
async fn duplicate_module_names_in_manifest_reject_with_manifest_malformed() {
    let vm = WasmVm::new().expect("VM constructs");
    let (fetch, log) = fetch_double(vec![(
        BASE.to_string(),
        Reply::Text(manifest(&[
            ("fetch_chain_c", "fetch_chain_c.beam", &[]),
            ("fetch_chain_c", "fetch_chain_c_copy.beam", &[]),
        ])),
    )]);
    let data = assert_rejects(load(&vm, &fetch).await, "manifest_malformed");
    assert_eq!(data["stage"], json!("manifest"));
    assert_eq!(
        log.borrow().len(),
        1,
        "per-batch dedupe rejects before any fetch"
    );
}

#[wasm_bindgen_test]
async fn dependant_before_dependency_defers_then_heals_at_call_time() {
    let mut vm = WasmVm::new().expect("VM constructs");
    // The dep edge is deliberately OMITTED and the dependant listed first, so
    // manifest order loads fetch_chain_b before fetch_chain_c in one batch.
    let (fetch, _log) = fetch_double(vec![
        (
            BASE.to_string(),
            Reply::Text(manifest(&[
                ("fetch_chain_b", "fetch_chain_b.beam", &[]),
                ("fetch_chain_c", "fetch_chain_c.beam", &[]),
            ])),
        ),
        (url("fetch_chain_b.beam"), Reply::Bytes(CHAIN_B)),
        (url("fetch_chain_c.beam"), Reply::Bytes(CHAIN_C)),
    ]);
    let report = load(&vm, &fetch).await.expect("batch resolves");
    assert_eq!(loaded_modules(&report), ["fetch_chain_b", "fetch_chain_c"]);
    assert_eq!(
        report["loaded"][0]["deferred"],
        json!([{"module": "fetch_chain_c", "function": "base", "arity": 0}]),
        "the dependant's import was Deferred at its load"
    );
    assert_eq!(
        report["missing_dependencies"],
        json!([]),
        "post-batch verification observed the deferred import healed"
    );
    // A call THROUGH the deferred import succeeds (call-time re-resolution).
    let pid = vm
        .spawn("fetch_chain_b", "double", "[20]")
        .expect("spawn through the existing export succeeds");
    let completion = await_completion(&mut vm, pid).await;
    assert_eq!(completion["state"], json!("exited"));
    assert_eq!(completion["result"], json!(40));
}

#[wasm_bindgen_test]
async fn post_batch_verification_asserts_in_manifest_deferreds_healed_and_reports_absent_targets() {
    let vm = WasmVm::new().expect("VM constructs");
    let (fetch, _log) = fetch_double(vec![
        (
            BASE.to_string(),
            Reply::Text(manifest(&[
                ("fetch_chain_c", "fetch_chain_c.beam", &[]),
                (
                    "fetch_unresolved_caller",
                    "fetch_unresolved_caller.beam",
                    &["fetch_chain_c"],
                ),
                ("fetch_cycle_ping", "fetch_cycle_ping.beam", &[]),
                ("fetch_cycle_pong", "fetch_cycle_pong.beam", &[]),
            ])),
        ),
        (url("fetch_chain_c.beam"), Reply::Bytes(CHAIN_C)),
        (
            url("fetch_unresolved_caller.beam"),
            Reply::Bytes(UNRESOLVED_CALLER),
        ),
        (url("fetch_cycle_ping.beam"), Reply::Bytes(CYCLE_PING)),
        (url("fetch_cycle_pong.beam"), Reply::Bytes(CYCLE_PONG)),
    ]);
    let report = load(&vm, &fetch).await.expect("batch resolves");
    let caller = report["loaded"]
        .as_array()
        .expect("loaded is an array")
        .iter()
        .find(|module| module["module"] == json!("fetch_unresolved_caller"))
        .expect("caller entry present");
    assert_eq!(
        caller["unresolved"],
        json!([{"module": "fetch_chain_c", "function": "not_exported", "arity": 0}]),
        "truly-unresolved keeps today's exact entry shape"
    );
    let deferred = caller["deferred"].as_array().expect("deferred is an array");
    assert!(
        deferred.contains(&json!({"module": "fetch_absent_dep", "function": "helper", "arity": 0})),
        "absent target stayed deferred at load: {deferred:?}"
    );
    assert!(
        deferred.contains(&json!({"module": "fetch_cycle_ping", "function": "bounce", "arity": 1})),
        "in-manifest later module was deferred at load: {deferred:?}"
    );
    assert_eq!(
        report["missing_dependencies"],
        json!([{"module": "fetch_absent_dep", "function": "helper", "arity": 0}]),
        "only the absent target is reported; healed deferreds are verified healed"
    );
}

#[wasm_bindgen_test]
async fn manifest_url_to_executed_module_end_to_end_reaches_settled_completion() {
    // The arc acceptance wall: manifest URL in, injected fetch, deps-first
    // load, spawn via the EXISTING export, real VM execution, settled
    // completion with the module's result — zero manual pump calls.
    let mut vm = WasmVm::new().expect("VM constructs");
    let (fetch, log) = fetch_double(vec![
        (
            BASE.to_string(),
            Reply::Text(manifest(&[
                ("fetch_chain_a", "fetch_chain_a.beam", &["fetch_chain_b"]),
                ("fetch_chain_b", "fetch_chain_b.beam", &["fetch_chain_c"]),
                ("fetch_chain_c", "fetch_chain_c.beam", &[]),
            ])),
        ),
        (url("fetch_chain_a.beam"), Reply::Bytes(CHAIN_A)),
        (url("fetch_chain_b.beam"), Reply::Bytes(CHAIN_B)),
        (url("fetch_chain_c.beam"), Reply::Bytes(CHAIN_C)),
    ]);
    let report = load(&vm, &fetch).await.expect("batch resolves");
    assert_eq!(report["ok"], json!(true));
    assert_eq!(log.borrow().len(), 4);
    let pid = vm
        .spawn("fetch_chain_a", "run", "[]")
        .expect("spawn through the existing export succeeds");
    let completion = await_completion(&mut vm, pid).await;
    assert_eq!(completion["state"], json!("exited"), "settled completion");
    assert_eq!(
        completion["result"],
        json!(42),
        "real execution through the fetched chain"
    );
}

#[wasm_bindgen_test]
async fn failing_manifest_fetch_rejects_with_manifest_fetch_failed() {
    let vm = WasmVm::new().expect("VM constructs");
    let (fetch, log) = fetch_double(vec![(BASE.to_string(), Reply::Reject("status 404"))]);
    let data = assert_rejects(load(&vm, &fetch).await, "manifest_fetch_failed");
    assert_eq!(data["artifact"], json!(null));
    assert_eq!(data["url"], json!(BASE));
    assert_eq!(data["stage"], json!("manifest"));
    assert_eq!(
        data["loaded"],
        json!([]),
        "nothing loaded at the manifest stage"
    );
    assert_eq!(log.borrow().len(), 1);
}

#[wasm_bindgen_test]
fn artifact_load_error_kind_slug_set_is_closed_and_exact() {
    // The R1 "pinned by a test" enumeration (wall 13): exact equality with
    // the eight documented slugs — additions and removals both fail here.
    assert_eq!(
        KIND_SLUGS,
        [
            "manifest_fetch_failed",
            "manifest_malformed",
            "dependency_missing",
            "artifact_fetch_failed",
            "fetch_protocol",
            "artifact_invalid_format",
            "artifact_decode_failed",
            "artifact_validation_failed",
        ]
    );
}
