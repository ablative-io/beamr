//! WPORT-6 runtime artifact loader: fetch-manifest orchestration over the
//! in-crate core loader (`beamr::loader::load_module_with_origin`).
//!
//! The loader is HOST-SIDE orchestration only: it fetches a versioned
//! manifest (schema: `docs/design/beamr/FETCH-MANIFEST.md`) through an
//! injected fetch capability, orders modules dependencies-first (Kahn over
//! the declared edges; strongly-connected components tolerated in manifest
//! order and reported — OQ-B RULED non-fatal), loads each `.beam` artifact
//! through the core loader with `ModuleOrigin::Fetched`, and resolves with a
//! JSON-string batch report that preserves today's unresolved vocabulary and
//! additively surfaces the deferred/denied buckets core already computes.
//! Operational failures reject the returned Promise with the one named error
//! class `ArtifactLoadError` carrying a closed kind-slug set.
//!
//! Discipline (WPORT-6 R1, grep-provable): this file holds `Arc` clones of
//! the three load inputs ONLY — it never touches the scheduler cell, makes
//! zero scheduling calls, and progress re-enters solely through each fetch
//! Promise's own microtask continuation (NO-POLLING). Loading registers code
//! and produces no runnable edge; execution after load is the host's explicit
//! act through the existing export surface.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use beamr::atom::{Atom, AtomTable};
use beamr::error::LoadError;
use beamr::loader::{DeniedImportEntry, UnresolvedImportEntry, load_module_with_origin};
use beamr::module::{ModuleOrigin, ModuleRegistry};
use beamr::native::BifRegistryImpl;
use js_sys::{Function, Promise, Reflect, Uint8Array};
use serde_json::{Value, json};
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, future_to_promise};

use crate::WasmVm;

/// The closed `ArtifactLoadError` kind-slug set (WPORT-6 D6), pinned by the
/// R3 wall-13 enumeration test. The last three slugs mirror core `LoadError`
/// variants matched in-crate — see [`load_error_slug`].
pub const KIND_SLUGS: [&str; 8] = [
    "manifest_fetch_failed",
    "manifest_malformed",
    "dependency_missing",
    "artifact_fetch_failed",
    "fetch_protocol",
    "artifact_invalid_format",
    "artifact_decode_failed",
    "artifact_validation_failed",
];

/// Stage vocabulary carried in the rejection `data` payload (OQ-D RATIFIED
/// mechanics: one `data` property holding the JSON string
/// `{"artifact","url","stage","loaded"}`).
const STAGE_MANIFEST: &str = "manifest";
const STAGE_ORDER: &str = "order";
const STAGE_FETCH: &str = "fetch";
const STAGE_LOAD: &str = "load";

/// The three shared load inputs the batch future owns (WPORT-6 D3): the core
/// load call consumes exactly these plus an origin, so the future never needs
/// any other `WasmVm` state.
struct LoadInputs {
    atom_table: Arc<AtomTable>,
    module_registry: Arc<ModuleRegistry>,
    bif_registry: Arc<BifRegistryImpl>,
}

#[wasm_bindgen]
impl WasmVm {
    /// Fetch, order, and load a batch of `.beam` artifacts named by a runtime
    /// fetch manifest (WPORT-6; schema v1 in
    /// `docs/design/beamr/FETCH-MANIFEST.md`).
    ///
    /// `fetch` is the injected fetch capability: a function taking one URL
    /// string and returning a thenable resolving to an `ArrayBuffer` or
    /// `Uint8Array`. It is called once for the manifest URL and once per
    /// artifact URL (resolved relative to the manifest URL). No global fetch
    /// is probed; explicit injection is the whole contract.
    ///
    /// Resolves with a JSON-string batch report
    /// `{"ok":true,"loaded":[{"module","unresolved","deferred","denied"},...],
    /// "cycles":[[...],...],"missing_dependencies":[...]}`; rejects fail-fast
    /// with an `ArtifactLoadError` (`"{kind}: {detail}"`) whose `data`
    /// property is the JSON string `{"artifact","url","stage","loaded"}` —
    /// honest about the no-unload reality: modules loaded before the failure
    /// stay loaded.
    pub fn load_artifacts(&self, manifest_url: String, fetch: Function) -> Promise {
        let inputs = LoadInputs {
            atom_table: Arc::clone(&self.atom_table),
            module_registry: Arc::clone(&self.module_registry),
            bif_registry: Arc::clone(&self.bif_registry),
        };
        future_to_promise(load_batch(inputs, manifest_url, fetch))
    }
}

