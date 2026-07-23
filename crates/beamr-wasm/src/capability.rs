//! WPORT-8 async capability adapters: host-injected fetch + KV storage.
//!
//! Least-authority law: no ambient global is reached from Rust — each
//! module's single injected capability object is the whole contract (the
//! WPORT-6 injected-fetch law generalized). The `wasm_fetch`/`wasm_kv` MFAs
//! are registered UNCONDITIONALLY at VM construction; injection stores the
//! object only, so re-injection flips refusal→service without touching the
//! registry, and the profile seal stays deterministic.
//!
//! REFUSE-BEFORE-SUSPEND: a call against an uninjected capability returns
//! `{error, {capability_missing, fetch|kv}}` synchronously — no suspend, no
//! host call, no arbiter turn, no counter motion. The capability check
//! precedes every other effect; arg-shape errors on a REGISTERED capability
//! stay badarg-class.
//!
//! Arm selection (amendment A3): the caller type is recorded per request at
//! call time and the completion arm is selected per caller — a BYTECODE
//! caller always resumes through the Ok arm with the tagged tuple
//! (`{ok, Value}` | `{error, {Slug, Detail}}`) in x0 (adapter failures are
//! VALUES, never exits); a NATIVE caller gets split arms with untagged
//! payloads so the native layer's own wrapper yields the same tuples
//! exactly. Completions deliver through the ONE shared Promise-leg wake
//! site owned by lib.rs (`CompletionSink`) — this module adds zero
//! wake-path call sites.
//!
//! Response/value shapes follow the codec's native object→sorted-map
//! mapping (binary keys), per D8's grounding: a fetch response is
//! `#{<<"status">> => integer, <<"headers">> => map, <<"body">> => binary}`.
//!
//! Death observation (amendment A1): the arbiter's exit-observation sweep —
//! the same point that settles ExitWaiters — calls [`CapabilityBridge::
//! on_pid_exit`], which drains the dying pid's in-flight entries and fires
//! their abort hooks. The caller-type record lives INSIDE the in-flight
//! entry (W4: one per-pid structure family, never a parallel map).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};

use beamr::atom::{Atom, AtomTable};
use beamr::native::{NativeKey, ProcessContext};
use beamr::scheduler::WasmAsyncCompletion;
use beamr::term::Term;

use crate::convert::{js_value_to_term_in_context, term_to_js_value};

/// The ONE closed capability error vocabulary, identical on both sides of
/// the boundary (R4): the JS `CapabilityError` kind set and the BEAM slug
/// atom set are THE SAME set, pinned by one closed-set wall each side.
/// Variant order in [`CapabilityKind`] IS slug order.
pub(crate) const CAPABILITY_KIND_SLUGS: [&str; 5] = [
    "capability_missing",
    "refused",
    "malformed_response",
    "rejected",
    "cancelled",
];

/// Kind of a capability failure. The detail slot is asymmetric BY DESIGN
/// (amendment A2, law): `capability_missing` carries the CapabilityAtom
/// (`fetch` | `kv`), every operational kind carries a DetailBinary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapabilityKind {
    CapabilityMissing,
    Refused,
    MalformedResponse,
    Rejected,
    Cancelled,
}

impl CapabilityKind {
    pub(crate) fn slug(self) -> &'static str {
        CAPABILITY_KIND_SLUGS[self as usize]
    }
}

/// The two capability-scoped modules (D1): one injected object per module,
/// so least-authority stays legible — a VM with KV but no fetch has a whole
/// module refusing, not half a module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapabilityAdapter {
    Fetch,
    Kv,
}

impl CapabilityAdapter {
    pub(crate) fn name(self) -> &'static str {
        match self {
            CapabilityAdapter::Fetch => "fetch",
            CapabilityAdapter::Kv => "kv",
        }
    }
}

