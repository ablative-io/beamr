//! The typed scheduler failure surface (WPORT-7 R1) and the browser panic
//! surface (WPORT-7 R2).
//!
//! # `SchedulerFailureError`
//!
//! Every scheduler/arbiter failure — a host turn-queue primitive throwing, or
//! the completed-drain deadline-reconcile seam throwing — converges on ONE
//! named error class: a `js_sys::Error` named `SchedulerFailureError` with a
//! `"{leg}: {detail}"` message and one `data` property holding the JSON
//! string `{"leg":…,"phase":…,"terminal":true}` (the WPORT-6
//! `ArtifactLoadError` async-operational pattern; payload mechanics reuse the
//! OQ-D-ratified `data`-property shape). The leg slug is the message's kind
//! position; no separate kind vocabulary exists. The closed leg set is
//! [`LEG_SLUGS`], pinned by a wall.
//!
//! The class is the latch's stored value (parked `await_exit` waiters reject
//! WITH it), the manual-drain `Err` returned to the caller, the fallible wake
//! path's thrown value, and the payload behind both observability surfaces
//! (`WasmVm::terminal_error`, `WasmVm::register_failure_callback`). WPORT-2
//! Ruling 3 is preserved verbatim — "Wrapper/scheduler/arbiter errors reject
//! the Promise." — the rejection SEMANTICS are unchanged; only the rejected
//! VALUE is typed (OQ-A ruled ADDITIVE).
//!
//! The latch is permanent by ruled design: `terminal` is literal `true`,
//! there is no clear/reset API, and no configuration surface of any kind
//! exists on this seam (the no-knob law, WPORT-2 Ruling 2).
//!
//! # Panic surface (reporting-only)
//!
//! A process-global `std::panic::set_hook` hook, installed once (guarded by a
//! [`Once`]) at `WasmVm::new`, hand-rolled over existing js-sys — no new
//! dependency (the crates.io panic-hook helper crate was REJECTED at the
//! outline; it is absent from Cargo.lock and stays absent). The hook body
//! formats the panic message + location, writes it through `console.error`
//! ALWAYS, then invokes the optionally registered plain-JS callback from the
//! process-global slot ([`register_panic_callback`], last-wins). The hook is
//! reporting-only: it performs no unwind interception, no resume, no waiter
//! rejection, and touches NO VM state — the panic may hold any VM `RefCell`
//! borrow, so reaching one from the hook would itself trap (pack hazard 10).
//!
//! # Recovery contract (ruled honesty, WPORT-7 D7)
//!
//! post-panic the instance is latched (borrowed RefCells, stuck Draining);
//! every scheduler-touching call re-traps; construct a fresh WasmVm.
//!
//! The arbiter's error machinery never observes a panic — `fail()` runs only
//! on `JsValue` `Err` returns, so after a panic `last_error` stays `None` and
//! parked waiters hang. This module documents that honestly rather than
//! pretending the latch covers it; the panic callback and `console.error`
//! line are the observable surface, and a fresh `WasmVm` (on a fresh
//! isolate/worker where the host caches VMs) is the only recovery.

use std::cell::RefCell;
use std::sync::Once;

use js_sys::{Function, Reflect};
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// The closed leg-slug set carried in the `data` payload's `leg` position
/// (WPORT-7 D2), pinned by a wall exactly as the WPORT-6 kind-slug set is.
pub(crate) const LEG_SLUGS: [&str; 5] = ["queued", "manual", "deadline", "promise", "spawn_edge"];

/// Phase slug for the completed-drain deadline-reconcile seam — the sole
/// fallible drain operation (the one `?` in `perform_drain`).
pub(crate) const PHASE_RECONCILE: &str = "reconcile";
/// Phase slug for a throwing `queueMicrotask` turn-queue primitive.
pub(crate) const PHASE_QUEUE_MICROTASK: &str = "queue_microtask";
/// Phase slug for a throwing `setTimeout` turn-queue primitive (the fairness
/// macrotask leg).
pub(crate) const PHASE_SET_TIMEOUT: &str = "set_timeout";

/// The failure leg a `SchedulerFailureError` is tagged with: which arbiter
/// entry point observed the failure. One closed set ([`LEG_SLUGS`]); the slug
/// doubles as the message's kind position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailureLeg {
    /// A queued-turn failure: the direct wake path (send/spawn/call/cast/
    /// timer-fired), the queued-turn drain, or the fairness re-queue.
    Queued,
    /// A manual `run_step` drain failure (the D1 wedge-fix leg).
    Manual,
    /// A wake failure inside the unified deadline late-fire callback.
    Deadline,
    /// A wake failure at async promise completion.
    Promise,
    /// A wake failure on the infallible spawn edge (`spawn_actor`), swallowed
    /// into the latch — no per-call surface exists on that path.
    SpawnEdge,
}

impl FailureLeg {
    pub(crate) fn slug(self) -> &'static str {
        // The variant order IS the `LEG_SLUGS` order — one closed set, one
        // wall; the enumeration test pins the pairing.
        LEG_SLUGS[self as usize]
    }
}