/// One fail-fast batch: manifest fetch, parse, edge validation, ordering,
/// fetch+load loop, post-batch verification, report.
async fn load_batch(
    inputs: LoadInputs,
    manifest_url: String,
    fetch: Function,
) -> Result<JsValue, JsValue> {
    let manifest_bytes = fetch_bytes(&fetch, &manifest_url).await.map_err(|issue| {
        issue.reject(
            "manifest_fetch_failed",
            None,
            &manifest_url,
            STAGE_MANIFEST,
            &[],
        )
    })?;
    let manifest_text = String::from_utf8(manifest_bytes)
        .map_err(|_| manifest_error("manifest bytes are not valid UTF-8", &manifest_url))?;
    let entries =
        parse_manifest(&manifest_text).map_err(|detail| manifest_error(&detail, &manifest_url))?;

    // Fatal pre-fetch (D2): a dep edge naming a module absent from the
    // manifest is the arc's "unsatisfied ordering".
    let manifest_names: HashSet<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    for entry in &entries {
        for dep in &entry.deps {
            if !manifest_names.contains(dep.as_str()) {
                return Err(artifact_load_error(
                    "dependency_missing",
                    &format!(
                        "module {} depends on {dep}, which is not in the manifest",
                        entry.name
                    ),
                    Some(dep),
                    None,
                    STAGE_ORDER,
                    &[],
                ));
            }
        }
    }

    let (order, cycles) = order_modules(&entries);

    let mut loaded_names: Vec<String> = Vec::with_capacity(order.len());
    let mut module_reports: Vec<Value> = Vec::with_capacity(order.len());
    let mut batch_deferred: Vec<UnresolvedImportEntry> = Vec::new();
    for index in order {
        let entry = &entries[index];
        let url = resolve_url(&manifest_url, &entry.url);
        let bytes = fetch_bytes(&fetch, &url).await.map_err(|issue| {
            issue.reject(
                "artifact_fetch_failed",
                Some(&entry.name),
                &url,
                STAGE_FETCH,
                &loaded_names,
            )
        })?;
        let (module, report) = load_module_with_origin(
            &bytes,
            inputs.atom_table.as_ref(),
            inputs.module_registry.as_ref(),
            inputs.bif_registry.as_ref(),
            ModuleOrigin::Fetched,
        )
        .map_err(|error| {
            artifact_load_error(
                load_error_slug(&error),
                &error.to_string(),
                Some(&entry.name),
                Some(&url),
                STAGE_LOAD,
                &loaded_names,
            )
        })?;
        // Reporting keys off the ACTUAL loaded module name; the manifest name
        // is trusted for ordering and URLs only. A manifest/artifact name
        // mismatch is not enforced — it surfaces honestly as unhealed
        // deferred imports in `missing_dependencies` (FETCH-MANIFEST.md).
        let loaded_name = atom_name(inputs.atom_table.as_ref(), module.name);
        let deferred_entries = report.deferred_imports();
        module_reports.push(json!({
            "module": loaded_name,
            "unresolved":
                crate::unresolved_imports_to_json(report.imports(), inputs.atom_table.as_ref()),
            "deferred": crate::unresolved_imports_to_json(
                deferred_entries.clone(),
                inputs.atom_table.as_ref(),
            ),
            "denied": denied_imports_to_json(report.denied_imports(), inputs.atom_table.as_ref()),
        }));
        batch_deferred.extend(deferred_entries);
        loaded_names.push(loaded_name);
    }

    let missing_dependencies = post_batch_missing(&inputs, &batch_deferred);
    let report = json!({
        "ok": true,
        "loaded": module_reports,
        "cycles": cycles,
        "missing_dependencies": missing_dependencies,
    });
    crate::json_to_js(&report)
}

