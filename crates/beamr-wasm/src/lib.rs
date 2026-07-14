//! JavaScript bindings for the cooperative Beamr WASM runtime.

mod convert;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use beamr::atom::AtomTable;
use beamr::loader::{UnresolvedImport, load_module_with_origin};
use beamr::module::{ModuleOrigin, ModuleRegistry};
use beamr::native::bifs::register_gate1_bifs;

use beamr::ets::OwnedTerm;
use beamr::native::process_bifs::register_gate2_bifs;
use beamr::native::stdlib_stubs::register_stdlib_stubs;
use beamr::native::{
    BifRegistryImpl, Capability, NativeKey, NativeRegistrationError, WasmAsyncNifFacility,
};
use beamr::scheduler::{WasmAsyncCompletion, WasmRunState, WasmRunSummary, WasmScheduler};
use beamr::term::json::term_to_value;
use beamr::term::{Term, format::format_term};
use beamr::{CoopSenderHandle, DynActor, ReplyFn, WireTerm, spawn_actor_cooperative};
use convert::{
    js_value_to_owned_term, js_value_to_term_in_context, term_to_js_value, terms_from_json_array,
    terms_to_js_array,
};
use js_sys::{Function, Promise, Reflect};
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// Construct a new Beamr VM handle for JavaScript hosts.
#[wasm_bindgen]
pub fn create_vm() -> Result<WasmVm, JsValue> {
    WasmVm::new()
}

/// A single-node Beamr VM driven cooperatively by JavaScript.
#[wasm_bindgen]
pub struct WasmVm {
    atom_table: Arc<AtomTable>,
    module_registry: Arc<ModuleRegistry>,
    bif_registry: Arc<BifRegistryImpl>,
    scheduler: Rc<RefCell<WasmScheduler>>,
    arbiter: Rc<HostArbiter>,
    timer_handles: Rc<RefCell<BTreeMap<u64, HostTimer>>>,
    async_bridge: Rc<HostAsyncNifs>,
    js_callbacks: Rc<HostJsCallbacks>,
    actor_handlers: Rc<HostActorHandlers>,
}