/// Mint the one named error class (WPORT-7 D2), byte-following the repo's
/// named typed-error precedent (`ArtifactLoadError`): a `js_sys::Error` named
/// `SchedulerFailureError` with a `"{leg}: {detail}"` message and one `data`
/// property holding the JSON string `{"leg":…,"phase":…,"terminal":true}`.
/// `phase` names the failing operation; `terminal` is literal `true` — the
/// latch is permanent by ruled design.
pub(crate) fn scheduler_failure_error(leg: FailureLeg, phase: &str, cause: &JsValue) -> JsValue {
    let slug = leg.slug();
    let detail = js_detail(cause);
    let error = js_sys::Error::new(&format!("{slug}: {detail}"));
    error.set_name("SchedulerFailureError");
    let data = json!({
        "leg": slug,
        "phase": phase,
        "terminal": true,
    });
    let _assigned = Reflect::set(
        error.as_ref(),
        &JsValue::from_str("data"),
        &JsValue::from_str(&data.to_string()),
    );
    error.into()
}

/// Best-effort human detail for an arbitrary thrown `JsValue`: the string
/// itself, an object's `toString()`, or the Rust debug form as a last resort.
fn js_detail(value: &JsValue) -> String {
    if let Some(text) = value.as_string() {
        return text;
    }
    value
        .dyn_ref::<js_sys::Object>()
        .map(|object| String::from(object.to_string()))
        .unwrap_or_else(|| format!("{value:?}"))
}

static PANIC_HOOK: Once = Once::new();

#[cfg(all(test, target_arch = "wasm32"))]
static PANIC_HOOK_INSTALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

thread_local! {
    /// The process-global panic-callback slot (WPORT-7 D6). Process-global BY
    /// NECESSITY: the hook must not borrow VM RefCells (the panic may hold
    /// any of them), so no per-VM route exists. Last-wins on registration.
    static PANIC_CALLBACK: RefCell<Option<Function>> = const { RefCell::new(None) };
}

/// Install the process-global reporting-only panic hook exactly once
/// (WPORT-7 D5). Called from `WasmVm::new` (constructor precedent); any
/// number of constructions install once and leak nothing.
pub(crate) fn install_panic_hook_once() {
    PANIC_HOOK.call_once(|| {
        #[cfg(all(test, target_arch = "wasm32"))]
        PANIC_HOOK_INSTALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::panic::set_hook(Box::new(panic_hook_body));
    });
}

#[cfg(all(test, target_arch = "wasm32"))]
pub(crate) fn panic_hook_install_count() -> u32 {
    PANIC_HOOK_INSTALLS.load(std::sync::atomic::Ordering::Relaxed)
}

/// The hook body: format message + location, `console.error` ALWAYS, then the
/// optionally registered plain-JS callback. Reporting-only — zero VM-state
/// access, no waiter rejection, no counter writes, no unwind interception or
/// resume of any kind; the trap follows immediately after this returns.
fn panic_hook_body(info: &std::panic::PanicHookInfo<'_>) {
    let message = if let Some(text) = info.payload().downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = info.payload().downcast_ref::<String>() {
        text.clone()
    } else {
        String::from("non-string panic payload")
    };
    let location = info
        .location()
        .map(|location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        })
        .unwrap_or_else(|| String::from("unknown location"));
    let payload = format!("beamr-wasm panicked: {message} ({location})");
    console_error(&payload);
    // `try_with`/`try_borrow`: the hook must never itself panic, even if the
    // panic interrupted a callback registration mid-borrow.
    let callback = PANIC_CALLBACK
        .try_with(|slot| slot.try_borrow().ok().and_then(|slot| slot.clone()))
        .ok()
        .flatten();
    if let Some(callback) = callback {
        let _ignored = callback.call1(&JsValue::NULL, &JsValue::from_str(&payload));
    }
}

/// Register (or replace — last-wins) the process-global plain-JS panic
/// callback, invoked by the reporting-only panic hook with one string
/// argument (message + location) BEFORE the trap. `console.error` fires
/// regardless of registration. The slot is process-global, shared by every
/// `WasmVm` in the realm — see the module doc's recovery contract: post-panic
/// the panicking instance is bricked and must be replaced, so the callback is
/// a report channel, never a recovery channel.
#[wasm_bindgen]
pub fn register_panic_callback(callback: Function) {
    PANIC_CALLBACK.with(|slot| *slot.borrow_mut() = Some(callback));
}

/// `console.error` via js-sys `Reflect` against the global — deliberately
/// independent of the VM's io_sink (the hook may run while every VM borrow
/// is live). A host without a console (or with a non-function member) drops
/// the line silently; that host still gets the registered callback, if any.
fn console_error(text: &str) {
    let global = js_sys::global();
    let Ok(console) = Reflect::get(&global, &JsValue::from_str("console")) else {
        return;
    };
    let Ok(method) = Reflect::get(&console, &JsValue::from_str("error")) else {
        return;
    };
    let Ok(method) = method.dyn_into::<Function>() else {
        return;
    };
    let _ignored = method.call1(&console, &JsValue::from_str(text));
}