/// POST-BATCH VERIFICATION (WPORT-6 D2/D7): the healed observable is a
/// registry/export lookup succeeding NOW — never mutated bucket state, since
/// `resolved_imports` is immutable inside `Arc<Module>` and healing is
/// call-time re-resolution. Deferred entries whose target MFA still resolves
/// to no registered export are reported as `missing_dependencies` data,
/// deduplicated by MFA — never a failure (hazard 11: unregistered targets are
/// routine report data on the browser profile).
fn post_batch_missing(inputs: &LoadInputs, deferred: &[UnresolvedImportEntry]) -> Vec<Value> {
    let mut reported: HashSet<(Atom, Atom, u8)> = HashSet::new();
    let mut missing = Vec::new();
    for entry in deferred {
        let healed = inputs
            .module_registry
            .lookup_mfa(entry.module, entry.function, entry.arity)
            .is_ok();
        if healed {
            continue;
        }
        if reported.insert((entry.module, entry.function, entry.arity)) {
            missing.push(json!({
                "module": atom_name(inputs.atom_table.as_ref(), entry.module),
                "function": atom_name(inputs.atom_table.as_ref(), entry.function),
                "arity": entry.arity,
            }));
        }
    }
    missing
}

/// One parsed manifest module row.
struct ManifestEntry {
    name: String,
    url: String,
    deps: Vec<String>,
}

/// Parse and schema-check a v1 fetch manifest (WPORT-6 D1). The reserved
/// `integrity` field (OQ-A RULED IN) is accepted and deliberately unenforced
/// in v1: the parser does not read it. Missing `deps` means no declared
/// edges. Duplicate module names are fatal (`manifest_malformed`, D9:
/// per-batch dedupe only).
fn parse_manifest(text: &str) -> Result<Vec<ManifestEntry>, String> {
    let root: Value =
        serde_json::from_str(text).map_err(|error| format!("manifest is not JSON: {error}"))?;
    if root.get("format").and_then(Value::as_str) != Some("beamr-fetch-manifest") {
        return Err("manifest `format` is not \"beamr-fetch-manifest\"".to_string());
    }
    if root.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("manifest `version` is not 1".to_string());
    }
    let modules = root
        .get("modules")
        .and_then(Value::as_array)
        .ok_or_else(|| "manifest `modules` is not an array".to_string())?;
    let mut entries = Vec::with_capacity(modules.len());
    let mut seen: HashSet<String> = HashSet::new();
    for module in modules {
        let name = module
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "module entry `name` is not a string".to_string())?;
        let url = module
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("module {name}: `url` is not a string"))?;
        let deps = match module.get("deps") {
            None => Vec::new(),
            Some(Value::Array(deps)) => deps
                .iter()
                .map(|dep| {
                    dep.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| format!("module {name}: `deps` entry is not a string"))
                })
                .collect::<Result<Vec<_>, String>>()?,
            Some(_) => return Err(format!("module {name}: `deps` is not an array")),
        };
        if !seen.insert(name.to_owned()) {
            return Err(format!("duplicate module name {name}"));
        }
        entries.push(ManifestEntry {
            name: name.to_owned(),
            url: url.to_owned(),
            deps,
        });
    }
    Ok(entries)
}