#[wasm_bindgen]
impl WasmVm {
    /// Create a VM with common atoms and wasm-safe BIF registrations.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<WasmVm, JsValue> {
        let primitives = HostPrimitives::probe()?;
        let atom_table = Arc::new(AtomTable::with_common_atoms());
        let module_registry = Arc::new(ModuleRegistry::new());
        let bif_registry = Arc::new(BifRegistryImpl::new());
        register_wasm_safe_bifs(&bif_registry, &atom_table).map_err(registration_error_to_js)?;
        let scheduler = Rc::new(RefCell::new(WasmScheduler::new(
            Arc::clone(&atom_table),
            Arc::clone(&module_registry),
            Arc::clone(&bif_registry),
        )));
        let timer_handles = Rc::new(RefCell::new(BTreeMap::new()));
        let arbiter = HostArbiter::new(
            primitives,
            Arc::clone(&atom_table),
            Rc::clone(&scheduler),
            Rc::clone(&timer_handles),
        );
        let async_bridge = Rc::new(HostAsyncNifs::new(
            Arc::clone(&atom_table),
            Rc::downgrade(&scheduler),
            Rc::downgrade(&arbiter),
        ));
        let js_callbacks = Rc::new(HostJsCallbacks::new(
            Arc::clone(&atom_table),
            Rc::downgrade(&scheduler),
            Rc::downgrade(&arbiter),
        ));
        let facility: Rc<dyn WasmAsyncNifFacility> = Rc::new(HostWasmFacility {
            async_nifs: Rc::clone(&async_bridge),
            js_callbacks: Rc::clone(&js_callbacks),
            js_callback_module: atom_table.intern("wasm_ffi"),
            js_callback_function: atom_table.intern("js_callback"),
        });
        scheduler
            .borrow_mut()
            .set_wasm_async_nif_facility(Some(facility));
        let actor_handlers = Rc::new(HostActorHandlers::new());
        Ok(Self {
            atom_table,
            module_registry,
            bif_registry,
            scheduler,
            arbiter,
            timer_handles,
            async_bridge,
            js_callbacks,
            actor_handlers,
        })
    }

    /// Load a caller-provided `.beam` module byte buffer.
    pub fn load_module(&mut self, bytes: &[u8]) -> Result<JsValue, JsValue> {
        let (module, unresolved) = load_module_with_origin(
            bytes,
            self.atom_table.as_ref(),
            self.module_registry.as_ref(),
            self.bif_registry.as_ref(),
            ModuleOrigin::Preloaded,
        )
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let unresolved = unresolved_imports_to_json(unresolved.imports(), self.atom_table.as_ref());
        let result = json!({
            "ok": true,
            "module": self.atom_table.resolve(module.name).unwrap_or("#<unknown>"),
            "unresolved": unresolved,
        });
        json_to_js(&result)
    }

    /// Send a JavaScript value to a BEAM process mailbox by local PID.
    pub fn send_message(&mut self, pid: u64, value: JsValue) -> Result<(), JsValue> {
        let message = js_value_to_owned_term(value, &self.atom_table)?;
        self.scheduler
            .borrow_mut()
            .send_owned(pid, &message)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        self.sync_host_timers()?;
        self.schedule_external_edge()?;
        Ok(())
    }

    /// Register a JavaScript function for `wasm_ffi:js_callback/{N}` calls.
    pub fn register_js_callback(&mut self, name: &str, callback: Function) {
        self.js_callbacks.register(name, callback);
    }

    /// Register `wasm_ffi:js_callback/Arity` for a previously registered JS callback.
    ///
    /// The BEAM call shape is `wasm_ffi:js_callback(Name, Arg1, ..., ArgN)`, so
    /// the registered native arity must include the leading callback name.
    pub fn register_js_callback_nif(&mut self, arity: u8) -> Result<(), JsValue> {
        let module_atom = self.atom_table.intern("wasm_ffi");
        let function_atom = self.atom_table.intern("js_callback");
        self.bif_registry
            .register(
                module_atom,
                function_atom,
                arity,
                js_callback_nif,
                Capability::ExternalIo,
            )
            .map_err(registration_error_to_js)
    }

    /// Register a JavaScript Promise-returning native under module/function/arity.
    pub fn register_async_nif(
        &mut self,
        module: &str,
        function: &str,
        arity: u8,
        callback: Function,
    ) -> Result<(), JsValue> {
        let module_atom = self.atom_table.intern(module);
        let function_atom = self.atom_table.intern(function);
        self.async_bridge
            .register((module_atom, function_atom, arity), callback);
        self.bif_registry
            .register(
                module_atom,
                function_atom,
                arity,
                wasm_async_nif_stub,
                Capability::ExternalIo,
            )
            .map_err(registration_error_to_js)
    }

    /// Spawn an exported function. Arguments are encoded as a JSON array string.
    pub fn spawn(&mut self, module: &str, function: &str, args_json: &str) -> Result<u64, JsValue> {
        let args_value: Value = serde_json::from_str(args_json)
            .map_err(|error| JsValue::from_str(&format!("invalid args JSON: {error}")))?;
        let args = self.json_args_to_terms(&args_value)?;
        let module = self.atom_table.intern(module);
        let function = self.atom_table.intern(function);
        let pid = self
            .scheduler
            .borrow_mut()
            .spawn_owned(module, function, args)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        self.schedule_external_edge()?;
        Ok(pid)
    }

    /// Run one bounded cooperative drain and return its complete JSON result.
    pub fn run_step(&mut self) -> Result<JsValue, JsValue> {
        let value = self.arbiter.run_manual_drain()?;
        json_to_js(&value)
    }

    /// Await target exit/error, or settled idle when no receive one-shot remains armed.
    pub fn await_exit(&mut self, pid: u64) -> Promise {
        self.arbiter.await_exit(pid)
    }

    /// Consume and return the captured exit value for `pid`, if that process has exited.
    ///
    /// Hosts that serve many independent requests should prefer this over repeatedly
    /// scanning `run_step().results`, because it releases the scheduler's retained
    /// copy of the process result once the host has converted it.
    pub fn take_exit_result(&mut self, pid: u64) -> Result<JsValue, JsValue> {
        let result = { self.scheduler.borrow_mut().take_exit_result(pid) };
        let value = result
            .map(|term| self.term_to_json_or_fallback(term.root()))
            .unwrap_or(Value::Null);
        json_to_js(&value)
    }

    /// Spawn a cooperative actor whose request/reply logic is a JavaScript
    /// function, returning its `u64` pid.
    ///
    /// `handler` is `reply = handler(request)`: the VM marshals each inbound
    /// request term to a `JsValue` (the term codec), calls `handler`, and marshals
    /// the returned value back to a reply term. The actor is a first-class beamr
    /// process (pid, mailbox, supervision) driven by the cooperative `call_async`
    /// surface, so [`WasmVm::call`] returns a real `Promise` over its reply. The
    /// handler must return synchronously (it computes a value, not a `Promise`);
    /// host *async* work belongs on the async-NIF seam ([`WasmVm::register_async_nif`]).
    ///
    /// The handler runs on the host thread during a pumped turn, so it stays alive
    /// for the actor's lifetime in a per-VM registry rather than crossing the
    /// `Send` actor boundary (a JS `Function` is `!Send`); the actor carries only a
    /// small registry id.
    pub fn spawn_actor(&mut self, handler: Function) -> u64 {
        let handler_id = self.actor_handlers.register(handler);
        let atom_table = Arc::clone(&self.atom_table);
        let reply: ReplyFn = Arc::new(move |request: &OwnedTerm| {
            invoke_actor_handler(handler_id, request, &atom_table)
        });
        let actor = spawn_actor_cooperative::<DynActor, _>(&self.scheduler, move || {
            DynActor::new(Arc::clone(&reply))
        });
        self.schedule_external_edge_infallible();
        actor.pid
    }

    /// Send `request` to an actor by pid and return a `Promise` that resolves with
    /// the actor's reply value (or rejects on timeout / a marshalling failure).
    ///
    /// The request value is marshalled to a term, sent through the cooperative
    /// `call_async` path (ref-correlated, so concurrent calls never cross
    /// replies), and the resulting `CallFuture` is wrapped as a JS `Promise`.
    /// The transient client spawn requests the VM's edge-triggered arbiter turn.
    pub fn call(&mut self, pid: u64, request: JsValue) -> Result<Promise, JsValue> {
        let owned = js_value_to_owned_term(request, &self.atom_table)?;
        let handle = CoopSenderHandle::<DynActor>::attach(&self.scheduler, pid);
        let future = handle.call_async(WireTerm::new(owned));
        let atom_table = Arc::clone(&self.atom_table);
        let promise = wasm_bindgen_futures::future_to_promise(async move {
            match future.await {
                Ok(reply) => term_to_js_value(reply.owned().root(), atom_table.as_ref()),
                Err(error) => Err(JsValue::from_str(&error.to_string())),
            }
        });
        self.schedule_external_edge()?;
        Ok(promise)
    }

    /// Send a fire-and-forget message to an actor by pid (non-blocking).
    ///
    /// The value is marshalled to a term and cast through the cooperative path; it
    /// reaches the actor's cast handler on a later arbiter turn. A cast to a dead
    /// pid is silently dropped, exactly like a BEAM send.
    pub fn cast(&mut self, pid: u64, message: JsValue) -> Result<(), JsValue> {
        let owned = js_value_to_owned_term(message, &self.atom_table)?;
        let handle = CoopSenderHandle::<DynActor>::attach(&self.scheduler, pid);
        handle
            .cast(WireTerm::new(owned))
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        self.schedule_external_edge()?;
        Ok(())
    }

    /// Called by tests or custom hosts to drive an already-fired timer manually.
    pub fn timer_fired(&mut self, pid: u64, timer_id: u64) -> Result<bool, JsValue> {
        if self.timer_handles.borrow_mut().remove(&timer_id).is_some() {
            self.arbiter.record_deadline_execution();
        }
        let fired = self.scheduler.borrow_mut().timer_fired(pid, timer_id);
        self.sync_host_timers()?;
        self.schedule_external_edge()?;
        Ok(fired)
    }

    fn schedule_external_edge(&self) -> Result<(), JsValue> {
        let edge = self.scheduler.borrow_mut().take_external_runnable_edge();
        if edge {
            self.arbiter.request_external_turn()?;
        }
        Ok(())
    }

    fn schedule_external_edge_infallible(&self) {
        if let Err(error) = self.schedule_external_edge() {
            self.arbiter.fail(error);
        }
    }

    fn json_args_to_terms(&self, value: &Value) -> Result<Vec<beamr::ets::OwnedTerm>, JsValue> {
        terms_from_json_array(value, &self.atom_table)
    }

    fn term_to_json_or_fallback(&self, term: Term) -> Value {
        term_to_json_or_fallback(term, self.atom_table.as_ref())
    }

    fn sync_host_timers(&mut self) -> Result<(), JsValue> {
        sync_host_timers_inner(
            &self.scheduler,
            &self.timer_handles,
            Rc::downgrade(&self.arbiter),
        )
    }
}

#[derive(Clone)]
struct HostPrimitives {
    global: JsValue,
    queue_microtask: Function,
    set_timeout: Function,
}

impl HostPrimitives {
    fn probe() -> Result<Self, JsValue> {
        let global = js_sys::global();
        let queue_microtask = required_host_function(&global, "queueMicrotask")?;
        let set_timeout = required_host_function(&global, "setTimeout")?;
        Ok(Self {
            global: global.into(),
            queue_microtask,
            set_timeout,
        })
    }
}