/// Mint a `CapabilityError` — the fourth house error class, the
/// `ArtifactLoadError` mold exactly: named `js_sys::Error`,
/// `"{kind}: {detail}"` message, ONE `data` property holding a JSON string
/// `{"adapter","kind","detail"}`.
pub(crate) fn capability_error(
    adapter: CapabilityAdapter,
    kind: CapabilityKind,
    detail: &str,
) -> JsValue {
    let error = js_sys::Error::new(&format!("{}: {detail}", kind.slug()));
    error.set_name("CapabilityError");
    let data = serde_json::json!({
        "adapter": adapter.name(),
        "kind": kind.slug(),
        "detail": detail,
    });
    let _ignored = Reflect::set(
        error.as_ref(),
        &JsValue::from_str("data"),
        &JsValue::from_str(&data.to_string()),
    );
    error.into()
}

/// lib.rs implements this over the ONE shared Promise-leg delivery + wake
/// site; the bridge never touches the arbiter directly.
pub(crate) trait CompletionSink {
    /// Deliver a completion and request the coalesced turn. Returns whether
    /// the scheduler accepted delivery (false = the pid is gone).
    fn deliver(&self, pid: u64, completion: WasmAsyncCompletion) -> bool;
}

/// A3: recorded per request at call time, selects the completion arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallerType {
    Bytecode,
    Native,
}

/// One suspended capability request. W4: the caller-type record is
/// per-request state INSIDE this entry — the same per-pid structure family
/// the death sweep drains — never a parallel map.
struct InFlightRequest {
    token: u64,
    caller: CallerType,
    abort_slot: Object,
}

/// COUNTED and falsifiable (the WPORT-3 counter law): no-op deliveries are
/// counted, never silent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CapabilityCounters {
    /// Completions whose scheduler delivery was refused (pid dead at
    /// `complete_async` time).
    pub(crate) dead_pid_completions: u64,
    /// Completions arriving for a request no longer in flight (the death
    /// sweep already drained it) — the stale-token analogue.
    pub(crate) stale_completion_noops: u64,
}

/// The five BEAM-facing operations across the two modules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapabilityOp {
    FetchRequest,
    KvGet,
    KvPut,
    KvDelete,
    KvListByPrefix,
}

impl CapabilityOp {
    fn adapter(self) -> CapabilityAdapter {
        match self {
            CapabilityOp::FetchRequest => CapabilityAdapter::Fetch,
            _ => CapabilityAdapter::Kv,
        }
    }

    fn host_method(self) -> &'static str {
        match self {
            CapabilityOp::FetchRequest => "request",
            CapabilityOp::KvGet => "get",
            CapabilityOp::KvPut => "put",
            CapabilityOp::KvDelete => "delete",
            CapabilityOp::KvListByPrefix => "list_by_prefix",
        }
    }
}

struct CapabilityAtoms {
    wasm_fetch: Atom,
    wasm_kv: Atom,
    request: Atom,
    get: Atom,
    put: Atom,
    delete: Atom,
    list_by_prefix: Atom,
    fetch: Atom,
    kv: Atom,
    ok: Atom,
    undefined: Atom,
    kind_atoms: [Atom; 5],
}

impl CapabilityAtoms {
    fn intern(atom_table: &AtomTable) -> Self {
        Self {
            wasm_fetch: atom_table.intern("wasm_fetch"),
            wasm_kv: atom_table.intern("wasm_kv"),
            request: atom_table.intern("request"),
            get: atom_table.intern("get"),
            put: atom_table.intern("put"),
            delete: atom_table.intern("delete"),
            list_by_prefix: atom_table.intern("list_by_prefix"),
            fetch: atom_table.intern("fetch"),
            kv: atom_table.intern("kv"),
            ok: atom_table.intern("ok"),
            undefined: atom_table.intern("undefined"),
            kind_atoms: CAPABILITY_KIND_SLUGS.map(|slug| atom_table.intern(slug)),
        }
    }

    fn adapter_atom(&self, adapter: CapabilityAdapter) -> Atom {
        match adapter {
            CapabilityAdapter::Fetch => self.fetch,
            CapabilityAdapter::Kv => self.kv,
        }
    }