/// Kahn order over the declared dep edges with SCC tolerance (WPORT-6 D2):
/// the acyclic part loads dependencies before dependants; members of a
/// strongly-connected component load in manifest order, and each such
/// component (size > 1, or a self-edge) is reported in the `cycles` array —
/// a structured diagnosis, not a failure (OQ-B RULED: mutual recursion is
/// legal BEAM reality; deferred imports heal at call time).
fn order_modules(entries: &[ManifestEntry]) -> (Vec<usize>, Vec<Vec<String>>) {
    let index_of: HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.name.as_str(), index))
        .collect();
    // dependant -> dependency adjacency; deps were validated pre-fetch.
    let dep_edges: Vec<Vec<usize>> = entries
        .iter()
        .map(|entry| {
            entry
                .deps
                .iter()
                .filter_map(|dep| index_of.get(dep.as_str()).copied())
                .collect()
        })
        .collect();
    let components = strongly_connected_components(&dep_edges);

    let mut component_of = vec![0usize; entries.len()];
    for (component_index, members) in components.iter().enumerate() {
        for &member in members {
            component_of[member] = component_index;
        }
    }
    let component_count = components.len();
    let mut indegree = vec![0usize; component_count];
    let mut dependants_of: Vec<Vec<usize>> = vec![Vec::new(); component_count];
    let mut self_looped = vec![false; component_count];
    let mut seen_edges: HashSet<(usize, usize)> = HashSet::new();
    for (dependant, deps) in dep_edges.iter().enumerate() {
        for &dependency in deps {
            let from = component_of[dependency];
            let to = component_of[dependant];
            if from == to {
                if dependant == dependency {
                    self_looped[to] = true;
                }
                continue;
            }
            if seen_edges.insert((from, to)) {
                indegree[to] += 1;
                dependants_of[from].push(to);
            }
        }
    }
    // Deterministic Kahn tie-break: among ready components, always emit the
    // one whose earliest member appears first in the manifest.
    let component_rank: Vec<usize> = components
        .iter()
        .map(|members| members.first().copied().unwrap_or(0))
        .collect();
    let mut ready: Vec<usize> = (0..component_count)
        .filter(|&component| indegree[component] == 0)
        .collect();
    let mut order = Vec::with_capacity(entries.len());
    let mut cycles: Vec<Vec<String>> = Vec::new();
    while !ready.is_empty() {
        let mut pick = 0;
        for candidate in 1..ready.len() {
            if component_rank[ready[candidate]] < component_rank[ready[pick]] {
                pick = candidate;
            }
        }
        let component = ready.swap_remove(pick);
        let members = &components[component];
        if members.len() > 1 || self_looped[component] {
            cycles.push(
                members
                    .iter()
                    .map(|&member| entries[member].name.clone())
                    .collect(),
            );
        }
        order.extend(members.iter().copied());
        for &dependant in &dependants_of[component] {
            indegree[dependant] -= 1;
            if indegree[dependant] == 0 {
                ready.push(dependant);
            }
        }
    }
    (order, cycles)
}

/// Tarjan state; components carry manifest indices sorted ascending, so SCC
/// members are already in manifest order.
struct SccState<'a> {
    dep_edges: &'a [Vec<usize>],
    visit_order: Vec<Option<usize>>,
    low: Vec<usize>,
    on_stack: Vec<bool>,
    stack: Vec<usize>,
    next_visit: usize,
    components: Vec<Vec<usize>>,
}

fn strongly_connected_components(dep_edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let node_count = dep_edges.len();
    let mut state = SccState {
        dep_edges,
        visit_order: vec![None; node_count],
        low: vec![0; node_count],
        on_stack: vec![false; node_count],
        stack: Vec::new(),
        next_visit: 0,
        components: Vec::new(),
    };
    for node in 0..node_count {
        if state.visit_order[node].is_none() {
            connect(&mut state, node);
        }
    }
    state.components
}