fn required_host_function(global: &JsValue, name: &str) -> Result<Function, JsValue> {
    let value = Reflect::get(global, &JsValue::from_str(name)).map_err(|_| {
        JsValue::from_str(&format!("required host primitive {name} is unavailable"))
    })?;
    value.dyn_into::<Function>().map_err(|_| {
        JsValue::from_str(&format!("required host primitive {name} is not a function"))
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArbiterState {
    Idle,
    Queued,
    Draining,
}

#[derive(Clone, Copy)]
enum HostTurnLeg {
    ExternalMicrotask,
    FairnessMacrotask,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CallbackCounters {
    requests: u64,
    queued_now: usize,
    executions: u64,
    cancellations: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ArbiterInstrumentation {
    arbiter: CallbackCounters,
    receive_timers: CallbackCounters,
}

struct ExitWaiter {
    resolve: Function,
    reject: Function,
}

struct HostArbiter {
    primitives: HostPrimitives,
    callback: Function,
    atom_table: Arc<AtomTable>,
    scheduler: Rc<RefCell<WasmScheduler>>,
    timer_handles: Rc<RefCell<BTreeMap<u64, HostTimer>>>,
    state: Cell<ArbiterState>,
    ignored_callbacks: Cell<usize>,
    waiters: RefCell<BTreeMap<u64, Vec<ExitWaiter>>>,
    last_summary: RefCell<Value>,
    last_error: RefCell<Option<JsValue>>,
    instrumentation: RefCell<ArbiterInstrumentation>,
}

impl HostArbiter {
    fn new(
        primitives: HostPrimitives,
        atom_table: Arc<AtomTable>,
        scheduler: Rc<RefCell<WasmScheduler>>,
        timer_handles: Rc<RefCell<BTreeMap<u64, HostTimer>>>,
    ) -> Rc<Self> {
        Rc::new_cyclic(|weak: &Weak<Self>| {
            let weak = weak.clone();
            let callback = Closure::<dyn FnMut()>::new(move || {
                if let Some(arbiter) = weak.upgrade() {
                    arbiter.execute_queued_turn();
                }
            })
            .into_js_value()
            .unchecked_into::<Function>();
            Self {
                primitives,
                callback,
                atom_table,
                scheduler,
                timer_handles,
                state: Cell::new(ArbiterState::Idle),
                ignored_callbacks: Cell::new(0),
                waiters: RefCell::new(BTreeMap::new()),
                last_summary: RefCell::new(summary_to_json(&WasmRunSummary::default(), Vec::new())),
                last_error: RefCell::new(None),
                instrumentation: RefCell::new(ArbiterInstrumentation::default()),
            }
        })
    }

    fn request_external_turn(self: &Rc<Self>) -> Result<(), JsValue> {
        match self.state.get() {
            ArbiterState::Idle => self.queue_turn(HostTurnLeg::ExternalMicrotask),
            ArbiterState::Queued | ArbiterState::Draining => Ok(()),
        }
    }

    fn queue_turn(self: &Rc<Self>, leg: HostTurnLeg) -> Result<(), JsValue> {
        self.state.set(ArbiterState::Queued);
        {
            let mut instrumentation = self.instrumentation.borrow_mut();
            instrumentation.arbiter.requests = instrumentation.arbiter.requests.saturating_add(1);
            instrumentation.arbiter.queued_now = 1;
        }
        let result = match leg {
            HostTurnLeg::ExternalMicrotask => self
                .primitives
                .queue_microtask
                .call1(&self.primitives.global, self.callback.as_ref()),
            HostTurnLeg::FairnessMacrotask => self.primitives.set_timeout.call2(
                &self.primitives.global,
                self.callback.as_ref(),
                &JsValue::from_f64(0.0),
            ),
        };
        match result {
            Ok(_opaque_handle) => Ok(()),
            Err(error) => {
                self.state.set(ArbiterState::Idle);
                self.instrumentation.borrow_mut().arbiter.queued_now = 0;
                self.fail(error.clone());
                Err(error)
            }
        }
    }

    fn execute_queued_turn(self: &Rc<Self>) {
        if self.ignored_callbacks.get() != 0 {
            self.ignored_callbacks
                .set(self.ignored_callbacks.get().saturating_sub(1));
            return;
        }
        if self.state.get() != ArbiterState::Queued {
            return;
        }
        self.state.set(ArbiterState::Draining);
        {
            let mut instrumentation = self.instrumentation.borrow_mut();
            instrumentation.arbiter.queued_now = 0;
            instrumentation.arbiter.executions =
                instrumentation.arbiter.executions.saturating_add(1);
        }
        match self.perform_drain() {
            Ok((summary, _json)) => self.finish_drain(&summary),
            Err(error) => {
                self.state.set(ArbiterState::Idle);
                self.fail(error);
            }
        }
    }

    fn run_manual_drain(self: &Rc<Self>) -> Result<Value, JsValue> {
        match self.state.get() {
            ArbiterState::Draining => {
                return Err(JsValue::from_str("arbiter is already draining"));
            }
            ArbiterState::Queued => {
                self.ignored_callbacks
                    .set(self.ignored_callbacks.get().saturating_add(1));
                self.instrumentation.borrow_mut().arbiter.queued_now = 0;
            }
            ArbiterState::Idle => {}
        }
        self.state.set(ArbiterState::Draining);
        let (summary, json) = self.perform_drain()?;
        self.finish_drain(&summary);
        Ok(json)
    }

    fn perform_drain(self: &Rc<Self>) -> Result<(WasmRunSummary, Value), JsValue> {
        let summary = self.scheduler.borrow_mut().run_until_idle();
        sync_host_timers_inner(&self.scheduler, &self.timer_handles, Rc::downgrade(self))?;
        let exits = self
            .scheduler
            .borrow()
            .exit_results()
            .into_iter()
            .map(|(pid, term)| {
                json!({
                    "pid": pid,
                    "value": term_to_json_or_fallback(term, self.atom_table.as_ref())
                })
            })
            .collect::<Vec<_>>();
        let json = summary_to_json(&summary, exits);
        *self.last_summary.borrow_mut() = json.clone();
        self.resolve_waiters(&summary, &json);
        Ok((summary, json))
    }

    fn finish_drain(self: &Rc<Self>, summary: &WasmRunSummary) {
        match summary.state {
            WasmRunState::Idle { .. } => self.state.set(ArbiterState::Idle),
            WasmRunState::FairnessYield { .. } => {
                if let Err(error) = self.queue_turn(HostTurnLeg::FairnessMacrotask) {
                    self.fail(error);
                }
            }
        }
    }

    fn await_exit(self: &Rc<Self>, pid: u64) -> Promise {
        if let Some(error) = self.last_error.borrow().clone() {
            return Promise::reject(&error);
        }
        if let Some(result) = self.scheduler.borrow_mut().take_exit_result(pid) {
            let value = term_to_json_or_fallback(result.root(), self.atom_table.as_ref());
            return Promise::resolve(&completion_to_js(
                "exited",
                pid,
                value,
                self.last_summary.borrow().clone(),
            ));
        }
        if self.scheduler.borrow().has_exit_error(pid) {
            return Promise::resolve(&completion_to_js(
                "errored",
                pid,
                Value::Null,
                self.last_summary.borrow().clone(),
            ));
        }
        if self.state.get() == ArbiterState::Idle
            && self.scheduler.borrow().runnable_count() == 0
            && self.timer_handles.borrow().is_empty()
        {
            return Promise::resolve(&completion_to_js(
                "idle",
                pid,
                Value::Null,
                self.last_summary.borrow().clone(),
            ));
        }

        let arbiter = Rc::clone(self);
        Promise::new(&mut move |resolve, reject| {
            arbiter
                .waiters
                .borrow_mut()
                .entry(pid)
                .or_default()
                .push(ExitWaiter { resolve, reject });
        })
    }

    fn resolve_waiters(&self, summary: &WasmRunSummary, summary_json: &Value) {
        for pid in &summary.exited {
            let Some(waiters) = self.waiters.borrow_mut().remove(pid) else {
                continue;
            };
            let result = self
                .scheduler
                .borrow_mut()
                .take_exit_result(*pid)
                .map(|term| term_to_json_or_fallback(term.root(), self.atom_table.as_ref()))
                .unwrap_or(Value::Null);
            resolve_waiters(
                waiters,
                completion_to_js("exited", *pid, result, summary_json.clone()),
            );
        }
        for pid in &summary.errored {
            let Some(waiters) = self.waiters.borrow_mut().remove(pid) else {
                continue;
            };
            resolve_waiters(
                waiters,
                completion_to_js("errored", *pid, Value::Null, summary_json.clone()),
            );
        }
        if matches!(summary.state, WasmRunState::Idle { .. })
            && self.timer_handles.borrow().is_empty()
        {
            let remaining = std::mem::take(&mut *self.waiters.borrow_mut());
            for (pid, waiters) in remaining {
                resolve_waiters(
                    waiters,
                    completion_to_js("idle", pid, Value::Null, summary_json.clone()),
                );
            }
        }
    }

    fn fail(&self, error: JsValue) {
        *self.last_error.borrow_mut() = Some(error.clone());
        let waiters = std::mem::take(&mut *self.waiters.borrow_mut());
        for waiter in waiters.into_values().flatten() {
            let _ignored = waiter.reject.call1(&JsValue::UNDEFINED, &error);
        }
    }

    fn record_deadline_request(&self) {
        let mut instrumentation = self.instrumentation.borrow_mut();
        instrumentation.receive_timers.requests =
            instrumentation.receive_timers.requests.saturating_add(1);
        instrumentation.receive_timers.queued_now =
            instrumentation.receive_timers.queued_now.saturating_add(1);
    }

    fn record_deadline_execution(&self) {
        let mut instrumentation = self.instrumentation.borrow_mut();
        instrumentation.receive_timers.queued_now =
            instrumentation.receive_timers.queued_now.saturating_sub(1);
        instrumentation.receive_timers.executions =
            instrumentation.receive_timers.executions.saturating_add(1);
    }

    fn record_deadline_cancellation(&self) {
        let mut instrumentation = self.instrumentation.borrow_mut();
        instrumentation.receive_timers.queued_now =
            instrumentation.receive_timers.queued_now.saturating_sub(1);
        instrumentation.receive_timers.cancellations = instrumentation
            .receive_timers
            .cancellations
            .saturating_add(1);
    }
}

fn completion_to_js(state: &str, pid: u64, result: Value, summary: Value) -> JsValue {
    JsValue::from_str(
        &json!({
            "state": state,
            "pid": pid,
            "result": result,
            "summary": summary,
        })
        .to_string(),
    )
}

fn resolve_waiters(waiters: Vec<ExitWaiter>, value: JsValue) {
    for waiter in waiters {
        let _ignored = waiter.resolve.call1(&JsValue::UNDEFINED, &value);
    }
}

/// Drain the scheduler's pending receive-timer cancellations and schedules,
/// reflecting each into a host `setTimeout`/`clearTimeout`.
///
/// Every scheduler borrow is scoped and dropped before a host call.
fn sync_host_timers_inner(
    scheduler: &Rc<RefCell<WasmScheduler>>,
    timer_handles: &Rc<RefCell<BTreeMap<u64, HostTimer>>>,
    arbiter: Weak<HostArbiter>,
) -> Result<(), JsValue> {
    let cancellations = scheduler.borrow_mut().take_pending_timer_cancellations();
    for timer_id in cancellations {
        clear_host_timer(timer_handles, timer_id, &arbiter);
    }
    let schedules = scheduler.borrow_mut().take_pending_timer_schedules();
    for schedule in schedules {
        schedule_host_timer(
            scheduler,
            timer_handles,
            schedule.pid,
            schedule.timer_id,
            schedule.milliseconds,
            arbiter.clone(),
        )?;
    }
    Ok(())
}

fn schedule_host_timer(
    scheduler: &Rc<RefCell<WasmScheduler>>,
    timer_handles: &Rc<RefCell<BTreeMap<u64, HostTimer>>>,
    pid: u64,
    timer_id: u64,
    milliseconds: u64,
    arbiter: Weak<HostArbiter>,
) -> Result<(), JsValue> {
    clear_host_timer(timer_handles, timer_id, &arbiter);
    let scheduler = Rc::clone(scheduler);
    let handles = Rc::clone(timer_handles);
    let arbiter_for_callback = arbiter.clone();
    let callback = Closure::<dyn FnMut()>::new(move || {
        let was_armed = handles.borrow_mut().remove(&timer_id).is_some();
        let edge = {
            let mut scheduler = scheduler.borrow_mut();
            let _fired = scheduler.timer_fired(pid, timer_id);
            scheduler.take_external_runnable_edge()
        };
        if let Some(arbiter) = arbiter_for_callback.upgrade() {
            if was_armed {
                arbiter.record_deadline_execution();
            }
            if edge && let Err(error) = arbiter.request_external_turn() {
                arbiter.fail(error);
            }
        }
    });
    let handle = set_timeout(&callback, milliseconds)?;
    timer_handles.borrow_mut().insert(
        timer_id,
        HostTimer {
            handle,
            _callback: callback,
        },
    );
    if let Some(arbiter) = arbiter.upgrade() {
        arbiter.record_deadline_request();
    }
    Ok(())
}

fn clear_host_timer(
    timer_handles: &Rc<RefCell<BTreeMap<u64, HostTimer>>>,
    timer_id: u64,
    arbiter: &Weak<HostArbiter>,
) {
    if let Some(timer) = timer_handles.borrow_mut().remove(&timer_id) {
        clear_timeout(&timer.handle);
        if let Some(arbiter) = arbiter.upgrade() {
            arbiter.record_deadline_cancellation();
        }
    }
}

struct HostTimer {
    handle: JsValue,
    _callback: Closure<dyn FnMut()>,
}

struct HostAsyncNifs {
    atom_table: Arc<AtomTable>,
    callbacks: RefCell<BTreeMap<NativeKey, Function>>,
    scheduler: Weak<RefCell<WasmScheduler>>,
    arbiter: Weak<HostArbiter>,
}

impl HostAsyncNifs {
    fn new(
        atom_table: Arc<AtomTable>,
        scheduler: Weak<RefCell<WasmScheduler>>,
        arbiter: Weak<HostArbiter>,
    ) -> Self {
        Self {
            atom_table,
            callbacks: RefCell::new(BTreeMap::new()),
            scheduler,
            arbiter,
        }
    }

    fn register(&self, key: NativeKey, callback: Function) {
        self.callbacks.borrow_mut().insert(key, callback);
    }
}

impl HostAsyncNifs {
    fn start_async_nif(
        &self,
        mfa: NativeKey,
        args: &[Term],
        context: &mut beamr::native::ProcessContext<'_>,
    ) -> Result<Term, Term> {
        let Some(callback) = self.callbacks.borrow().get(&mfa).cloned() else {
            return Err(Term::atom(beamr::atom::Atom::UNDEF));
        };
        self.start_callback(callback, args, context, HostCallbackArguments::SingleArray)
    }

    fn start_callback(
        &self,
        callback: Function,
        args: &[Term],
        context: &mut beamr::native::ProcessContext<'_>,
        arguments: HostCallbackArguments,
    ) -> Result<Term, Term> {
        let Some(pid) = context.pid() else {
            return Err(Term::atom(beamr::atom::Atom::BADARG));
        };
        let args_array = terms_to_js_array(args, self.atom_table.as_ref())
            .map_err(|_| Term::atom(beamr::atom::Atom::BADARG))?;
        let value = match arguments {
            HostCallbackArguments::SingleArray => callback.call1(&JsValue::UNDEFINED, &args_array),
            HostCallbackArguments::Positional => Reflect::apply(
                &callback,
                &JsValue::UNDEFINED,
                args_array.unchecked_ref::<js_sys::Array>(),
            ),
        }
        .map_err(|_| Term::atom(beamr::atom::Atom::BADARG))?;
        if is_promise_like(&value) {
            self.start_promise_completion(pid, Promise::resolve(&value));
            context.request_suspend(None);
            Ok(Term::NIL)
        } else {
            js_value_to_term_in_context(value, context)
                .map_err(|_| Term::atom(beamr::atom::Atom::BADARG))
        }
    }

    fn start_promise_completion(&self, pid: u64, promise: Promise) {
        let scheduler = self.scheduler.clone();
        let arbiter = self.arbiter.clone();
        let atom_table = Arc::clone(&self.atom_table);
        wasm_bindgen_futures::spawn_local(async move {
            let completion = match JsFuture::from(promise).await {
                Ok(value) => js_value_to_owned_term(value, &atom_table)
                    .map(WasmAsyncCompletion::Ok)
                    .unwrap_or_else(|_| {
                        WasmAsyncCompletion::Error(beamr::ets::OwnedTerm::immediate(Term::atom(
                            beamr::atom::Atom::BADARG,
                        )))
                    }),
                Err(error) => js_value_to_owned_term(error, &atom_table)
                    .map(WasmAsyncCompletion::Error)
                    .unwrap_or_else(|_| {
                        WasmAsyncCompletion::Error(beamr::ets::OwnedTerm::immediate(Term::atom(
                            beamr::atom::Atom::ERROR,
                        )))
                    }),
            };
            if let Some(scheduler) = scheduler.upgrade() {
                let edge = {
                    let mut scheduler = scheduler.borrow_mut();
                    let _completed = scheduler.complete_async(pid, completion);
                    scheduler.take_external_runnable_edge()
                };
                if edge
                    && let Some(arbiter) = arbiter.upgrade()
                    && let Err(error) = arbiter.request_external_turn()
                {
                    arbiter.fail(error);
                }
            }
        });
    }
}

struct HostJsCallbacks {
    atom_table: Arc<AtomTable>,
    callbacks: RefCell<BTreeMap<String, Function>>,
    async_nifs: Rc<HostAsyncNifs>,
}

impl HostJsCallbacks {
    fn new(
        atom_table: Arc<AtomTable>,
        scheduler: Weak<RefCell<WasmScheduler>>,
        arbiter: Weak<HostArbiter>,
    ) -> Self {
        let async_nifs = Rc::new(HostAsyncNifs::new(
            Arc::clone(&atom_table),
            scheduler,
            arbiter,
        ));
        Self {
            atom_table,
            callbacks: RefCell::new(BTreeMap::new()),
            async_nifs,
        }
    }

    fn register(&self, name: &str, callback: Function) {
        self.callbacks
            .borrow_mut()
            .insert(name.to_owned(), callback);
    }

    fn start_js_callback(
        &self,
        args: &[Term],
        context: &mut beamr::native::ProcessContext<'_>,
    ) -> Result<Term, Term> {
        let Some((name_term, callback_args)) = args.split_first() else {
            return Err(Term::atom(beamr::atom::Atom::BADARG));
        };
        let name_value = term_to_js_value(*name_term, self.atom_table.as_ref())
            .map_err(|_| Term::atom(beamr::atom::Atom::BADARG))?;
        let Some(name) = name_value.as_string() else {
            return Err(Term::atom(beamr::atom::Atom::BADARG));
        };
        let Some(callback) = self.callbacks.borrow().get(&name).cloned() else {
            return Err(Term::atom(beamr::atom::Atom::UNDEF));
        };
        self.async_nifs.start_callback(
            callback,
            callback_args,
            context,
            HostCallbackArguments::Positional,
        )
    }
}

#[derive(Clone, Copy)]
enum HostCallbackArguments {
    SingleArray,
    Positional,
}

// Process-global registry of JavaScript actor handlers (`reply = handler(request)`).
//
// A JS `Function` is `!Send`, but [`beamr::DynActor`]'s reply transform must be
// `Send + Sync` to be captured by the restart-capable spawn factory. The transform
// therefore captures only a `u64` handler id (and the `Send + Sync` atom table)
// and dispatches through this thread-local, where the live `Function` is held —
// so nothing `!Send` ever crosses the actor boundary. The wasm runtime is
// single-threaded; the thread-local is reached only on the host thread during a
// pumped turn, so the `RefCell` is never contended. Ids are drawn from a global
// monotonic counter, so they are unique across every VM in this thread.
thread_local! {
    static ACTOR_HANDLERS: RefCell<BTreeMap<u64, Function>> = const { RefCell::new(BTreeMap::new()) };
}

static NEXT_ACTOR_HANDLER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Per-VM owner of the handler ids it registered, so a dropped VM removes its JS
/// handlers from the thread-local registry (no leak across VM lifetimes).
struct HostActorHandlers {
    ids: RefCell<Vec<u64>>,
}

impl HostActorHandlers {
    fn new() -> Self {
        Self {
            ids: RefCell::new(Vec::new()),
        }
    }

    /// Store `handler` in the thread-local registry and return its global id.
    fn register(&self, handler: Function) -> u64 {
        let id = NEXT_ACTOR_HANDLER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ACTOR_HANDLERS.with(|handlers| {
            handlers.borrow_mut().insert(id, handler);
        });
        self.ids.borrow_mut().push(id);
        id
    }
}

impl Drop for HostActorHandlers {
    fn drop(&mut self) {
        ACTOR_HANDLERS.with(|handlers| {
            let mut handlers = handlers.borrow_mut();
            for id in self.ids.borrow().iter() {
                handlers.remove(id);
            }
        });
    }
}

/// Run the registered JS handler `id` over `request`, marshalling request and
/// reply through the term codec.
///
/// Returns the reply term graph. A missing handler, a marshalling failure, or a
/// JS exception is surfaced as an `{error, Reason}` reply term (never a panic
/// across the actor boundary), so the awaiting `Promise` still resolves with an
/// inspectable value.
fn invoke_actor_handler(id: u64, request: &OwnedTerm, atom_table: &Arc<AtomTable>) -> OwnedTerm {
    let handler = ACTOR_HANDLERS.with(|handlers| handlers.borrow().get(&id).cloned());
    let Some(handler) = handler else {
        return error_reply_term(atom_table, "actor handler is not registered");
    };
    let request_value = match term_to_js_value(request.root(), atom_table.as_ref()) {
        Ok(value) => value,
        Err(_) => return error_reply_term(atom_table, "failed to marshal request to JavaScript"),
    };
    let reply_value = match handler.call1(&JsValue::UNDEFINED, &request_value) {
        Ok(value) => value,
        Err(_) => return error_reply_term(atom_table, "actor handler threw an exception"),
    };
    match js_value_to_owned_term(reply_value, atom_table) {
        Ok(owned) => owned,
        Err(_) => error_reply_term(atom_table, "failed to marshal reply from JavaScript"),
    }
}

/// Build an `{error, <<reason>>}` owned reply term graph for a handler failure.
fn error_reply_term(atom_table: &Arc<AtomTable>, reason: &str) -> OwnedTerm {
    let mut context = beamr::native::ProcessContext::new();
    context.set_atom_table(Some(Arc::clone(atom_table)));
    let error_atom = Term::atom(beamr::atom::Atom::ERROR);
    let reason_term = context
        .alloc_binary(reason.as_bytes())
        .unwrap_or(error_atom);
    let tuple = context
        .alloc_tuple(&[error_atom, reason_term])
        .unwrap_or(error_atom);
    context
        .take_detached_result(tuple)
        .unwrap_or_else(|| OwnedTerm::immediate(error_atom))
}

struct HostWasmFacility {
    async_nifs: Rc<HostAsyncNifs>,
    js_callbacks: Rc<HostJsCallbacks>,
    js_callback_module: beamr::atom::Atom,
    js_callback_function: beamr::atom::Atom,
}

impl WasmAsyncNifFacility for HostWasmFacility {
    fn start_async_nif(
        &self,
        mfa: NativeKey,
        args: &[Term],
        context: &mut beamr::native::ProcessContext<'_>,
    ) -> Result<Term, Term> {
        if mfa.0 == self.js_callback_module && mfa.1 == self.js_callback_function {
            self.js_callbacks.start_js_callback(args, context)
        } else {
            self.async_nifs.start_async_nif(mfa, args, context)
        }
    }
}

fn js_callback_nif(
    args: &[Term],
    context: &mut beamr::native::ProcessContext<'_>,
) -> Result<Term, Term> {
    wasm_async_nif_stub(args, context)
}

fn wasm_async_nif_stub(
    args: &[Term],
    context: &mut beamr::native::ProcessContext<'_>,
) -> Result<Term, Term> {
    let Some(mfa) = context.current_native() else {
        return Err(Term::atom(beamr::atom::Atom::UNDEF));
    };
    let Some(facility) = context.wasm_async_nif_facility() else {
        return Err(Term::atom(beamr::atom::Atom::UNDEF));
    };
    facility.start_async_nif(mfa, args, context)
}

fn is_promise_like(value: &JsValue) -> bool {
    Reflect::get(value, &JsValue::from_str("then"))
        .ok()
        .is_some_and(|then| then.is_function())
}

fn register_wasm_safe_bifs(
    registry: &BifRegistryImpl,
    atom_table: &AtomTable,
) -> Result<(), NativeRegistrationError> {
    register_gate1_bifs(registry, atom_table)?;
    register_gate2_bifs(registry, atom_table)?;
    register_stdlib_stubs(registry, atom_table)?;
    Ok(())
}

fn unresolved_imports_to_json(
    imports: Vec<UnresolvedImport>,
    atom_table: &AtomTable,
) -> Vec<Value> {
    imports
        .into_iter()
        .map(|import| {
            let module = atom_table
                .resolve(import.module)
                .map_or_else(|| format!("{:?}", import.module), str::to_owned);
            let function = atom_table
                .resolve(import.function)
                .map_or_else(|| format!("{:?}", import.function), str::to_owned);
            json!({
                "module": module,
                "function": function,
                "arity": import.arity,
            })
        })
        .collect()
}

fn summary_to_json(summary: &WasmRunSummary, exits: Vec<Value>) -> Value {
    let next_native_deadline_ms = summary.state.next_native_deadline_millis_from_now();
    let (state, runnable_remaining) = match summary.state {
        WasmRunState::Idle { .. } => ("idle", 0),
        WasmRunState::FairnessYield { runnable_remaining } => {
            ("fairness_yield", runnable_remaining)
        }
    };
    json!({
        "state": state,
        "next_native_deadline_ms": next_native_deadline_ms,
        "runnable_remaining": runnable_remaining,
        "executed": summary.executed,
        "yielded": &summary.yielded,
        "waiting": &summary.waiting,
        "exited": &summary.exited,
        "errored": &summary.errored,
        "results": exits,
    })
}

fn term_to_json_or_fallback(term: Term, atom_table: &AtomTable) -> Value {
    match term_to_value(term, atom_table) {
        Ok(value) => value,
        Err(_) => Value::String(format_term(term, atom_table)),
    }
}

fn json_to_js(value: &Value) -> Result<JsValue, JsValue> {
    Ok(JsValue::from_str(&value.to_string()))
}

fn registration_error_to_js(error: NativeRegistrationError) -> JsValue {
    JsValue::from_str(&error.to_string())
}

fn set_timeout(callback: &Closure<dyn FnMut()>, milliseconds: u64) -> Result<JsValue, JsValue> {
    let global = js_sys::global();
    let set_timeout = Reflect::get(&global, &JsValue::from_str("setTimeout"))?
        .dyn_into::<Function>()
        .map_err(|_| JsValue::from_str("global setTimeout is not a function"))?;
    let delay = i32::try_from(milliseconds).unwrap_or(i32::MAX);
    set_timeout.call2(
        &global,
        callback.as_ref().unchecked_ref(),
        &JsValue::from_f64(f64::from(delay)),
    )
}

fn clear_timeout(handle: &JsValue) {
    let global = js_sys::global();
    if let Ok(clear_timeout) = Reflect::get(&global, &JsValue::from_str("clearTimeout"))
        && let Ok(clear_timeout) = clear_timeout.dyn_into::<Function>()
    {
        let _ignored = clear_timeout.call1(&global, handle);
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CallbackCounterSnapshot {
    requests: u64,
    queued_now: usize,
    executions: u64,
    cancellations: u64,
}

#[cfg(all(test, target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ArbiterCounterSnapshot {
    arbiter: CallbackCounterSnapshot,
    receive_timers: CallbackCounterSnapshot,
}

#[cfg(all(test, target_arch = "wasm32"))]
impl From<CallbackCounters> for CallbackCounterSnapshot {
    fn from(counters: CallbackCounters) -> Self {
        Self {
            requests: counters.requests,
            queued_now: counters.queued_now,
            executions: counters.executions,
            cancellations: counters.cancellations,
        }
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
impl WasmVm {
    fn arbiter_counters(&self) -> ArbiterCounterSnapshot {
        let instrumentation = *self.arbiter.instrumentation.borrow();
        ArbiterCounterSnapshot {
            arbiter: instrumentation.arbiter.into(),
            receive_timers: instrumentation.receive_timers.into(),
        }
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use beamr::atom::Atom;
    use beamr::constant_pool::ConstantPool;
    use beamr::loader::decode::compact::Operand;
    use beamr::loader::{Instruction, LambdaEntry, LineInfo, Literal};
    use beamr::module::{Module, ModuleOrigin, ResolvedImport};
    use beamr::native::native_process::{NativeContext, NativeHandler, NativeOutcome};
    use js_sys::{Array, Object};
    use wasm_bindgen_test::wasm_bindgen_test;

    fn parse_json(value: JsValue) -> Value {
        serde_json::from_str(
            value
                .as_string()
                .expect("wrapper JSON values are returned as strings")
                .as_str(),
        )
        .expect("wrapper JSON parses")
    }

    fn increment_handler() -> Closure<dyn FnMut(JsValue) -> JsValue> {
        Closure::new(|request: JsValue| {
            let n = Reflect::get(&request, &JsValue::from_str("n"))
                .ok()
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0);
            let reply = Object::new();
            let _set = Reflect::set(
                &reply,
                &JsValue::from_str("result"),
                &JsValue::from_f64(n + 1.0),
            );
            reply.into()
        })
    }

    fn timeout_value(milliseconds: i32, value: JsValue) -> Promise {
        Promise::new(&mut move |resolve, _reject| {
            let callback_value = value.clone();
            let callback = Closure::once_into_js(move || {
                let _ignored = resolve.call1(&JsValue::UNDEFINED, &callback_value);
            });
            let global = js_sys::global();
            let set_timeout = Reflect::get(&global, &JsValue::from_str("setTimeout"))
                .expect("setTimeout is present")
                .dyn_into::<Function>()
                .expect("setTimeout is a function");
            let _opaque_handle = set_timeout
                .call2(
                    &global,
                    &callback,
                    &JsValue::from_f64(f64::from(milliseconds)),
                )
                .expect("test macrotask schedules");
        })
    }

    async fn host_macrotask() {
        JsFuture::from(timeout_value(0, JsValue::UNDEFINED))
            .await
            .expect("test macrotask resolves");
    }

    async fn host_microtask() {
        JsFuture::from(Promise::resolve(&JsValue::UNDEFINED))
            .await
            .expect("test microtask resolves");
    }

    fn request(n: f64) -> JsValue {
        let request = Object::new();
        let _set = Reflect::set(&request, &JsValue::from_str("n"), &JsValue::from_f64(n));
        request.into()
    }

    struct MailboxDrainer;

    impl NativeHandler for MailboxDrainer {
        fn handle(&mut self, context: &mut NativeContext<'_>) -> NativeOutcome {
            while context.recv().is_some() {}
            NativeOutcome::Wait
        }
    }

    async fn spawn_waiting_mailbox(vm: &mut WasmVm) -> u64 {
        let pid = vm
            .scheduler
            .borrow_mut()
            .spawn_native_root(Box::new(|| Box::new(MailboxDrainer)));
        vm.schedule_external_edge()
            .expect("test native root schedules the arbiter");
        host_macrotask().await;
        pid
    }

    fn receive_after_module(atoms: &AtomTable) -> (Atom, Atom, Module) {
        let name = atoms.intern("wport2_receive_after");
        let function = atoms.intern("run");
        let timed_out = atoms.intern("timed_out");
        let code = vec![
            Instruction::Label { label: 1 },
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
                timeout: Operand::Unsigned(25),
            },
            Instruction::Timeout,
            Instruction::Move {
                source: Operand::Atom(Some(timed_out)),
                destination: Operand::X(0),
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
        let mut exports = HashMap::new();
        exports.insert((function, 0), 1);
        (
            name,
            function,
            Module {
                name,
                generation: 0,
                origin: ModuleOrigin::Preloaded,
                exports,
                label_index,
                code,
                function_table: Vec::new(),
                line_table: Vec::new(),
                literals: Vec::<Literal>::new(),
                constant_pool: ConstantPool::new(),
                resolved_imports: Vec::<ResolvedImport>::new(),
                lambdas: Vec::<LambdaEntry>::new(),
                string_table: Vec::new(),
                line_info: Vec::<LineInfo>::new(),
            },
        )
    }

    #[wasm_bindgen_test]
    async fn await_vm_call_resolves_with_js_handler_reply() {
        let mut vm = create_vm().expect("VM constructs");
        let handler = increment_handler();
        let handler_fn = handler.as_ref().unchecked_ref::<Function>().clone();
        let pid = vm.spawn_actor(handler_fn);
        let promise = vm.call(pid, request(41.0)).expect("call returns a Promise");
        let value = JsFuture::from(promise)
            .await
            .expect("the arbiter drives the call Promise to completion");
        let result = Reflect::get(&value, &JsValue::from_str("result"))
            .expect("reply has a result field")
            .as_f64();
        assert_eq!(result, Some(42.0), "JS handler replied with n + 1");
        drop(handler);
    }

    #[wasm_bindgen_test]
    async fn arbiter_drives_an_actor_call_to_completion() {
        let mut vm = WasmVm::new().expect("VM constructs");
        let handler = increment_handler();
        let handler_fn = handler.as_ref().unchecked_ref::<Function>().clone();
        let pid = vm.spawn_actor(handler_fn);
        let promise = vm.call(pid, request(7.0)).expect("call returns a Promise");
        let value = JsFuture::from(promise)
            .await
            .expect("the call Promise resolves after arbiter execution");
        let result = Reflect::get(&value, &JsValue::from_str("result"))
            .expect("reply has a result field")
            .as_f64();
        assert_eq!(result, Some(8.0), "the arbiter drove the actor reply");
        drop(handler);
    }

    #[wasm_bindgen_test]
    async fn idle_vm_schedules_zero_recurring_callbacks() {
        let mut vm = WasmVm::new().expect("VM constructs");
        let summary = parse_json(vm.run_step().expect("manual idle drain succeeds"));
        assert_eq!(summary["state"], "idle");
        let before = vm.arbiter_counters();

        host_macrotask().await;

        let after = vm.arbiter_counters();
        assert_eq!(after.arbiter.requests, before.arbiter.requests);
        assert_eq!(after.arbiter.executions, before.arbiter.executions);
        assert_eq!(after.arbiter.queued_now, 0);
        assert_eq!(after.receive_timers, before.receive_timers);
    }

    #[wasm_bindgen_test]
    async fn idle_to_runnable_burst_queues_one_arbiter_turn() {
        let mut vm = WasmVm::new().expect("VM constructs");
        let pid = spawn_waiting_mailbox(&mut vm).await;
        let before = vm.arbiter_counters();

        for value in 0..8 {
            vm.send_message(pid, JsValue::from_f64(f64::from(value)))
                .expect("host send succeeds");
        }
        let queued = vm.arbiter_counters();
        assert_eq!(queued.arbiter.requests, before.arbiter.requests + 1);
        assert_eq!(queued.arbiter.queued_now, 1);
        assert_eq!(queued.arbiter.executions, before.arbiter.executions);

        host_macrotask().await;
        let after = vm.arbiter_counters();
        assert_eq!(after.arbiter.executions, before.arbiter.executions + 1);
        assert_eq!(after.arbiter.queued_now, 0);
    }

    #[wasm_bindgen_test]
    async fn arbiter_reedges_after_true_idle() {
        let mut vm = WasmVm::new().expect("VM constructs");
        let pid = spawn_waiting_mailbox(&mut vm).await;
        let before = vm.arbiter_counters();

        vm.send_message(pid, JsValue::from_f64(1.0))
            .expect("first host send succeeds");
        host_macrotask().await;
        vm.send_message(pid, JsValue::from_f64(2.0))
            .expect("second host send succeeds");
        host_macrotask().await;

        let after = vm.arbiter_counters();
        assert_eq!(after.arbiter.requests, before.arbiter.requests + 2);
        assert_eq!(after.arbiter.executions, before.arbiter.executions + 2);
        assert_eq!(after.arbiter.queued_now, 0);
    }

    #[wasm_bindgen_test]
    async fn fairness_yield_reports_runnable_remaining() {
        let mut vm = WasmVm::new().expect("VM constructs");
        let handler = increment_handler();
        let handler_fn = handler.as_ref().unchecked_ref::<Function>().clone();
        for _ in 0..1_025 {
            let _pid = vm.spawn_actor(handler_fn.clone());
        }
        let queued = vm.arbiter_counters();
        assert_eq!(queued.arbiter.requests, 1);
        assert_eq!(queued.arbiter.queued_now, 1);

        host_microtask().await;

        let middle = vm.arbiter_counters();
        assert_eq!(middle.arbiter.executions, 1);
        assert_eq!(middle.arbiter.requests, 2);
        assert_eq!(middle.arbiter.queued_now, 1);
        let summary = vm.arbiter.last_summary.borrow().clone();
        assert_eq!(summary["state"], "fairness_yield");
        assert_eq!(summary["runnable_remaining"], 1);

        host_macrotask().await;
        let after = vm.arbiter_counters();
        assert_eq!(after.arbiter.executions, 2);
        assert_eq!(after.arbiter.queued_now, 0);
        assert_eq!(vm.arbiter.last_summary.borrow()["state"], "idle");
        drop(handler);
    }

    #[wasm_bindgen_test]
    fn arbiter_installation_rejects_missing_host_primitive() {
        let global = js_sys::global();

        let queue_key = JsValue::from_str("queueMicrotask");
        let queue_microtask = Reflect::get(&global, &queue_key).expect("queueMicrotask exists");
        assert!(Reflect::delete_property(&global, &queue_key).expect("queueMicrotask deletes"));
        let queue_error = match WasmVm::new() {
            Ok(_) => String::from("constructor unexpectedly succeeded"),
            Err(error) => error.as_string().unwrap_or_default(),
        };
        Reflect::set(&global, &queue_key, &queue_microtask).expect("queueMicrotask restores");
        assert!(queue_error.contains("queueMicrotask"), "{queue_error}");

        let timeout_key = JsValue::from_str("setTimeout");
        let set_timeout = Reflect::get(&global, &timeout_key).expect("setTimeout exists");
        assert!(Reflect::delete_property(&global, &timeout_key).expect("setTimeout deletes"));
        let timeout_error = match WasmVm::new() {
            Ok(_) => String::from("constructor unexpectedly succeeded"),
            Err(error) => error.as_string().unwrap_or_default(),
        };
        Reflect::set(&global, &timeout_key, &set_timeout).expect("setTimeout restores");
        assert!(timeout_error.contains("setTimeout"), "{timeout_error}");
    }

    #[wasm_bindgen_test]
    async fn await_exit_waits_for_armed_receive_timer() {
        let mut vm = WasmVm::new().expect("VM constructs");
        let (module, function, definition) = receive_after_module(&vm.atom_table);
        vm.module_registry.insert(definition);
        let module_name = vm
            .atom_table
            .resolve(module)
            .expect("module name")
            .to_owned();
        let function_name = vm
            .atom_table
            .resolve(function)
            .expect("function name")
            .to_owned();
        let pid = vm
            .spawn(&module_name, &function_name, "[]")
            .expect("receive-after process spawns");
        let completion = vm.await_exit(pid);
        let marker = JsValue::from_str("macrotask");
        let race = Promise::race(&Array::of2(
            completion.as_ref(),
            timeout_value(0, marker.clone()).as_ref(),
        ));

        let first = JsFuture::from(race).await.expect("race resolves");
        assert_eq!(first.as_string(), marker.as_string());
        assert_eq!(vm.arbiter_counters().receive_timers.queued_now, 1);

        let settled = parse_json(
            JsFuture::from(completion)
                .await
                .expect("receive timer fires and target exits"),
        );
        assert_eq!(settled["state"], "exited");
        assert_eq!(settled["pid"], pid);
        assert_eq!(settled["result"], "timed_out");
        let counters = vm.arbiter_counters();
        assert_eq!(counters.receive_timers.requests, 1);
        assert_eq!(counters.receive_timers.executions, 1);
        assert_eq!(counters.receive_timers.queued_now, 0);
    }
}