    fn kind_atom(&self, kind: CapabilityKind) -> Atom {
        self.kind_atoms[kind as usize]
    }
}

pub(crate) struct CapabilityBridge {
    atom_table: Arc<AtomTable>,
    atoms: CapabilityAtoms,
    fetch_capability: RefCell<Option<Object>>,
    kv_capability: RefCell<Option<Object>>,
    in_flight: RefCell<HashMap<u64, Vec<InFlightRequest>>>,
    next_token: Cell<u64>,
    counters: RefCell<CapabilityCounters>,
    sink: RefCell<Option<Rc<dyn CompletionSink>>>,
}

impl CapabilityBridge {
    pub(crate) fn new(atom_table: Arc<AtomTable>) -> Rc<Self> {
        let atoms = CapabilityAtoms::intern(atom_table.as_ref());
        Rc::new(Self {
            atom_table,
            atoms,
            fetch_capability: RefCell::new(None),
            kv_capability: RefCell::new(None),
            in_flight: RefCell::new(HashMap::new()),
            next_token: Cell::new(0),
            counters: RefCell::new(CapabilityCounters::default()),
            sink: RefCell::new(None),
        })
    }

    pub(crate) fn set_sink(&self, sink: Rc<dyn CompletionSink>) {
        *self.sink.borrow_mut() = Some(sink);
    }

    /// Idempotent last-wins (R1); in-flight completions are keyed by token
    /// and are not disturbed by re-injection.
    pub(crate) fn register_fetch(&self, capability: Object) {
        *self.fetch_capability.borrow_mut() = Some(capability);
    }