fn connect(state: &mut SccState<'_>, node: usize) {
    state.visit_order[node] = Some(state.next_visit);
    state.low[node] = state.next_visit;
    state.next_visit += 1;
    state.stack.push(node);
    state.on_stack[node] = true;
    for edge_index in 0..state.dep_edges[node].len() {
        let next_node = state.dep_edges[node][edge_index];
        match state.visit_order[next_node] {
            None => {
                connect(state, next_node);
                state.low[node] = state.low[node].min(state.low[next_node]);
            }
            Some(visited) if state.on_stack[next_node] => {
                state.low[node] = state.low[node].min(visited);
            }
            Some(_) => {}
        }
    }
    if Some(state.low[node]) == state.visit_order[node] {
        let mut component = Vec::new();
        while let Some(member) = state.stack.pop() {
            state.on_stack[member] = false;
            component.push(member);
            if member == node {
                break;
            }
        }
        component.sort_unstable();
        state.components.push(component);
    }
}

/// How one injected-fetch invocation went wrong.
enum FetchIssue {
    /// The fetch threw synchronously or its Promise rejected.
    Rejected(String),
    /// The fetch broke the capability contract (non-thenable return or a
    /// resolution that is neither `ArrayBuffer` nor `Uint8Array`).
    Protocol(String),
}

impl FetchIssue {
    /// Build the batch rejection: contract violations always carry the
    /// `fetch_protocol` slug; rejections carry the stage's fetch-failure slug.
    fn reject(
        self,
        rejected_kind: &str,
        artifact: Option<&str>,
        url: &str,
        stage: &str,
        loaded: &[String],
    ) -> JsValue {
        let (kind, detail) = match self {
            Self::Rejected(detail) => (rejected_kind, detail),
            Self::Protocol(detail) => ("fetch_protocol", detail),
        };
        artifact_load_error(kind, &detail, artifact, Some(url), stage, loaded)
    }
}

/// Call the injected fetch for one URL and await its bytes. Each await here
/// is the fetch Promise's own microtask continuation — the loader's only
/// re-entry mechanism (NO-POLLING).
async fn fetch_bytes(fetch: &Function, url: &str) -> Result<Vec<u8>, FetchIssue> {
    let returned = fetch
        .call1(&JsValue::UNDEFINED, &JsValue::from_str(url))
        .map_err(|thrown| FetchIssue::Rejected(describe_js(&thrown)))?;
    let promise = coerce_thenable(returned)?;
    let resolved = JsFuture::from(promise)
        .await
        .map_err(|rejection| FetchIssue::Rejected(describe_js(&rejection)))?;
    if let Some(bytes) = resolved.dyn_ref::<Uint8Array>() {
        return Ok(bytes.to_vec());
    }
    if let Some(buffer) = resolved.dyn_ref::<js_sys::ArrayBuffer>() {
        return Ok(Uint8Array::new(buffer).to_vec());
    }
    Err(FetchIssue::Protocol(
        "fetch resolved to neither ArrayBuffer nor Uint8Array".to_string(),
    ))
}

/// Accept a real `Promise` directly and assimilate other thenables through
/// `Promise.resolve`; a non-thenable return is a contract violation.
fn coerce_thenable(returned: JsValue) -> Result<Promise, FetchIssue> {
    if returned.has_type::<Promise>() {
        return Ok(returned.unchecked_into());
    }
    let then = returned
        .is_object()
        .then(|| Reflect::get(&returned, &JsValue::from_str("then")).ok())
        .flatten();
    match then {
        Some(then) if then.is_function() => Ok(Promise::resolve(&returned)),
        _ => Err(FetchIssue::Protocol(
            "fetch returned a non-thenable value".to_string(),
        )),
    }
}

/// A manifest-stage `manifest_malformed` rejection (nothing loaded yet).
fn manifest_error(detail: &str, manifest_url: &str) -> JsValue {
    artifact_load_error(
        "manifest_malformed",
        detail,
        None,
        Some(manifest_url),
        STAGE_MANIFEST,
        &[],
    )
}

