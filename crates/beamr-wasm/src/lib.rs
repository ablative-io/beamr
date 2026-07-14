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
    deadline: Rc<HostDeadlineService>,
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
        let deadline = HostDeadlineService::new(primitives.clone(), Rc::clone(&scheduler));
        let arbiter = HostArbiter::new(
            primitives,
            Arc::clone(&atom_table),
            Rc::clone(&scheduler),
            Rc::clone(&deadline),
        );
        deadline.set_arbiter(Rc::downgrade(&arbiter));
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
            deadline,
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
        self.deadline.sync_and_reconcile()?;
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
    ///
    /// This is external host driving, not an admitted unified-deadline fire: the
    /// record leaves the receive map and the unified arm is reconciled (moving
    /// or clearing if this was the earliest deadline), but no admitted
    /// execution is counted.
    pub fn timer_fired(&mut self, pid: u64, timer_id: u64) -> Result<bool, JsValue> {
        self.deadline.remove_receive_record(timer_id);
        let fired = self.scheduler.borrow_mut().timer_fired(pid, timer_id);
        self.deadline.sync_and_reconcile()?;
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
}

#[derive(Clone)]
struct HostPrimitives {
    global: JsValue,
    queue_microtask: Function,
    set_timeout: Function,
    clear_timeout: Function,
}

