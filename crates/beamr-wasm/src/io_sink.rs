//! The cooperative host IO sink (WPORT-5 R2 item 4).
//!
//! The scheduler-facing half ([`CooperativeSinkBuffer`]) implements the
//! `Send + Sync` [`IoSink`] trait by capturing each tagged write into an
//! uncontended single-threaded buffer at the writing BIF — synchronous
//! write-through, PUSH-ONLY. The host-facing half ([`HostIoSinkBridge`])
//! delivers the captured writes to the registered JavaScript sink callback —
//! or, with zero configuration, to the browser console (OQ2 ruled:
//! `console.log` for out, `console.error` for err) — once per host turn,
//! after the drain has settled and EVERY scheduler borrow has dropped (the
//! lib.rs borrow discipline, binding on the sink exactly as the WPORT-3 pen
//! note states it for the deadline service).
//!
//! NO-POLLING (Tom's ruling, counted law): there is no flush timer, no
//! recurring callback, and no buffer-poll anywhere on this seam. The flush is
//! invoked synchronously by the arbiter at the tail of the same host turn
//! whose slices produced the output; an empty buffer costs one uncontended
//! lock and no host call.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use beamr::io_sink::{IoSink, IoStream};
use js_sys::{Function, Reflect};
use wasm_bindgen::{JsCast, JsValue};

/// Scheduler-side sink: captures tagged writes for the end-of-turn delivery.
///
/// `Mutex` only to satisfy the `Send + Sync` [`IoSink`] bound; never contended
/// (one thread) — the same pattern as the scheduler's `DeferredEffects`.
pub(crate) struct CooperativeSinkBuffer {
    pending: Mutex<Vec<(IoStream, Vec<u8>)>>,
}

impl CooperativeSinkBuffer {
    fn drain(&self) -> Vec<(IoStream, Vec<u8>)> {
        let mut guard = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        std::mem::take(&mut *guard)
    }
}

impl IoSink for CooperativeSinkBuffer {
    fn write(&self, bytes: &[u8]) {
        // Untagged writes are stdout-flavoured by the `IoSink` contract.
        self.write_stream(IoStream::Out, bytes);
    }

    fn write_stream(&self, stream: IoStream, bytes: &[u8]) {
        let mut guard = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        guard.push((stream, bytes.to_vec()));
    }
}

/// Host-side bridge owned by the `WasmVm`: holds the buffer the scheduler
/// writes into plus the optional registered JavaScript sink callback.
pub(crate) struct HostIoSinkBridge {
    buffer: Arc<CooperativeSinkBuffer>,
    callback: RefCell<Option<Function>>,
}

impl HostIoSinkBridge {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            buffer: Arc::new(CooperativeSinkBuffer {
                pending: Mutex::new(Vec::new()),
            }),
            callback: RefCell::new(None),
        })
    }

    /// The `Send + Sync` sink half handed to `WasmScheduler::set_io_sink`.
    pub(crate) fn scheduler_sink(&self) -> Arc<dyn IoSink> {
        Arc::clone(&self.buffer) as Arc<dyn IoSink>
    }

    /// Install (or replace) the JavaScript sink callback:
    /// `callback(stream, text)` with `stream` `"out"`/`"err"`.
    pub(crate) fn register(&self, callback: Function) {
        *self.callback.borrow_mut() = Some(callback);
    }

    /// Deliver every captured write, in write order, one callback invocation
    /// per BIF write (bytes decoded as lossy UTF-8).
    ///
    /// Called by the arbiter once per host turn AFTER the drain has settled
    /// (no scheduler borrow is live, so a sink callback may legally re-enter
    /// the VM surface). With no registered callback — or when the registered
    /// callback throws — the write falls back to the console default so
    /// output is never silently lost (OQ2: the console IS the platform sink).
    pub(crate) fn flush(&self) {
        let drained = self.buffer.drain();
        if drained.is_empty() {
            return;
        }
        // Clone the callback out of the cell before invoking it: a callback
        // that re-enters `register_io_sink` must not hit a live borrow.
        let callback = self.callback.borrow().clone();
        for (stream, bytes) in drained {
            let text = String::from_utf8_lossy(&bytes);
            let delivered = callback.as_ref().is_some_and(|function| {
                function
                    .call2(
                        &JsValue::NULL,
                        &JsValue::from_str(stream_tag(stream)),
                        &JsValue::from_str(&text),
                    )
                    .is_ok()
            });
            if !delivered {
                console_write(stream, &text);
            }
        }
    }
}

fn stream_tag(stream: IoStream) -> &'static str {
    match stream {
        IoStream::Out => "out",
        IoStream::Err => "err",
    }
}

/// Console default (OQ2 ruled): `console.log` for out, `console.error` for
/// err, via js-sys `Reflect` against the global — no new dependency. A host
/// without a console (or with a non-function member) drops the write
/// silently; that host has opted out of the platform sink.
fn console_write(stream: IoStream, text: &str) {
    let global = js_sys::global();
    let Ok(console) = Reflect::get(&global, &JsValue::from_str("console")) else {
        return;
    };
    let method_name = match stream {
        IoStream::Out => "log",
        IoStream::Err => "error",
    };
    let Ok(method) = Reflect::get(&console, &JsValue::from_str(method_name)) else {
        return;
    };
    let Ok(method) = method.dyn_into::<Function>() else {
        return;
    };
    let _ = method.call1(&console, &JsValue::from_str(text));
}