// ---------------------------------------------------------------------------
// Test support (cfg(test) only): throwing host-primitive doubles and typed-
// error introspection/assertion helpers for this module's contract, shared
// with the io_sink wall battery. Zero production surface.
// ---------------------------------------------------------------------------

#[cfg(all(test, target_arch = "wasm32"))]
mod test_support {
    use js_sys::{Function, Reflect};
    use serde_json::{Value, json};
    use wasm_bindgen::JsValue;

    use crate::WasmVm;

    /// A delegating throw-capable double over a named global host primitive,
    /// installed BEFORE `WasmVm::new` so the constructor probe captures it.
    pub(crate) struct PrimitiveDouble {
        name: &'static str,
        original: JsValue,
    }

    pub(crate) fn install_throwing_double(name: &'static str) -> PrimitiveDouble {
        let global = js_sys::global();
        let original = Reflect::get(&global, &JsValue::from_str(name))
            .expect("host primitive exists to double");
        let factory = Function::new_with_args(
            "original",
            &format!(
                "return function() {{ \
                     if (globalThis.__wport7_throw_{name}) {{ \
                         throw new Error('{name} injected throw'); \
                     }} \
                     return original.apply(this, arguments); \
                 }};"
            ),
        );
        let double = factory
            .call1(&JsValue::UNDEFINED, &original)
            .expect("throwing double constructs");
        Reflect::set(&global, &JsValue::from_str(name), &double).expect("throwing double installs");
        PrimitiveDouble { name, original }
    }

    /// A targeted `queueMicrotask` double: throws ONLY for the arbiter's own
    /// turn callback (identity-matched against `__wport7_arbiter_callback`),
    /// delegating every other enqueue — wasm-bindgen-futures schedules its task
    /// queue through the global `queueMicrotask`, and a blanket throw would crash
    /// the test harness's own machinery instead of exercising the arbiter seam.
    pub(crate) fn install_targeted_queue_microtask_double() -> PrimitiveDouble {
        let name = "queueMicrotask";
        let global = js_sys::global();
        let original = Reflect::get(&global, &JsValue::from_str(name))
            .expect("host primitive exists to double");
        let factory = Function::new_with_args(
            "original",
            "return function() { \
                 if (globalThis.__wport7_throw_queueMicrotask \
                     && arguments[0] === globalThis.__wport7_arbiter_callback) { \
                     throw new Error('queueMicrotask injected throw'); \
                 } \
                 return original.apply(this, arguments); \
             };",
        );
        let double = factory
            .call1(&JsValue::UNDEFINED, &original)
            .expect("throwing double constructs");
        Reflect::set(&global, &JsValue::from_str(name), &double).expect("throwing double installs");
        PrimitiveDouble { name, original }
    }

    /// Point the targeted double at this VM's arbiter turn callback.
    pub(crate) fn target_arbiter_callback(vm: &WasmVm) {
        let _set = Reflect::set(
            &js_sys::global(),
            &JsValue::from_str("__wport7_arbiter_callback"),
            vm.arbiter.callback.as_ref(),
        );
    }

    impl PrimitiveDouble {
        pub(crate) fn set_throwing(&self, throwing: bool) {
            let flag = format!("__wport7_throw_{}", self.name);
            let _set = Reflect::set(
                &js_sys::global(),
                &JsValue::from_str(&flag),
                &JsValue::from_bool(throwing),
            );
        }

        pub(crate) fn restore(self) {
            self.set_throwing(false);
            let _set = Reflect::set(
                &js_sys::global(),
                &JsValue::from_str(self.name),
                &self.original,
            );
        }
    }

    pub(crate) fn error_name(error: &JsValue) -> String {
        Reflect::get(error, &JsValue::from_str("name"))
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_default()
    }

    pub(crate) fn error_message(error: &JsValue) -> String {
        Reflect::get(error, &JsValue::from_str("message"))
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_default()
    }

    pub(crate) fn error_data(error: &JsValue) -> Value {
        let raw = Reflect::get(error, &JsValue::from_str("data")).expect("data property reads");
        serde_json::from_str(&raw.as_string().expect("data is a JSON string")).expect("data parses")
    }

    /// Assert the full typed shape: class name, `"{leg}: {detail}"` message, and
    /// the pinned three-key data schema with literal `terminal: true`.
    pub(crate) fn assert_typed(error: &JsValue, leg: &str, phase: &str) {
        assert_eq!(error_name(error), "SchedulerFailureError");
        let message = error_message(error);
        assert!(
            message.starts_with(&format!("{leg}: ")),
            "message carries the leg slug in kind position: {message}"
        );
        let data = error_data(error);
        assert_eq!(data["leg"], json!(leg));
        assert_eq!(data["phase"], json!(phase));
        assert_eq!(data["terminal"], json!(true));
        let keys: Vec<&str> = data
            .as_object()
            .expect("data is an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys.len(), 3, "exactly the pinned schema keys: {keys:?}");
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
pub(crate) use test_support::{
    PrimitiveDouble, assert_typed, error_data, error_name, install_targeted_queue_microtask_double,
    install_throwing_double, target_arbiter_callback,
};

#[cfg(all(test, target_arch = "wasm32"))]
#[path = "failure_tests.rs"]
mod tests;