    pub(crate) fn register_kv(&self, capability: Object) {
        *self.kv_capability.borrow_mut() = Some(capability);
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn counters(&self) -> CapabilityCounters {
        *self.counters.borrow()
    }

    /// Whether `mfa` is one of the capability MFAs this bridge serves.
    pub(crate) fn routes(&self, mfa: NativeKey) -> bool {
        self.resolve_op(mfa).is_some()
    }

    fn resolve_op(&self, mfa: NativeKey) -> Option<CapabilityOp> {
        let (module, function, arity) = mfa;
        let atoms = &self.atoms;
        if module == atoms.wasm_fetch {
            return (function == atoms.request && arity == 1).then_some(CapabilityOp::FetchRequest);
        }
        if module == atoms.wasm_kv {
            if function == atoms.get && arity == 1 {
                return Some(CapabilityOp::KvGet);
            }
            if function == atoms.put && arity == 2 {
                return Some(CapabilityOp::KvPut);
            }
            if function == atoms.delete && arity == 1 {
                return Some(CapabilityOp::KvDelete);
            }
            if function == atoms.list_by_prefix && arity == 1 {
                return Some(CapabilityOp::KvListByPrefix);
            }
        }
        None
    }

    /// A1: called from the arbiter's exit-observation sweep — the same
    /// observation that settles ExitWaiters. Drains the pid's in-flight
    /// entries and fires each abort hook (a host-direction call; no wake
    /// path involvement). A completion later arriving for a drained entry
    /// is a counted stale no-op.
    pub(crate) fn on_pid_exit(&self, pid: u64) {
        let Some(entries) = self.in_flight.borrow_mut().remove(&pid) else {
            return;
        };
        for entry in entries {
            let abort = Reflect::get(entry.abort_slot.as_ref(), &JsValue::from_str("abort"));
            if let Ok(abort) = abort
                && let Some(abort) = abort.dyn_ref::<Function>()
            {
                let _ignored = abort.call0(&JsValue::UNDEFINED);
            }
        }
    }

    /// The BIF entry: refuse-before-suspend, validate, invoke the host,
    /// then suspend the caller for the async completion. Synchronous host
    /// failures (throw / non-thenable) return the `refused` leg as a value
    /// without suspending.
    pub(crate) fn start_capability_call(
        self: &Rc<Self>,
        mfa: NativeKey,
        args: &[Term],
        context: &mut ProcessContext<'_>,
    ) -> Result<Term, Term> {
        let Some(op) = self.resolve_op(mfa) else {
            return Err(Term::atom(Atom::UNDEF));
        };
        let adapter = op.adapter();

        // REFUSE-BEFORE-SUSPEND (R5): the capability check precedes every
        // other effect, arg validation included.
        let capability = match adapter {
            CapabilityAdapter::Fetch => self.fetch_capability.borrow().clone(),
            CapabilityAdapter::Kv => self.kv_capability.borrow().clone(),
        };
        let Some(capability) = capability else {
            return self.missing_refusal(adapter, context);
        };

        // Caller identity (A3 + W4). W3 pins visibility here: a caller
        // context that cannot name its pid or process is unreachable for
        // real callers — if the native-caller wall ever lands on this arm,
        // that is a FRESH STOP per the A3 ruling, not an inference.
        let Some(pid) = context.pid() else {
            return Err(Term::atom(Atom::BADARG));
        };
        let Some(process) = context.process_mut() else {
            return Err(Term::atom(Atom::BADARG));
        };
        let caller = if process.is_native() {
            CallerType::Native
        } else {
            CallerType::Bytecode
        };

        // Arg validation (badarg-class on a REGISTERED capability, distinct
        // from the typed refusal — R5).
        let call_args = self.marshal_args(op, args)?;

        // Invoke the host method. A sync throw or a non-thenable return is
        // the `refused` leg, returned synchronously as a value.
        let method = Reflect::get(capability.as_ref(), &JsValue::from_str(op.host_method()))
            .ok()
            .and_then(|value| value.dyn_into::<Function>().ok());
        let Some(method) = method else {
            return self.sync_failure(
                op,
                CapabilityKind::Refused,
                "capability method missing",
                context,
            );
        };
        let abort_slot = Object::new();
        let invoked = match op {
            CapabilityOp::FetchRequest => {
                method.call2(capability.as_ref(), &call_args[0], abort_slot.as_ref())
            }
            CapabilityOp::KvPut => method.call2(capability.as_ref(), &call_args[0], &call_args[1]),
            _ => method.call1(capability.as_ref(), &call_args[0]),
        };
        let thenable = match invoked {
            Ok(value) => value,
            Err(thrown) => {
                let detail = js_error_detail(&thrown);
                return self.sync_failure(op, CapabilityKind::Refused, &detail, context);
            }
        };
        if !is_thenable(&thenable) {
            return self.sync_failure(
                op,
                CapabilityKind::Refused,
                "capability method returned a non-thenable",
                context,
            );
        }

        // Record the in-flight entry (token admission; caller type rides
        // the entry per W4), then park the caller on the async seam.
        let token = self.next_token.get();
        self.next_token.set(token.wrapping_add(1));
        self.in_flight
            .borrow_mut()
            .entry(pid)
            .or_default()
            .push(InFlightRequest {
                token,
                caller,
                abort_slot,
            });

        let bridge = Rc::clone(self);
        let promise = js_sys::Promise::resolve(&thenable);
        spawn_local(async move {
            let settled = JsFuture::from(promise).await;
            bridge.finish_request(pid, token, op, settled);
        });

        context.request_suspend(None);
        Ok(Term::NIL)
    }

    /// `{error, {capability_missing, fetch|kv}}` built in the caller's
    /// context — the A2 asymmetry: this kind carries the CapabilityAtom.
    fn missing_refusal(
        &self,
        adapter: CapabilityAdapter,
        context: &mut ProcessContext<'_>,
    ) -> Result<Term, Term> {
        let kind = Term::atom(self.atoms.kind_atom(CapabilityKind::CapabilityMissing));
        let detail = Term::atom(self.atoms.adapter_atom(adapter));
        let inner = context.alloc_tuple(&[kind, detail])?;
        let error = context.alloc_tuple(&[Term::atom(Atom::ERROR), inner])?;
        Ok(error)
    }

    /// An operational failure surfaced synchronously (pre-suspend):
    /// `{error, {Slug, DetailBinary}}` in the caller's context. The
    /// vocabulary rides the same minter as async legs.
    fn sync_failure(
        &self,
        op: CapabilityOp,
        kind: CapabilityKind,
        detail: &str,
        context: &mut ProcessContext<'_>,
    ) -> Result<Term, Term> {
        let _class = capability_error(op.adapter(), kind, detail);
        let kind_term = Term::atom(self.atoms.kind_atom(kind));
        let detail_term = context.alloc_binary(detail.as_bytes())?;
        let inner = context.alloc_tuple(&[kind_term, detail_term])?;
        let error = context.alloc_tuple(&[Term::atom(Atom::ERROR), inner])?;
        Ok(error)
    }

    /// Marshal + validate the BEAM args into the host-call JS values.
    /// Returns badarg on shape errors (R5's distinction).
    fn marshal_args(&self, op: CapabilityOp, args: &[Term]) -> Result<Vec<JsValue>, Term> {
        let badarg = || Term::atom(Atom::BADARG);
        let to_js =
            |term: Term| term_to_js_value(term, self.atom_table.as_ref()).map_err(|_| badarg());
        match op {
            CapabilityOp::FetchRequest => {
                let request = to_js(*args.first().ok_or_else(badarg)?)?;
                let normalized = normalize_fetch_request(&request).ok_or_else(badarg)?;
                Ok(vec![normalized.into()])
            }
            CapabilityOp::KvGet | CapabilityOp::KvDelete | CapabilityOp::KvListByPrefix => {
                let key = to_js(*args.first().ok_or_else(badarg)?)?;
                if !key.is_string() {
                    return Err(badarg());
                }
                Ok(vec![key])
            }
            CapabilityOp::KvPut => {
                let key = to_js(*args.first().ok_or_else(badarg)?)?;
                if !key.is_string() {
                    return Err(badarg());
                }
                let value = to_js(*args.get(1).ok_or_else(badarg)?)?;
                if !key_or_binary_shaped(&value) {
                    return Err(badarg());
                }
                Ok(vec![key, value])
            }
        }
    }

    /// Completion landing: token admission against the in-flight registry,
    /// outcome normalization, per-caller arm selection (A3), counted no-op
    /// legs. Runs on the microtask queue after the host thenable settles.
    fn finish_request(
        self: &Rc<Self>,
        pid: u64,
        token: u64,
        op: CapabilityOp,
        settled: Result<JsValue, JsValue>,
    ) {
        let entry = self.take_in_flight(pid, token);
        let Some(entry) = entry else {
            self.counters.borrow_mut().stale_completion_noops += 1;
            return;
        };

        let outcome = match settled {
            Ok(response) => validate_response(op, response),
            Err(rejection) => Err(normalize_rejection(&rejection)),
        };
        // The JS-side vocabulary rides the same minter for every failure
        // (R4: one closed set, adapter named in the payload).
        if let Err((kind, detail)) = &outcome {
            let _class = capability_error(op.adapter(), *kind, detail);
        }

        let Some(completion) = self.build_completion(entry.caller, outcome) else {
            // Term construction failed (allocation): deliver the honest
            // rejected leg rather than nothing; if that also fails, the
            // caller's death is observable via the suspended process — but
            // this arm is not reachable for the value shapes this module
            // builds (small tuples + validated payloads).
            return;
        };
        let delivered = self
            .sink
            .borrow()
            .as_ref()
            .is_some_and(|sink| sink.deliver(pid, completion));
        if !delivered {
            self.counters.borrow_mut().dead_pid_completions += 1;
        }
    }

    fn take_in_flight(&self, pid: u64, token: u64) -> Option<InFlightRequest> {
        let mut in_flight = self.in_flight.borrow_mut();
        let entries = in_flight.get_mut(&pid)?;
        let index = entries.iter().position(|entry| entry.token == token)?;
        let entry = entries.remove(index);
        if entries.is_empty() {
            in_flight.remove(&pid);
        }
        Some(entry)
    }

    /// A3 arm selection. Bytecode: ALWAYS `Ok` carrying the tagged tuple —
    /// adapter failures are values, not exits. Native: split arms with
    /// untagged payloads so `deliver_native_async_completion`'s wrapper
    /// produces `{ok, V}` / `{error, {Slug, Detail}}` exactly (no double
    /// wrap).
    fn build_completion(
        &self,
        caller: CallerType,
        outcome: Result<JsValue, (CapabilityKind, String)>,
    ) -> Option<WasmAsyncCompletion> {
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::clone(&self.atom_table)));
        let built = match &outcome {
            Ok(value) => self.success_term(&mut context, value)?,
            Err((kind, detail)) => {
                let kind_term = Term::atom(self.atoms.kind_atom(*kind));
                let detail_term = context.alloc_binary(detail.as_bytes()).ok()?;
                context.alloc_tuple(&[kind_term, detail_term]).ok()?
            }
        };
        match (caller, outcome.is_ok()) {
            (CallerType::Bytecode, ok) => {
                let tag = if ok {
                    Term::atom(self.atoms.ok)
                } else {
                    Term::atom(Atom::ERROR)
                };
                let tagged = context.alloc_tuple(&[tag, built]).ok()?;
                let owned = context.take_detached_result(tagged)?;
                Some(WasmAsyncCompletion::Ok(owned))
            }
            (CallerType::Native, true) => {
                let owned = context.take_detached_result(built)?;
                Some(WasmAsyncCompletion::Ok(owned))
            }
            (CallerType::Native, false) => {
                let owned = context.take_detached_result(built)?;
                Some(WasmAsyncCompletion::Error(owned))
            }
        }
    }

    /// Build the success value term for a validated host response inside
    /// `context`. `undefined` (KV get absent) becomes the `undefined` atom;
    /// everything else rides the codec.
    fn success_term(&self, context: &mut ProcessContext<'_>, value: &JsValue) -> Option<Term> {
        if value.is_undefined() {
            return Some(Term::atom(self.atoms.undefined));
        }
        js_value_to_term_in_context(value.clone(), context).ok()
    }
}