impl HostPrimitives {
    fn probe() -> Result<Self, JsValue> {
        let global = js_sys::global();
        let queue_microtask = required_host_function(&global, "queueMicrotask")?;
        let set_timeout = required_host_function(&global, "setTimeout")?;
        let clear_timeout = required_host_function(&global, "clearTimeout")?;
        Ok(Self {
            global: global.into(),
            queue_microtask,
            set_timeout,
            clear_timeout,
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

/// Instrumentation for the unified deadline service (WPORT-3 R4).
///
/// Host-call totals: `requests` counts actual `setTimeout` arms,
/// `cancellations` counts actual `clearTimeout` calls that retire a live arm,
/// and `executions` counts admitted active-arm fires. `queued_now` is a true
/// gauge of outstanding host arms — incremented on every arm, decremented on
/// every retire/consume — so the 0/1 cardinality invariant is COUNTED and
/// falsifiable, never derived. The transition counters classify why the
/// totals changed; a stale (cleared/replaced) callback changes none of them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct DeadlineCounters {
    requests: u64,
    queued_now: usize,
    executions: u64,
    cancellations: u64,
    /// A new earlier minimum retired the old arm and re-armed earlier.
    rearms_earlier: u64,
    /// Cancelling/consuming the earliest moved the arm later or cleared it.
    cancel_moves_or_clears: u64,
    /// A new deadline arrived that did not change the armed minimum.
    later_noops: u64,
    /// A fresh arm was created while none was active (first or post-fire).
    next_arms: u64,
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
    deadline: Rc<HostDeadlineService>,
    state: Cell<ArbiterState>,
    ignored_callbacks: Cell<usize>,
    waiters: RefCell<BTreeMap<u64, Vec<ExitWaiter>>>,
    last_summary: RefCell<Value>,
    last_error: RefCell<Option<JsValue>>,
    instrumentation: RefCell<CallbackCounters>,
}

impl HostArbiter {
    fn new(
        primitives: HostPrimitives,
        atom_table: Arc<AtomTable>,
        scheduler: Rc<RefCell<WasmScheduler>>,
        deadline: Rc<HostDeadlineService>,
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
                deadline,
                state: Cell::new(ArbiterState::Idle),
                ignored_callbacks: Cell::new(0),
                waiters: RefCell::new(BTreeMap::new()),
                last_summary: RefCell::new(summary_to_json(&WasmRunSummary::default(), Vec::new())),
                last_error: RefCell::new(None),
                instrumentation: RefCell::new(CallbackCounters::default()),
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
            instrumentation.requests = instrumentation.requests.saturating_add(1);
            instrumentation.queued_now = 1;
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
                self.instrumentation.borrow_mut().queued_now = 0;
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
            instrumentation.queued_now = 0;
            instrumentation.executions = instrumentation.executions.saturating_add(1);
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
                self.instrumentation.borrow_mut().queued_now = 0;
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
        // Completed-drain deadline seam (WPORT-3 R1): synchronize receive
        // records, update the native component from the settled result (a
        // FAIRNESS YIELD carries no native deadline value, so the known one is
        // retained), then reconcile the single host arm — all before idle
        // waiters are resolved. Re-arm decisions key off this drain-completion
        // state, never off which arbiter callback identity ran.
        let observed_receive = self.deadline.sync_receive_records();
        let observed_native = match summary.state {
            WasmRunState::Idle {
                next_native_deadline,
            } => self.deadline.update_native_earliest(next_native_deadline),
            WasmRunState::FairnessYield { .. } => false,
        };
        self.deadline
            .reconcile(observed_receive || observed_native)?;
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
        let true_idle_now =
            self.state.get() == ArbiterState::Idle && self.scheduler.borrow().runnable_count() == 0;
        if self.settled_idle(true_idle_now) {
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
        if self.settled_idle(matches!(summary.state, WasmRunState::Idle { .. })) {
            let remaining = std::mem::take(&mut *self.waiters.borrow_mut());
            for (pid, waiters) in remaining {
                resolve_waiters(
                    waiters,
                    completion_to_js("idle", pid, Value::Null, summary_json.clone()),
                );
            }
        }
    }

    /// SETTLED IDLE (WPORT-2 Ruling 3, extended by WPORT-3): TRUE IDLE with no
    /// pending deadline of either class. A receive-after record or a native
    /// `Deliver` earliest — armed as the one known-deadline callback — means
    /// not settled; its fire is an edge that continues the loop. Evaluated only
    /// after deadline reconciliation, by both `await_exit` paths.
    fn settled_idle(&self, true_idle: bool) -> bool {
        true_idle && !self.deadline.has_pending_deadline()
    }

    fn fail(&self, error: JsValue) {
        *self.last_error.borrow_mut() = Some(error.clone());
        let waiters = std::mem::take(&mut *self.waiters.borrow_mut());
        for waiter in waiters.into_values().flatten() {
            let _ignored = waiter.reject.call1(&JsValue::UNDEFINED, &error);
        }
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

/// One receive-after record owned by the unified deadline service: the target
/// pid plus the wrapper-stamped absolute deadline (stamp-at-sync, WPORT-3
/// Ruling 2 — records stay relative in core, and the stamp trails the
/// bytecode's `receive after` by up to one drain, a LATE-biased bounded skew).
struct ReceiveDeadline {
    pid: u64,
    deadline: web_time::Instant,
}

/// The single active host arm: one opaque `setTimeout` handle armed at the
/// unified minimum, plus the local arm token that exists ONLY to reject a
/// cleared/replaced callback that still runs. The token never drives re-arm
/// logic — re-arm keys off drain-completion state, never callback identity.
struct ActiveDeadlineArm {
    deadline: web_time::Instant,
    handle: JsValue,
    token: u64,
    _callback: Closure<dyn FnMut()>,
}

/// Wrapper-owned unified deadline service (WPORT-3 R1, Ruling 1 candidate A).
///
/// Owns (a) the receive map from timer id to `{pid, absolute deadline}` and
/// (b) the earliest native `Deliver` deadline learned from the last settled
/// drain, computes their minimum, and owns the only active known-deadline host
/// callback. NO-POLLING binds every branch: a one-shot callback for a known
/// deadline is event delivery; a recurring callback that checks the timer
/// wheel is a design error and a tear condition.
///
/// Borrow discipline (tear pen note, binding): reconcile and fire make host
/// calls (`clearTimeout`/`setTimeout`), so every scheduler and deadline-service
/// borrow is scoped and dropped before any host call.
struct HostDeadlineService {
    primitives: HostPrimitives,
    scheduler: Rc<RefCell<WasmScheduler>>,
    arbiter: RefCell<Weak<HostArbiter>>,
    receive_deadlines: RefCell<BTreeMap<u64, ReceiveDeadline>>,
    native_earliest: Cell<Option<web_time::Instant>>,
    active_arm: RefCell<Option<ActiveDeadlineArm>>,
    next_arm_token: Cell<u64>,
    counters: RefCell<DeadlineCounters>,
}

impl HostDeadlineService {
    fn new(primitives: HostPrimitives, scheduler: Rc<RefCell<WasmScheduler>>) -> Rc<Self> {
        Rc::new(Self {
            primitives,
            scheduler,
            arbiter: RefCell::new(Weak::new()),
            receive_deadlines: RefCell::new(BTreeMap::new()),
            native_earliest: Cell::new(None),
            active_arm: RefCell::new(None),
            next_arm_token: Cell::new(0),
            counters: RefCell::new(DeadlineCounters::default()),
        })
    }

    fn set_arbiter(&self, arbiter: Weak<HostArbiter>) {
        *self.arbiter.borrow_mut() = arbiter;
    }

    /// Whether a deadline of either class is pending. Feeds SETTLED IDLE: a
    /// pending receive or native deadline means not settled.
    fn has_pending_deadline(&self) -> bool {
        self.native_earliest.get().is_some() || !self.receive_deadlines.borrow().is_empty()
    }

    /// Drain the scheduler's pending receive mutations and reconcile the arm.
    /// The wrapper-sync seam used by host entry points between drains.
    fn sync_and_reconcile(self: &Rc<Self>) -> Result<(), JsValue> {
        let observed_new = self.sync_receive_records();
        self.reconcile(observed_new)
    }

    /// Synchronize receive records from the scheduler queues: cancellations
    /// are drained first and their entries removed, then one `now` is sampled
    /// and every schedule is inserted at `now + milliseconds` (stamp-at-sync).
    /// Returns whether any new deadline record was inserted.
    fn sync_receive_records(&self) -> bool {
        let cancellations = self
            .scheduler
            .borrow_mut()
            .take_pending_timer_cancellations();
        {
            let mut records = self.receive_deadlines.borrow_mut();
            for timer_id in cancellations {
                records.remove(&timer_id);
            }
        }
        let schedules = self.scheduler.borrow_mut().take_pending_timer_schedules();
        if schedules.is_empty() {
            return false;
        }
        let now = web_time::Instant::now();
        let mut records = self.receive_deadlines.borrow_mut();
        for schedule in schedules {
            let delay = std::time::Duration::from_millis(schedule.milliseconds);
            records.insert(
                schedule.timer_id,
                ReceiveDeadline {
                    pid: schedule.pid,
                    deadline: now.checked_add(delay).unwrap_or(now),
                },
            );
        }
        true
    }

    /// Record the settled drain's native component. Returns whether a new
    /// native deadline value was observed.
    fn update_native_earliest(&self, value: Option<web_time::Instant>) -> bool {
        let previous = self.native_earliest.replace(value);
        value.is_some() && value != previous
    }

    /// Remove one receive record without firing it (host-driven `timer_fired`
    /// and message-won cancellation both retire records this way).
    fn remove_receive_record(&self, timer_id: u64) -> bool {
        self.receive_deadlines
            .borrow_mut()
            .remove(&timer_id)
            .is_some()
    }

    fn unified_minimum(&self) -> Option<web_time::Instant> {
        let receive_min = self
            .receive_deadlines
            .borrow()
            .values()
            .map(|record| record.deadline)
            .min();
        match (receive_min, self.native_earliest.get()) {
            (Some(receive), Some(native)) => Some(receive.min(native)),
            (Some(receive), None) => Some(receive),
            (None, native) => native,
        }
    }

    /// Reconcile the single host arm against `min(receive_earliest,
    /// native_earliest)`: an unchanged minimum makes no host call (a new later
    /// deadline is a counted no-op); a different minimum retires the old
    /// opaque handle and arms exactly one new one-shot; no minimum clears the
    /// arm. Every borrow is dropped before the `clearTimeout`/`setTimeout`
    /// host calls.
    fn reconcile(self: &Rc<Self>, observed_new: bool) -> Result<(), JsValue> {
        let target = self.unified_minimum();
        let previous = self.active_arm.borrow().as_ref().map(|arm| arm.deadline);
        if previous == target {
            if observed_new && target.is_some() {
                let mut counters = self.counters.borrow_mut();
                counters.later_noops = counters.later_noops.saturating_add(1);
            }
            return Ok(());
        }
        let retired = self.active_arm.borrow_mut().take();
        if let Some(retired) = retired {
            // Borrows dropped; retire the old opaque handle at the host.
            self.primitives
                .clear_timeout
                .call1(&self.primitives.global, &retired.handle)?;
            let mut counters = self.counters.borrow_mut();
            counters.cancellations = counters.cancellations.saturating_add(1);
            counters.queued_now = counters.queued_now.saturating_sub(1);
        }
        match (previous, target) {
            (_, None) => {
                let mut counters = self.counters.borrow_mut();
                counters.cancel_moves_or_clears = counters.cancel_moves_or_clears.saturating_add(1);
            }
            (None, Some(deadline)) => {
                self.arm(deadline)?;
                let mut counters = self.counters.borrow_mut();
                counters.next_arms = counters.next_arms.saturating_add(1);
            }
            (Some(previous), Some(deadline)) if deadline < previous => {
                self.arm(deadline)?;
                let mut counters = self.counters.borrow_mut();
                counters.rearms_earlier = counters.rearms_earlier.saturating_add(1);
            }
            (Some(_), Some(deadline)) => {
                self.arm(deadline)?;
                let mut counters = self.counters.borrow_mut();
                counters.cancel_moves_or_clears = counters.cancel_moves_or_clears.saturating_add(1);
            }
        }
        Ok(())
    }

    /// Arm the one host one-shot for `deadline` at a saturating non-negative
    /// delay. The delay is rounded UP to whole milliseconds so the callback
    /// never runs before the stamped deadline (LATE-biased, matching the arc's
    /// unclaimed-promptness law).
    fn arm(self: &Rc<Self>, deadline: web_time::Instant) -> Result<(), JsValue> {
        let token = self.next_arm_token.get();
        self.next_arm_token.set(token.saturating_add(1));
        let weak = Rc::downgrade(self);
        let callback = Closure::<dyn FnMut()>::new(move || {
            if let Some(service) = weak.upgrade() {
                service.deadline_callback_fired(token, web_time::Instant::now());
            }
        });
        let delay_ms = millis_until_ceil(deadline);
        // No service borrow is held across the host call.
        let handle = self.primitives.set_timeout.call2(
            &self.primitives.global,
            callback.as_ref().unchecked_ref(),
            &JsValue::from_f64(delay_ms),
        )?;
        *self.active_arm.borrow_mut() = Some(ActiveDeadlineArm {
            deadline,
            handle,
            token,
            _callback: callback,
        });
        let mut counters = self.counters.borrow_mut();
        counters.requests = counters.requests.saturating_add(1);
        counters.queued_now = counters.queued_now.saturating_add(1);
        Ok(())
    }

    /// The one shared deadline-callback logic, entered by every arm's host
    /// callback (with `Instant::now()`) and by the test-only deterministic
    /// fire seam (with a supplied instant).
    ///
    /// Admission is by local arm token only: a stale (cleared/replaced)
    /// callback performs no delivery, no arbiter request, and no state or
    /// counter mutation. An admitted fire consumes the active arm, delivers
    /// the complete due set of BOTH classes at `now`, consumes the Rust ready
    /// edge once, and requests exactly ONE arbiter turn for the whole event —
    /// never one per expiry. All next-arm decisions are deferred to the
    /// requested drain's completion seam.
    fn deadline_callback_fired(self: &Rc<Self>, token: u64, now: web_time::Instant) {
        let admitted = {
            let mut arm_slot = self.active_arm.borrow_mut();
            match arm_slot.as_ref() {
                Some(arm) if arm.token == token => arm_slot.take(),
                _ => None,
            }
        };
        let Some(_consumed_arm) = admitted else {
            return;
        };
        {
            let mut counters = self.counters.borrow_mut();
            counters.executions = counters.executions.saturating_add(1);
            counters.queued_now = counters.queued_now.saturating_sub(1);
        }
        // Every due native `Deliver` is removed from the wheel and delivered
        // to its mailbox (scoped scheduler borrow).
        {
            let _woken = self.scheduler.borrow_mut().tick_native_timers_at(now);
        }
        // Collect and remove every due receive record, then fire each by id;
        // the core fire is stale-safe for a missing/mismatched id and never
        // rejects a timer because `now` is later than its requested instant.
        let due: Vec<(u64, u64)> = {
            let mut records = self.receive_deadlines.borrow_mut();
            let due_ids: Vec<u64> = records
                .iter()
                .filter(|(_, record)| record.deadline <= now)
                .map(|(timer_id, _)| *timer_id)
                .collect();
            due_ids
                .into_iter()
                .filter_map(|timer_id| {
                    records
                        .remove(&timer_id)
                        .map(|record| (timer_id, record.pid))
                })
                .collect()
        };
        for (timer_id, pid) in due {
            let _fired = self.scheduler.borrow_mut().timer_fired(pid, timer_id);
        }
        let _edge = self.scheduler.borrow_mut().take_external_runnable_edge();
        let arbiter = self.arbiter.borrow().clone();
        if let Some(arbiter) = arbiter.upgrade()
            && let Err(error) = arbiter.request_external_turn()
        {
            arbiter.fail(error);
        }
    }
}

impl Drop for HostDeadlineService {
    fn drop(&mut self) {
        // Ownership hygiene for the one active callback: retire the pending
        // host timeout at teardown so a dropped closure can never be invoked
        // by a still-queued `setTimeout` after the VM is gone.
        if let Some(arm) = self.active_arm.borrow_mut().take() {
            let _ignored = self
                .primitives
                .clear_timeout
                .call1(&self.primitives.global, &arm.handle);
        }
    }
}

/// Whole milliseconds from now until `deadline`, rounded UP so the callback
/// never runs before the stamped deadline, saturating at zero for past
/// deadlines and at `i32::MAX` (the host `setTimeout` clamp) for far futures —
/// a beyond-clamp deadline fires as one harmless early one-shot whose drain
/// re-arms the remainder, never a recurring check.
fn millis_until_ceil(deadline: web_time::Instant) -> f64 {
    let micros = deadline
        .saturating_duration_since(web_time::Instant::now())
        .as_micros();
    let max_delay = 0x7fff_ffff_u32; // i32::MAX as the host clamp ceiling
    let millis = micros.div_ceil(1_000).min(u128::from(max_delay));
    // The min() above keeps the value in u32 range.
    u32::try_from(millis).map_or(f64::from(max_delay), f64::from)
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

/// Test snapshot of the unified deadline instrumentation group (WPORT-3 R4).
/// `queued_now` is the counted gauge of outstanding host arms (0/1 when the
/// cardinality invariant holds) and `armed_deadline` the exact armed instant.
#[cfg(all(test, target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UnifiedDeadlineSnapshot {
    requests: u64,
    queued_now: usize,
    executions: u64,
    cancellations: u64,
    rearms_earlier: u64,
    cancel_moves_or_clears: u64,
    later_noops: u64,
    next_arms: u64,
    armed_deadline: Option<web_time::Instant>,
}

#[cfg(all(test, target_arch = "wasm32"))]
impl WasmVm {
    fn arbiter_counters(&self) -> ArbiterCounterSnapshot {
        let instrumentation = *self.arbiter.instrumentation.borrow();
        ArbiterCounterSnapshot {
            arbiter: instrumentation.into(),
        }
    }

    fn unified_deadline_snapshot(&self) -> UnifiedDeadlineSnapshot {
        let counters = *self.deadline.counters.borrow();
        let armed_deadline = self
            .deadline
            .active_arm
            .borrow()
            .as_ref()
            .map(|arm| arm.deadline);
        UnifiedDeadlineSnapshot {
            requests: counters.requests,
            queued_now: counters.queued_now,
            executions: counters.executions,
            cancellations: counters.cancellations,
            rearms_earlier: counters.rearms_earlier,
            cancel_moves_or_clears: counters.cancel_moves_or_clears,
            later_noops: counters.later_noops,
            next_arms: counters.next_arms,
            armed_deadline,
        }
    }

    fn unified_arm_token(&self) -> Option<u64> {
        self.deadline
            .active_arm
            .borrow()
            .as_ref()
            .map(|arm| arm.token)
    }

    /// Deterministic fire seam: invoke the one shared deadline-callback logic
    /// for the CURRENT arm at a supplied instant. The still-pending host
    /// timeout is neutralized first (test plumbing, uncounted) so the consumed
    /// arm's queued host callback cannot later invoke a dropped closure.
    fn fire_unified_deadline_at(&self, now: web_time::Instant) {
        let token = self
            .unified_arm_token()
            .expect("a unified deadline arm is active");
        self.fire_unified_deadline_token_at(token, now);
    }

    /// Deterministic fire seam for an explicit (possibly stale) arm token:
    /// runs the same shared callback logic a real host callback runs, so a
    /// captured cleared/replaced callback can be proven harmless.
    fn fire_unified_deadline_token_at(&self, token: u64, now: web_time::Instant) {
        let live_handle = {
            let arm = self.deadline.active_arm.borrow();
            arm.as_ref()
                .filter(|arm| arm.token == token)
                .map(|arm| arm.handle.clone())
        };
        if let Some(handle) = live_handle {
            let _ignored = self
                .deadline
                .primitives
                .clear_timeout
                .call1(&self.deadline.primitives.global, &handle);
        }
        self.deadline.deadline_callback_fired(token, now);
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use super::*;
    use beamr::atom::Atom;
    use beamr::constant_pool::ConstantPool;
    use beamr::loader::decode::compact::Operand;
    use beamr::loader::{Instruction, LambdaEntry, LineInfo, Literal};
    use beamr::module::{Module, ModuleOrigin, ResolvedImport, ResolvedImportTarget};
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

    fn build_module(
        atoms: &AtomTable,
        name: &str,
        exports: &[(&str, u8, u32)],
        code: Vec<Instruction>,
        resolved_imports: Vec<ResolvedImport>,
    ) -> Module {
        let label_index = code
            .iter()
            .enumerate()
            .filter_map(|(ip, instruction)| match instruction {
                Instruction::Label { label } => Some((*label, ip)),
                _ => None,
            })
            .collect();
        let mut export_map = HashMap::new();
        for (function, arity, label) in exports {
            export_map.insert((atoms.intern(function), *arity), *label);
        }
        Module {
            name: atoms.intern(name),
            generation: 0,
            origin: ModuleOrigin::Preloaded,
            exports: export_map,
            label_index,
            code,
            function_table: Vec::new(),
            line_table: Vec::new(),
            literals: Vec::<Literal>::new(),
            constant_pool: ConstantPool::new(),
            resolved_imports,
            lambdas: Vec::<LambdaEntry>::new(),
            string_table: Vec::new(),
            line_info: Vec::<LineInfo>::new(),
        }
    }

    /// `run/0`: receive one message and exit with it, or exit with `timed_out`
    /// after `milliseconds` (the receive-after class of the deadline service).
    fn receive_after_module(
        atoms: &AtomTable,
        name: &str,
        milliseconds: u64,
    ) -> (Atom, Atom, Module) {
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
                timeout: Operand::Unsigned(milliseconds),
            },
            Instruction::Timeout,
            Instruction::Move {
                source: Operand::Atom(Some(timed_out)),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ];
        let module = build_module(atoms, name, &[("run", 0, 1)], code, Vec::new());
        (module.name, atoms.intern("run"), module)
    }

    /// `run/0`: park until one message arrives, then exit with it (a plain
    /// receive with no timeout — the target of native `Deliver` timers).
    fn receive_one_module(atoms: &AtomTable, name: &str) -> (Atom, Atom, Module) {
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
            Instruction::Wait {
                fail: Operand::Label(10),
            },
        ];
        let module = build_module(atoms, name, &[("run", 0, 1)], code, Vec::new());
        (module.name, atoms.intern("run"), module)
    }

    /// Resolve one registered `erlang` BIF into a bytecode import entry, the
    /// way the loader would.
    fn erlang_bif_import(vm: &WasmVm, function: &str, arity: u8) -> ResolvedImport {
        let erlang = vm.atom_table.intern("erlang");
        let function_atom = vm.atom_table.intern(function);
        let entry = vm
            .bif_registry
            .lookup(erlang, function_atom, arity)
            .expect("gate-1 timer BIF is registered");
        ResolvedImport {
            module: erlang,
            function: function_atom,
            arity,
            target: ResolvedImportTarget::Native(entry),
        }
    }

    /// Real-bytecode timer BIF module (WPORT-3 R2): every function executes
    /// through `run_with_native_services` under the cooperative scheduler.
    ///
    /// - `arm_send/3` (`Pid, DelayMs, Msg`): `erlang:send_after/3`, exits with
    ///   the returned reference id.
    /// - `start_probe/0`: `erlang:start_timer(180_000, self(), 77)` then
    ///   `erlang:cancel_timer/1` on the returned reference; exits with the
    ///   remaining milliseconds.
    /// - `start_wait/0`: `erlang:start_timer(90_000, self(), 77)`, then parks
    ///   in a receive and exits with the delivered `{timeout, Ref, Msg}`.
    /// - `cancel_ref/1` (`RefId`): `erlang:cancel_timer/1`; exits with the
    ///   remaining milliseconds or `false`.
    fn timer_bif_module(vm: &WasmVm, name: &str) -> Module {
        let imports = vec![
            erlang_bif_import(vm, "self", 0),
            erlang_bif_import(vm, "send_after", 3),
            erlang_bif_import(vm, "start_timer", 3),
            erlang_bif_import(vm, "cancel_timer", 1),
        ];
        let code = vec![
            // arm_send/3: x0 = target pid, x1 = delay ms, x2 = message.
            Instruction::Label { label: 1 },
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
            // start_probe/0.
            Instruction::Label { label: 2 },
            Instruction::CallExt {
                arity: Operand::Unsigned(0),
                import: Operand::Unsigned(0),
            },
            Instruction::Move {
                source: Operand::X(0),
                destination: Operand::X(1),
            },
            Instruction::Move {
                source: Operand::Integer(180_000),
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
            Instruction::CallExt {
                arity: Operand::Unsigned(1),
                import: Operand::Unsigned(3),
            },
            Instruction::Return,
            // start_wait/0.
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
            // cancel_ref/1: x0 = timer reference id.
            Instruction::Label { label: 4 },
            Instruction::CallExt {
                arity: Operand::Unsigned(1),
                import: Operand::Unsigned(3),
            },
            Instruction::Return,
        ];
        build_module(
            &vm.atom_table,
            name,
            &[
                ("arm_send", 3, 1),
                ("start_probe", 0, 2),
                ("start_wait", 0, 3),
                ("cancel_ref", 1, 4),
            ],
            code,
            imports,
        )
    }

    fn spawn_bytecode(vm: &mut WasmVm, module: Atom, function: Atom, args: Vec<OwnedTerm>) -> u64 {
        let pid = vm
            .scheduler
            .borrow_mut()
            .spawn_owned(module, function, args)
            .expect("bytecode process spawns");
        vm.schedule_external_edge()
            .expect("bytecode spawn schedules the arbiter");
        pid
    }

    async fn await_exit_json(vm: &mut WasmVm, pid: u64) -> Value {
        parse_json(
            JsFuture::from(vm.await_exit(pid))
                .await
                .expect("await_exit resolves"),
        )
    }

    /// Race `completion` against one fresh macrotask; true when the completion
    /// settles first.
    async fn resolves_before_macrotask(completion: &Promise) -> bool {
        let marker = JsValue::from_str("macrotask");
        let race = Promise::race(&Array::of2(
            completion.as_ref(),
            timeout_value(0, marker.clone()).as_ref(),
        ));
        let first = JsFuture::from(race).await.expect("race resolves");
        first.as_string() != marker.as_string()
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
        let deadline_before = vm.unified_deadline_snapshot();
        assert_eq!(deadline_before.queued_now, 0);

        host_macrotask().await;

        let after = vm.arbiter_counters();
        assert_eq!(after.arbiter.requests, before.arbiter.requests);
        assert_eq!(after.arbiter.executions, before.arbiter.executions);
        assert_eq!(after.arbiter.queued_now, 0);
        assert_eq!(vm.unified_deadline_snapshot(), deadline_before);

        // Strengthened (WPORT-3 R4): TRUE IDLE with one future deadline keeps
        // exactly one COUNTED armed one-shot — `queued_now = 1` with a stable
        // armed deadline — while an intervening host macrotask causes zero new
        // arms, zero executions, and zero arbiter churn. The permitted
        // one-shot is counted, never subtracted or ignored.
        let (module, function, definition) =
            receive_after_module(&vm.atom_table, "wport3_idle_deadline", 3_600_000);
        vm.module_registry.insert(definition);
        let _pid = spawn_bytecode(&mut vm, module, function, Vec::new());
        host_macrotask().await;
        let armed = vm.unified_deadline_snapshot();
        assert_eq!(armed.queued_now, 1, "the armed one-shot is counted");
        assert_eq!(armed.requests, 1);
        assert_eq!(armed.executions, 0);
        assert!(armed.armed_deadline.is_some());
        let arbiter_armed = vm.arbiter_counters();

        host_macrotask().await;

        assert_eq!(
            vm.unified_deadline_snapshot(),
            armed,
            "an intervening macrotask arms nothing new and keeps the armed deadline stable"
        );
        let arbiter_after = vm.arbiter_counters();
        assert_eq!(
            arbiter_after.arbiter.requests,
            arbiter_armed.arbiter.requests
        );
        assert_eq!(
            arbiter_after.arbiter.executions,
            arbiter_armed.arbiter.executions
        );
        assert_eq!(arbiter_after.arbiter.queued_now, 0);
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

        // WPORT-3 R1: `clearTimeout` is constructor-probed alongside the other
        // hybrid primitives — the former per-call silent fallback is retired.
        let clear_key = JsValue::from_str("clearTimeout");
        let clear_timeout = Reflect::get(&global, &clear_key).expect("clearTimeout exists");
        assert!(Reflect::delete_property(&global, &clear_key).expect("clearTimeout deletes"));
        let clear_error = match WasmVm::new() {
            Ok(_) => String::from("constructor unexpectedly succeeded"),
            Err(error) => error.as_string().unwrap_or_default(),
        };
        Reflect::set(&global, &clear_key, &clear_timeout).expect("clearTimeout restores");
        assert!(clear_error.contains("clearTimeout"), "{clear_error}");
    }

    #[wasm_bindgen_test]
    async fn await_exit_waits_for_armed_receive_timer() {
        let mut vm = WasmVm::new().expect("VM constructs");
        let (module, function, definition) =
            receive_after_module(&vm.atom_table, "wport2_receive_after", 25);
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
        let armed = vm.unified_deadline_snapshot();
        assert_eq!(armed.queued_now, 1);
        assert!(armed.armed_deadline.is_some());

        let settled = parse_json(
            JsFuture::from(completion)
                .await
                .expect("receive timer fires and target exits"),
        );
        assert_eq!(settled["state"], "exited");
        assert_eq!(settled["pid"], pid);
        assert_eq!(settled["result"], "timed_out");
        let counters = vm.unified_deadline_snapshot();
        assert_eq!(counters.requests, 1);
        assert_eq!(counters.executions, 1);
        assert_eq!(counters.queued_now, 0);
        assert_eq!(counters.armed_deadline, None);
        assert_eq!(counters.next_arms, 1);
        assert_eq!(counters.cancellations, 0);
    }

    #[wasm_bindgen_test]
    async fn unified_deadline_keeps_one_host_callback_for_mixed_timer_burst() {
        let mut vm = WasmVm::new().expect("VM constructs");
        assert_eq!(vm.unified_deadline_snapshot().queued_now, 0);

        // Receive class: a far-future receive-after arms the first one-shot.
        let (module_a, function_a, definition_a) =
            receive_after_module(&vm.atom_table, "wport3_card_recv_a", 3_600_000);
        vm.module_registry.insert(definition_a);
        let _pid_a = spawn_bytecode(&mut vm, module_a, function_a, Vec::new());
        host_macrotask().await;
        let first = vm.unified_deadline_snapshot();
        assert_eq!(first.queued_now, 1);
        assert_eq!(first.requests, 1);
        assert_eq!(first.next_arms, 1);
        assert!(first.armed_deadline.is_some(), "one opaque live handle");

        // Native class: bytecode `send_after/3` schedules an EARLIER native
        // `Deliver`; the service re-arms the same single callback.
        let (recv_module, recv_function, recv_definition) =
            receive_one_module(&vm.atom_table, "wport3_card_w");
        vm.module_registry.insert(recv_definition);
        let target = spawn_bytecode(&mut vm, recv_module, recv_function, Vec::new());
        host_macrotask().await;
        let timers = timer_bif_module(&vm, "wport3_card_timers");
        let timers_name = timers.name;
        vm.module_registry.insert(timers);
        let arm_send = vm.atom_table.intern("arm_send");
        let message = vm.atom_table.intern("msg_native");
        let _armer = spawn_bytecode(
            &mut vm,
            timers_name,
            arm_send,
            vec![
                OwnedTerm::immediate(Term::pid(target)),
                OwnedTerm::immediate(Term::small_int(1_800_000)),
                OwnedTerm::immediate(Term::atom(message)),
            ],
        );
        host_macrotask().await;
        let second = vm.unified_deadline_snapshot();
        assert_eq!(
            second.queued_now, 1,
            "a mixed burst never creates a second deadline callback"
        );
        assert_eq!(second.rearms_earlier, 1);
        assert!(second.armed_deadline < first.armed_deadline);

        // A later receive deadline is a counted no-op: still one callback.
        let (module_b, function_b, definition_b) =
            receive_after_module(&vm.atom_table, "wport3_card_recv_b", 7_200_000);
        vm.module_registry.insert(definition_b);
        let _pid_b = spawn_bytecode(&mut vm, module_b, function_b, Vec::new());
        host_macrotask().await;
        let third = vm.unified_deadline_snapshot();
        assert_eq!(third.queued_now, 1);
        assert_eq!(third.requests, second.requests);
        assert_eq!(third.later_noops, second.later_noops + 1);
        assert_eq!(third.armed_deadline, second.armed_deadline);
    }

    #[wasm_bindgen_test]
    async fn earlier_deadline_cancels_and_rearms_the_unified_callback() {
        let mut vm = WasmVm::new().expect("VM constructs");
        let (module_late, function_late, definition_late) =
            receive_after_module(&vm.atom_table, "wport3_rearm_late", 7_200_000);
        vm.module_registry.insert(definition_late);
        let _late = spawn_bytecode(&mut vm, module_late, function_late, Vec::new());
        host_macrotask().await;
        let armed = vm.unified_deadline_snapshot();
        assert_eq!(armed.queued_now, 1);
        assert_eq!(armed.requests, 1);
        assert_eq!(armed.cancellations, 0);
        assert_eq!(armed.rearms_earlier, 0);

        let (module_early, function_early, definition_early) =
            receive_after_module(&vm.atom_table, "wport3_rearm_early", 60_000);
        vm.module_registry.insert(definition_early);
        let _early = spawn_bytecode(&mut vm, module_early, function_early, Vec::new());
        host_macrotask().await;
        let rearmed = vm.unified_deadline_snapshot();
        assert_eq!(rearmed.requests, 2, "exactly one new arm");
        assert_eq!(rearmed.cancellations, 1, "exactly one clear");
        assert_eq!(rearmed.rearms_earlier, 1);
        assert_eq!(rearmed.queued_now, 1, "no duplicate queued callback");
        assert!(rearmed.armed_deadline < armed.armed_deadline);
    }

    #[wasm_bindgen_test]
    async fn cancelling_earliest_deadline_moves_or_clears_the_unified_callback() {
        let mut vm = WasmVm::new().expect("VM constructs");
        let (module_t1, function_t1, definition_t1) =
            receive_after_module(&vm.atom_table, "wport3_cancel_t1", 1_800_000);
        vm.module_registry.insert(definition_t1);
        let earliest = spawn_bytecode(&mut vm, module_t1, function_t1, Vec::new());
        host_macrotask().await;
        let (module_t2, function_t2, definition_t2) =
            receive_after_module(&vm.atom_table, "wport3_cancel_t2", 7_200_000);
        vm.module_registry.insert(definition_t2);
        let latest = spawn_bytecode(&mut vm, module_t2, function_t2, Vec::new());
        host_macrotask().await;
        let armed = vm.unified_deadline_snapshot();
        assert_eq!(armed.queued_now, 1);
        let stale_token = vm.unified_arm_token().expect("an arm is active");

        // A message wins the earliest receive: its record leaves the map and
        // the armed snapshot MOVES to the next minimum.
        vm.send_message(earliest, JsValue::from_f64(5.0))
            .expect("host send succeeds");
        let moved = vm.unified_deadline_snapshot();
        assert_eq!(
            moved.cancel_moves_or_clears,
            armed.cancel_moves_or_clears + 1
        );
        assert_eq!(moved.cancellations, armed.cancellations + 1);
        assert_eq!(moved.requests, armed.requests + 1);
        assert!(moved.armed_deadline > armed.armed_deadline);
        assert_eq!(moved.queued_now, 1);
        let arbiter_after_move = vm.arbiter_counters();

        // A racing stale T1 callback is rejected by the local arm token: no
        // delivery, no arbiter request, no state or counter mutation.
        vm.fire_unified_deadline_token_at(
            stale_token,
            web_time::Instant::now() + Duration::from_secs(36_000),
        );
        assert_eq!(vm.unified_deadline_snapshot(), moved);
        assert_eq!(vm.arbiter_counters(), arbiter_after_move);

        host_macrotask().await;
        let won = await_exit_json(&mut vm, earliest).await;
        assert_eq!(won["state"], "exited");
        assert_eq!(won["result"], 5.0);

        // Cancelling the now-earliest (and only) deadline CLEARS the arm.
        let before_clear = vm.unified_deadline_snapshot();
        vm.send_message(latest, JsValue::from_f64(6.0))
            .expect("host send succeeds");
        let cleared = vm.unified_deadline_snapshot();
        assert_eq!(
            cleared.cancel_moves_or_clears,
            before_clear.cancel_moves_or_clears + 1
        );
        assert_eq!(cleared.cancellations, before_clear.cancellations + 1);
        assert_eq!(
            cleared.requests, before_clear.requests,
            "clear arms nothing"
        );
        assert_eq!(cleared.queued_now, 0);
        assert_eq!(cleared.armed_deadline, None);
        host_macrotask().await;
    }

    #[wasm_bindgen_test]
    async fn later_deadline_does_not_rearm_the_unified_callback() {
        let mut vm = WasmVm::new().expect("VM constructs");
        let (module_t1, function_t1, definition_t1) =
            receive_after_module(&vm.atom_table, "wport3_later_t1", 3_600_000);
        vm.module_registry.insert(definition_t1);
        let _t1 = spawn_bytecode(&mut vm, module_t1, function_t1, Vec::new());
        host_macrotask().await;
        let first = vm.unified_deadline_snapshot();
        assert_eq!(first.queued_now, 1);
        assert_eq!(first.requests, 1);

        let (module_t3, function_t3, definition_t3) =
            receive_after_module(&vm.atom_table, "wport3_later_t3", 10_800_000);
        vm.module_registry.insert(definition_t3);
        let _t3 = spawn_bytecode(&mut vm, module_t3, function_t3, Vec::new());
        host_macrotask().await;
        let second = vm.unified_deadline_snapshot();
        assert_eq!(second.requests, first.requests, "no new request");
        assert_eq!(second.cancellations, first.cancellations, "no cancellation");
        assert_eq!(second.later_noops, first.later_noops + 1);
        assert_eq!(second.armed_deadline, first.armed_deadline);
        assert_eq!(second.queued_now, 1);
    }

    #[wasm_bindgen_test]
    async fn late_fire_delivers_all_due_timer_classes_queues_one_turn_and_arms_next() {
        let mut vm = WasmVm::new().expect("VM constructs");
        let (module_a, function_a, definition_a) =
            receive_after_module(&vm.atom_table, "wport3_fire_a", 25_000);
        vm.module_registry.insert(definition_a);
        let pid_a = spawn_bytecode(&mut vm, module_a, function_a, Vec::new());
        host_macrotask().await;
        let (module_b, function_b, definition_b) =
            receive_after_module(&vm.atom_table, "wport3_fire_b", 50_000);
        vm.module_registry.insert(definition_b);
        let pid_b = spawn_bytecode(&mut vm, module_b, function_b, Vec::new());
        host_macrotask().await;
        let (module_c, function_c, definition_c) =
            receive_after_module(&vm.atom_table, "wport3_fire_c", 3_600_000);
        vm.module_registry.insert(definition_c);
        let pid_c = spawn_bytecode(&mut vm, module_c, function_c, Vec::new());
        host_macrotask().await;
        let (recv_module, recv_function, recv_definition) =
            receive_one_module(&vm.atom_table, "wport3_fire_w");
        vm.module_registry.insert(recv_definition);
        let target = spawn_bytecode(&mut vm, recv_module, recv_function, Vec::new());
        host_macrotask().await;
        let timers = timer_bif_module(&vm, "wport3_fire_timers");
        let timers_name = timers.name;
        vm.module_registry.insert(timers);
        let arm_send = vm.atom_table.intern("arm_send");
        let message = vm.atom_table.intern("msg_native");
        let armer = spawn_bytecode(
            &mut vm,
            timers_name,
            arm_send,
            vec![
                OwnedTerm::immediate(Term::pid(target)),
                OwnedTerm::immediate(Term::small_int(75_000)),
                OwnedTerm::immediate(Term::atom(message)),
            ],
        );
        host_macrotask().await;
        let armer_exit = await_exit_json(&mut vm, armer).await;
        assert_eq!(armer_exit["state"], "exited");
        assert!(armer_exit["result"].as_i64().unwrap_or(0) >= 1);

        let arbiter_before = vm.arbiter_counters();
        let before = vm.unified_deadline_snapshot();
        assert_eq!(before.queued_now, 1);
        let completion_a = vm.await_exit(pid_a);
        let completion_b = vm.await_exit(pid_b);
        let completion_c = vm.await_exit(pid_c);
        let completion_w = vm.await_exit(target);

        // One admitted fire, arbitrarily late: 200s is past the 25s/50s
        // receive deadlines and the 75s native deadline, before the 3600s one.
        let fire_at = web_time::Instant::now() + Duration::from_millis(200_000);
        vm.fire_unified_deadline_at(fire_at);

        let fired = vm.unified_deadline_snapshot();
        assert_eq!(fired.executions, before.executions + 1);
        assert_eq!(
            fired.queued_now, 0,
            "the active arm is consumed before delivery settles"
        );
        let arbiter_queued = vm.arbiter_counters();
        assert_eq!(
            arbiter_queued.arbiter.requests,
            arbiter_before.arbiter.requests + 1,
            "one arbiter turn for the complete due set, never one per expiry"
        );
        assert_eq!(arbiter_queued.arbiter.queued_now, 1);

        host_macrotask().await;

        let exited_a = parse_json(
            JsFuture::from(completion_a)
                .await
                .expect("due receive timer A resolves"),
        );
        assert_eq!(exited_a["state"], "exited");
        assert_eq!(exited_a["result"], "timed_out");
        let exited_b = parse_json(
            JsFuture::from(completion_b)
                .await
                .expect("due receive timer B resolves"),
        );
        assert_eq!(exited_b["state"], "exited");
        assert_eq!(exited_b["result"], "timed_out");
        let exited_w = parse_json(
            JsFuture::from(completion_w)
                .await
                .expect("due native Deliver resolves"),
        );
        assert_eq!(exited_w["state"], "exited");
        assert_eq!(exited_w["result"], "msg_native");

        let after = vm.unified_deadline_snapshot();
        assert_eq!(after.next_arms, before.next_arms + 1);
        assert_eq!(after.queued_now, 1);
        let armed = after
            .armed_deadline
            .expect("the next deadline is armed after settle");
        assert!(armed > fire_at, "the not-yet-due entry is the next arm");
        let arbiter_after = vm.arbiter_counters();
        assert_eq!(
            arbiter_after.arbiter.requests,
            arbiter_before.arbiter.requests + 1
        );
        assert_eq!(
            arbiter_after.arbiter.executions,
            arbiter_before.arbiter.executions + 1
        );
        assert!(
            !resolves_before_macrotask(&completion_c).await,
            "the not-yet-due entry remains pending"
        );
    }

    #[wasm_bindgen_test]
    async fn await_exit_waits_for_armed_native_deliver_timer() {
        let mut vm = WasmVm::new().expect("VM constructs");
        let (recv_module, recv_function, recv_definition) =
            receive_one_module(&vm.atom_table, "wport3_native_w");
        vm.module_registry.insert(recv_definition);
        let target = spawn_bytecode(&mut vm, recv_module, recv_function, Vec::new());
        host_macrotask().await;
        let timers = timer_bif_module(&vm, "wport3_native_timers");
        let timers_name = timers.name;
        vm.module_registry.insert(timers);
        let arm_send = vm.atom_table.intern("arm_send");
        let message = vm.atom_table.intern("done");
        let armer = spawn_bytecode(
            &mut vm,
            timers_name,
            arm_send,
            vec![
                OwnedTerm::immediate(Term::pid(target)),
                OwnedTerm::immediate(Term::small_int(600_000)),
                OwnedTerm::immediate(Term::atom(message)),
            ],
        );
        host_macrotask().await;
        let armer_exit = await_exit_json(&mut vm, armer).await;
        assert_eq!(armer_exit["state"], "exited");
        assert!(armer_exit["result"].as_i64().unwrap_or(0) >= 1);

        // TRUE IDLE with a pending native Deliver is NOT settled idle: the
        // Promise must stay pending rather than resolve 'idle'.
        let completion = vm.await_exit(target);
        assert!(
            !resolves_before_macrotask(&completion).await,
            "await_exit must not resolve while a native Deliver is armed"
        );
        let armed = vm.unified_deadline_snapshot();
        assert_eq!(armed.queued_now, 1);
        assert!(armed.armed_deadline.is_some());

        vm.fire_unified_deadline_at(web_time::Instant::now() + Duration::from_millis(900_000));
        host_macrotask().await;
        let settled = parse_json(
            JsFuture::from(completion)
                .await
                .expect("deadline delivery drives the target to exited"),
        );
        assert_eq!(settled["state"], "exited");
        assert_eq!(settled["pid"], target);
        assert_eq!(settled["result"], "done");
        let after = vm.unified_deadline_snapshot();
        assert_eq!(after.executions, armed.executions + 1);
        assert_eq!(after.queued_now, 0);
        assert_eq!(after.armed_deadline, None);
    }

    #[wasm_bindgen_test]
    async fn cooperative_bytecode_timer_bifs_round_trip_and_arm_unified_deadline() {
        let mut vm = WasmVm::new().expect("VM constructs");
        let (w1_module, w1_function, w1_definition) =
            receive_one_module(&vm.atom_table, "wport3_bif_w1");
        vm.module_registry.insert(w1_definition);
        let w1 = spawn_bytecode(&mut vm, w1_module, w1_function, Vec::new());
        let (w2_module, w2_function, w2_definition) =
            receive_one_module(&vm.atom_table, "wport3_bif_w2");
        vm.module_registry.insert(w2_definition);
        let w2 = spawn_bytecode(&mut vm, w2_module, w2_function, Vec::new());
        host_macrotask().await;
        let timers = timer_bif_module(&vm, "wport3_bif_timers");
        let timers_name = timers.name;
        vm.module_registry.insert(timers);
        let arm_send = vm.atom_table.intern("arm_send");
        let start_probe = vm.atom_table.intern("start_probe");
        let start_wait = vm.atom_table.intern("start_wait");
        let cancel_ref = vm.atom_table.intern("cancel_ref");
        let msg_a = vm.atom_table.intern("msg_a");
        let msg_b = vm.atom_table.intern("msg_b");

        // send_after/3 returns a reference; no badarg/missing-service refusal.
        let armer_a = spawn_bytecode(
            &mut vm,
            timers_name,
            arm_send,
            vec![
                OwnedTerm::immediate(Term::pid(w1)),
                OwnedTerm::immediate(Term::small_int(120_000)),
                OwnedTerm::immediate(Term::atom(msg_a)),
            ],
        );
        host_macrotask().await;
        let armer_a_exit = await_exit_json(&mut vm, armer_a).await;
        assert_eq!(
            armer_a_exit["state"], "exited",
            "send_after must not refuse: {armer_a_exit}"
        );
        let reference_a = armer_a_exit["result"]
            .as_i64()
            .expect("send_after returns a reference id");
        assert!(reference_a >= 1);
        // The bytecode-scheduled future timer is the unified minimum and arms
        // the one host callback.
        let first = vm.unified_deadline_snapshot();
        assert_eq!(first.queued_now, 1);
        assert_eq!(first.requests, 1);
        assert_eq!(first.next_arms, 1);
        assert!(first.armed_deadline.is_some());

        // start_timer/3 to self, earlier: the same single callback re-arms.
        let waiter = spawn_bytecode(&mut vm, timers_name, start_wait, Vec::new());
        host_macrotask().await;
        let second = vm.unified_deadline_snapshot();
        assert_eq!(second.queued_now, 1);
        assert_eq!(second.rearms_earlier, first.rearms_earlier + 1);
        assert!(second.armed_deadline < first.armed_deadline);

        // A second, LATER BIF timer does not create a second callback.
        let armer_b = spawn_bytecode(
            &mut vm,
            timers_name,
            arm_send,
            vec![
                OwnedTerm::immediate(Term::pid(w2)),
                OwnedTerm::immediate(Term::small_int(240_000)),
                OwnedTerm::immediate(Term::atom(msg_b)),
            ],
        );
        host_macrotask().await;
        let armer_b_exit = await_exit_json(&mut vm, armer_b).await;
        assert_eq!(armer_b_exit["state"], "exited");
        let reference_b = armer_b_exit["result"]
            .as_i64()
            .expect("send_after returns a reference id");
        assert!(reference_b >= 1);
        let third = vm.unified_deadline_snapshot();
        assert_eq!(third.queued_now, 1, "no second host callback");
        assert_eq!(third.requests, second.requests, "no new host arm");
        assert_eq!(third.cancellations, second.cancellations, "no clear");
        assert_eq!(third.armed_deadline, second.armed_deadline);

        // start_timer/3 returns a reference usable by cancel_timer/1, which
        // reports the remaining milliseconds for a pending reference.
        let probe = spawn_bytecode(&mut vm, timers_name, start_probe, Vec::new());
        host_macrotask().await;
        let probe_exit = await_exit_json(&mut vm, probe).await;
        assert_eq!(probe_exit["state"], "exited");
        let probe_remaining = probe_exit["result"]
            .as_i64()
            .expect("cancel_timer returns remaining milliseconds");
        assert!(probe_remaining > 0 && probe_remaining <= 180_000);

        // cancel_timer/1 on the pending send_after reference: remaining ms,
        // then `false` on the second cancel.
        let cancel_one = spawn_bytecode(
            &mut vm,
            timers_name,
            cancel_ref,
            vec![OwnedTerm::immediate(Term::small_int(reference_a))],
        );
        host_macrotask().await;
        let cancel_one_exit = await_exit_json(&mut vm, cancel_one).await;
        let remaining_a = cancel_one_exit["result"]
            .as_i64()
            .expect("first cancel returns remaining milliseconds");
        assert!(remaining_a > 0 && remaining_a <= 120_000);
        let cancel_two = spawn_bytecode(
            &mut vm,
            timers_name,
            cancel_ref,
            vec![OwnedTerm::immediate(Term::small_int(reference_a))],
        );
        host_macrotask().await;
        let cancel_two_exit = await_exit_json(&mut vm, cancel_two).await;
        assert_eq!(cancel_two_exit["result"], Value::Bool(false));

        // Fire past the start_timer deadline: exact {timeout, Ref, Msg}
        // delivery, while the cancelled send_after target stays undisturbed.
        let w1_completion = vm.await_exit(w1);
        vm.fire_unified_deadline_at(web_time::Instant::now() + Duration::from_millis(150_000));
        host_macrotask().await;
        let waiter_exit = await_exit_json(&mut vm, waiter).await;
        assert_eq!(waiter_exit["state"], "exited");
        let tuple = waiter_exit["result"]
            .as_array()
            .expect("start_timer delivers the {timeout, Ref, Msg} tuple");
        assert_eq!(tuple.len(), 3);
        assert_eq!(tuple[0], "timeout");
        assert!(tuple[1].as_i64().unwrap_or(0) >= 1);
        assert_eq!(tuple[2], 77);
        assert!(
            !resolves_before_macrotask(&w1_completion).await,
            "cancel-before-fire suppresses delivery"
        );

        // Fire past the remaining send_after deadline: the ORIGINAL message
        // is delivered.
        vm.fire_unified_deadline_at(web_time::Instant::now() + Duration::from_millis(400_000));
        host_macrotask().await;
        let w2_exit = await_exit_json(&mut vm, w2).await;
        assert_eq!(w2_exit["state"], "exited");
        assert_eq!(w2_exit["result"], "msg_b");

        // With every deadline consumed or cancelled, the suppressed target
        // settles idle: msg_a was never delivered.
        let w1_settled = parse_json(
            JsFuture::from(w1_completion)
                .await
                .expect("settled idle resolves the suppressed target"),
        );
        assert_eq!(w1_settled["state"], "idle", "cancel-before-fire held");

        // cancel-after-fire returns `false` and cannot retract the already
        // delivered message (w2's exit result above stays `msg_b`).
        let cancel_after_fire = spawn_bytecode(
            &mut vm,
            timers_name,
            cancel_ref,
            vec![OwnedTerm::immediate(Term::small_int(reference_b))],
        );
        host_macrotask().await;
        let cancel_after_fire_exit = await_exit_json(&mut vm, cancel_after_fire).await;
        assert_eq!(cancel_after_fire_exit["result"], Value::Bool(false));
    }
}