/// The one named error class (WPORT-6 D6), byte-following the repo's named
/// typed-error precedent: a `js_sys::Error` named `ArtifactLoadError` with a
/// `"{kind}: {detail}"` message and one `data` property holding the JSON
/// string `{"artifact","url","stage","loaded"}` (OQ-D RATIFIED).
fn artifact_load_error(
    kind: &str,
    detail: &str,
    artifact: Option<&str>,
    url: Option<&str>,
    stage: &str,
    loaded: &[String],
) -> JsValue {
    let error = js_sys::Error::new(&format!("{kind}: {detail}"));
    error.set_name("ArtifactLoadError");
    let data = json!({
        "artifact": artifact,
        "url": url,
        "stage": stage,
        "loaded": loaded,
    });
    let _assigned = Reflect::set(
        error.as_ref(),
        &JsValue::from_str("data"),
        &JsValue::from_str(&data.to_string()),
    );
    error.into()
}

/// Map a core `LoadError` to its decode-stage kind slug IN-CRATE (WPORT-6
/// D6; hazard 2 — never parsed back out of a flattened string). The
/// `OldCodeStillRunning`/`UnknownNamespace` variants are structurally
/// unreachable through `load_module_with_origin` on this path (no namespace
/// argument; reload clobbers by design, D9); they keep the slug set closed by
/// mapping to the validation-stage slug with the full detail preserved in the
/// message.
fn load_error_slug(error: &LoadError) -> &'static str {
    match error {
        LoadError::InvalidFormat | LoadError::MissingChunk(_) => "artifact_invalid_format",
        LoadError::DecodeError(_) => "artifact_decode_failed",
        LoadError::ValidationError(_)
        | LoadError::OldCodeStillRunning
        | LoadError::UnknownNamespace { .. } => "artifact_validation_failed",
    }
}

/// Minimal URL resolution, documented in FETCH-MANIFEST.md: a URL containing
/// `://` is absolute; a leading `/` resolves against the manifest URL's
/// scheme+authority; anything else resolves against the manifest URL's
/// directory. Dot segments are passed through, not normalised.
fn resolve_url(manifest_url: &str, url: &str) -> String {
    if url.contains("://") {
        return url.to_string();
    }
    let path_start = manifest_url
        .find("://")
        .map_or(0, |scheme_end| scheme_end + 3);
    if url.starts_with('/') {
        let authority_end = manifest_url[path_start..]
            .find('/')
            .map_or(manifest_url.len(), |offset| path_start + offset);
        return format!("{}{url}", &manifest_url[..authority_end]);
    }
    match manifest_url[path_start..].rfind('/') {
        Some(offset) => format!("{}{url}", &manifest_url[..=path_start + offset]),
        None => format!("{manifest_url}/{url}"),
    }
}

/// Additive `denied` sibling entries (D7): the unresolved-entry shape plus
/// the capability the denied native import would have required.
fn denied_imports_to_json(entries: Vec<DeniedImportEntry>, atom_table: &AtomTable) -> Vec<Value> {
    entries
        .into_iter()
        .map(|entry| {
            json!({
                "module": atom_name(atom_table, entry.module),
                "function": atom_name(atom_table, entry.function),
                "arity": entry.arity,
                "capability": format!("{:?}", entry.capability),
            })
        })
        .collect()
}

/// Resolve an atom to its name with the same fallback shape the existing
/// report vocabulary uses (`unresolved_imports_to_json`).
fn atom_name(atom_table: &AtomTable, atom: Atom) -> String {
    atom_table
        .resolve(atom)
        .map_or_else(|| format!("{atom:?}"), str::to_owned)
}

/// Describe a JS throw/rejection for the `"{kind}: {detail}"` message.
fn describe_js(value: &JsValue) -> String {
    if let Some(error) = value.dyn_ref::<js_sys::Error>() {
        return String::from(error.message());
    }
    value
        .as_string()
        .unwrap_or_else(|| "non-string JavaScript value".to_string())
}

#[cfg(all(test, target_arch = "wasm32"))]
#[path = "artifact_loader_tests.rs"]
mod tests;