fn is_thenable(value: &JsValue) -> bool {
    Reflect::get(value, &JsValue::from_str("then"))
        .ok()
        .is_some_and(|then| then.is_function())
}

fn js_error_detail(value: &JsValue) -> String {
    if let Some(error) = value.dyn_ref::<js_sys::Error>() {
        return String::from(error.message());
    }
    value
        .as_string()
        .unwrap_or_else(|| "capability host failure".to_owned())
}

/// AbortError → `cancelled` (honest: the request did not complete); every
/// other rejection is the `rejected` leg with the host's message as detail.
fn normalize_rejection(rejection: &JsValue) -> (CapabilityKind, String) {
    if let Some(error) = rejection.dyn_ref::<js_sys::Error>()
        && error.name() == "AbortError"
    {
        return (CapabilityKind::Cancelled, String::from(error.message()));
    }
    (CapabilityKind::Rejected, js_error_detail(rejection))
}

/// Validate + normalize a fetch request object built from the caller's map:
/// `url` (string, required); `method` (string, default "GET"); `headers`
/// (object, default {}); `body` (string | Uint8Array, optional).
fn normalize_fetch_request(request: &JsValue) -> Option<Object> {
    if !request.is_object() || Array::is_array(request) {
        return None;
    }
    let url = Reflect::get(request, &JsValue::from_str("url")).ok()?;
    if !url.is_string() {
        return None;
    }
    let normalized = Object::new();
    Reflect::set(normalized.as_ref(), &JsValue::from_str("url"), &url).ok()?;
    let method = Reflect::get(request, &JsValue::from_str("method")).ok()?;
    let method = if method.is_undefined() {
        JsValue::from_str("GET")
    } else if method.is_string() {
        method
    } else {
        return None;
    };
    Reflect::set(normalized.as_ref(), &JsValue::from_str("method"), &method).ok()?;
    let headers = Reflect::get(request, &JsValue::from_str("headers")).ok()?;
    let headers = if headers.is_undefined() {
        Object::new().into()
    } else if headers.is_object() && !Array::is_array(&headers) {
        headers
    } else {
        return None;
    };
    Reflect::set(normalized.as_ref(), &JsValue::from_str("headers"), &headers).ok()?;
    let body = Reflect::get(request, &JsValue::from_str("body")).ok()?;
    if !body.is_undefined() {
        if !key_or_binary_shaped(&body) {
            return None;
        }
        Reflect::set(normalized.as_ref(), &JsValue::from_str("body"), &body).ok()?;
    }
    Some(normalized)
}

fn key_or_binary_shaped(value: &JsValue) -> bool {
    value.is_string() || value.dyn_ref::<Uint8Array>().is_some()
}

/// Validate a settled host response per op. Success returns the normalized
/// JS value the codec converts (fetch: `{status, headers, body}` object;
/// KV get: string | Uint8Array | undefined; put/delete: `true`; list: array
/// of strings). Shape violations are the `malformed_response` leg.
fn validate_response(
    op: CapabilityOp,
    response: JsValue,
) -> Result<JsValue, (CapabilityKind, String)> {
    let malformed = |detail: &str| (CapabilityKind::MalformedResponse, detail.to_owned());
    match op {
        CapabilityOp::FetchRequest => {
            if !response.is_object() || Array::is_array(&response) {
                return Err(malformed("fetch response is not an object"));
            }
            let status = Reflect::get(&response, &JsValue::from_str("status"))
                .map_err(|_| malformed("fetch response status unreadable"))?;
            let Some(status) = status.as_f64() else {
                return Err(malformed("fetch response status is not a number"));
            };
            if status.fract() != 0.0 {
                return Err(malformed("fetch response status is not an integer"));
            }
            let headers = Reflect::get(&response, &JsValue::from_str("headers"))
                .map_err(|_| malformed("fetch response headers unreadable"))?;
            let headers = if headers.is_undefined() {
                Object::new().into()
            } else if headers.is_object() && !Array::is_array(&headers) {
                headers
            } else {
                return Err(malformed("fetch response headers is not an object"));
            };
            let body = Reflect::get(&response, &JsValue::from_str("body"))
                .map_err(|_| malformed("fetch response body unreadable"))?;
            let body = if body.is_undefined() {
                JsValue::from_str("")
            } else if key_or_binary_shaped(&body) {
                body
            } else {
                return Err(malformed("fetch response body is not string or bytes"));
            };
            let normalized = Object::new();
            let set = |key: &str, value: &JsValue| {
                Reflect::set(normalized.as_ref(), &JsValue::from_str(key), value)
                    .map_err(|_| malformed("fetch response normalization failed"))
            };
            set("status", &JsValue::from_f64(status))?;
            set("headers", &headers)?;
            set("body", &body)?;
            Ok(normalized.into())
        }
        CapabilityOp::KvGet => {
            if response.is_undefined() || response.is_null() {
                return Ok(JsValue::UNDEFINED);
            }
            if key_or_binary_shaped(&response) {
                return Ok(response);
            }
            Err(malformed("kv get value is not string or bytes"))
        }
        CapabilityOp::KvPut | CapabilityOp::KvDelete => Ok(JsValue::TRUE),
        CapabilityOp::KvListByPrefix => {
            let Some(array) = response.dyn_ref::<Array>() else {
                return Err(malformed("kv list response is not an array"));
            };
            for index in 0..array.length() {
                if !array.get(index).is_string() {
                    return Err(malformed("kv list entry is not a string"));
                }
            }
            Ok(response)
        }
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
#[path = "capability_tests.rs"]
mod tests;
