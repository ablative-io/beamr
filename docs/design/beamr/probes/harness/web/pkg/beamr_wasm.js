/**
 * A single-node Beamr VM driven cooperatively by JavaScript.
 */
export class WasmVm {
    static __wrap(ptr) {
        const obj = Object.create(WasmVm.prototype);
        obj.__wbg_ptr = ptr;
        WasmVmFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmVmFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmvm_free(ptr, 0);
    }
    /**
     * Await target exit/error, or settled idle when no receive one-shot remains armed.
     * @param {bigint} pid
     * @returns {Promise<any>}
     */
    await_exit(pid) {
        const ret = wasm.wasmvm_await_exit(this.__wbg_ptr, pid);
        return ret;
    }
    /**
     * Send `request` to an actor by pid and return a `Promise` that resolves with
     * the actor's reply value (or rejects on timeout / a marshalling failure).
     *
     * The request value is marshalled to a term, sent through the cooperative
     * `call_async` path (ref-correlated, so concurrent calls never cross
     * replies), and the resulting `CallFuture` is wrapped as a JS `Promise`.
     * The transient client spawn requests the VM's edge-triggered arbiter turn.
     * @param {bigint} pid
     * @param {any} request
     * @returns {Promise<any>}
     */
    call(pid, request) {
        const ret = wasm.wasmvm_call(this.__wbg_ptr, pid, request);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return takeFromExternrefTable0(ret[0]);
    }
    /**
     * Send a fire-and-forget message to an actor by pid (non-blocking).
     *
     * The value is marshalled to a term and cast through the cooperative path; it
     * reaches the actor's cast handler on a later arbiter turn. A cast to a dead
     * pid is silently dropped, exactly like a BEAM send.
     * @param {bigint} pid
     * @param {any} message
     */
    cast(pid, message) {
        const ret = wasm.wasmvm_cast(this.__wbg_ptr, pid, message);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Down-ingress for the browser connection-event hub: the host feeds
     * `{node, reason}` with `reason` drawn from the ruled mapping onto the
     * seven native `ConnectionDownReason` variants (see the
     * `connection_events` module contract).
     *
     * # Errors
     *
     * An unmapped reason or a Down with no open session is a loud typed
     * `ConnectionEventProtocolError`.
     * @param {string} node
     * @param {string} reason
     */
    connection_down(node, reason) {
        const ptr0 = passStringToWasm0(node, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(reason, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.wasmvm_connection_down(this.__wbg_ptr, ptr0, len0, ptr1, len1);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Replacement ingress: the open session for `node` was displaced by a
     * new peer incarnation. Expands atomically into `Down(g, reason)` then
     * `Up(g+1, new_peer_creation)` — the native "peer bounced" sequence;
     * `peer_creation` (not generation) answers restart-vs-blip.
     *
     * # Errors
     *
     * An unmapped reason or a replacement with no open session is a loud
     * typed `ConnectionEventProtocolError`.
     * @param {string} node
     * @param {number} new_peer_creation
     * @param {string} reason
     */
    connection_replaced(node, new_peer_creation, reason) {
        const ptr0 = passStringToWasm0(node, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(reason, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.wasmvm_connection_replaced(this.__wbg_ptr, ptr0, len0, new_peer_creation, ptr1, len1);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Up-ingress for the browser connection-event hub (WPORT-4 R2): the host
     * feeds `{node, peer_creation}` — nothing else. The hub mints the session
     * generation locally (per-peer monotonic from 1; never host-supplied).
     *
     * Takes `&self` so a subscriber callback may legally re-enter this
     * surface through the wasm-bindgen borrow guard (shared borrows nest).
     *
     * # Errors
     *
     * A bare double-Up without an intervening Down is a loud typed
     * `ConnectionEventProtocolError`.
     * @param {string} node
     * @param {number} peer_creation
     */
    connection_up(node, peer_creation) {
        const ptr0 = passStringToWasm0(node, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmvm_connection_up(this.__wbg_ptr, ptr0, len0, peer_creation);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Fetch, order, and load a batch of `.beam` artifacts named by a runtime
     * fetch manifest (WPORT-6; schema v1 in
     * `docs/design/beamr/FETCH-MANIFEST.md`).
     *
     * `fetch` is the injected fetch capability: a function taking one URL
     * string and returning a thenable resolving to an `ArrayBuffer` or
     * `Uint8Array`. It is called once for the manifest URL and once per
     * artifact URL (resolved relative to the manifest URL). No global fetch
     * is probed; explicit injection is the whole contract.
     *
     * Resolves with a JSON-string batch report
     * `{"ok":true,"loaded":[{"module","unresolved","deferred","denied"},...],
     * "cycles":[[...],...],"missing_dependencies":[...]}`; rejects fail-fast
     * with an `ArtifactLoadError` (`"{kind}: {detail}"`) whose `data`
     * property is the JSON string `{"artifact","url","stage","loaded"}` —
     * honest about the no-unload reality: modules loaded before the failure
     * stay loaded.
     * @param {string} manifest_url
     * @param {Function} fetch
     * @returns {Promise<any>}
     */
    load_artifacts(manifest_url, fetch) {
        const ptr0 = passStringToWasm0(manifest_url, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmvm_load_artifacts(this.__wbg_ptr, ptr0, len0, fetch);
        return ret;
    }
    /**
     * Load a caller-provided `.beam` module byte buffer.
     * @param {Uint8Array} bytes
     * @returns {any}
     */
    load_module(bytes) {
        const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmvm_load_module(this.__wbg_ptr, ptr0, len0);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return takeFromExternrefTable0(ret[0]);
    }
    /**
     * Create a VM with common atoms and wasm-safe BIF registrations.
     */
    constructor() {
        const ret = wasm.wasmvm_new();
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        this.__wbg_ptr = ret[0];
        WasmVmFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Register a JavaScript Promise-returning native under module/function/arity.
     * @param {string} module
     * @param {string} _function
     * @param {number} arity
     * @param {Function} callback
     */
    register_async_nif(module, _function, arity, callback) {
        const ptr0 = passStringToWasm0(module, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(_function, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.wasmvm_register_async_nif(this.__wbg_ptr, ptr0, len0, ptr1, len1, arity, callback);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Register the one-shot push failure callback (WPORT-7 R1), invoked with
     * the typed `SchedulerFailureError` at the FIRST `fail()` after
     * registration reaches the latch — event-driven, never polled (the VM
     * arms no timer for this). Covers the two latch-only legs (`deadline`,
     * `promise`) invisible to hosts not parked in `await_exit`. Registering
     * after the latch has already set never fires — consult
     * [`WasmVm::terminal_error`] for the already-latched value. Pre-failure
     * re-registration replaces the callback (last-wins).
     * @param {Function} callback
     */
    register_failure_callback(callback) {
        wasm.wasmvm_register_failure_callback(this.__wbg_ptr, callback);
    }
    /**
     * Register a JavaScript IO sink callback `(stream, text)` receiving the
     * VM's `io`-family output with `stream` `"out"` or `"err"` (WPORT-5 R2
     * item 4). Replaces the zero-configuration console default. The sink is
     * PUSH-ONLY: output is delivered synchronously at the tail of the host
     * turn that produced it — no flush timer, no recurring callback.
     * @param {Function} callback
     */
    register_io_sink(callback) {
        wasm.wasmvm_register_io_sink(this.__wbg_ptr, callback);
    }
    /**
     * Register a JavaScript function for `wasm_ffi:js_callback/{N}` calls.
     * @param {string} name
     * @param {Function} callback
     */
    register_js_callback(name, callback) {
        const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        wasm.wasmvm_register_js_callback(this.__wbg_ptr, ptr0, len0, callback);
    }
    /**
     * Register `wasm_ffi:js_callback/Arity` for a previously registered JS callback.
     *
     * The BEAM call shape is `wasm_ffi:js_callback(Name, Arg1, ..., ArgN)`, so
     * the registered native arity must include the leading callback name.
     * @param {number} arity
     */
    register_js_callback_nif(arity) {
        const ret = wasm.wasmvm_register_js_callback_nif(this.__wbg_ptr, arity);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Run one bounded cooperative drain and return its complete JSON result.
     * @returns {any}
     */
    run_step() {
        const ret = wasm.wasmvm_run_step(this.__wbg_ptr);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return takeFromExternrefTable0(ret[0]);
    }
    /**
     * Send a JavaScript value to a BEAM process mailbox by local PID.
     * @param {bigint} pid
     * @param {any} value
     */
    send_message(pid, value) {
        const ret = wasm.wasmvm_send_message(this.__wbg_ptr, pid, value);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Spawn an exported function. Arguments are encoded as a JSON array string.
     * @param {string} module
     * @param {string} _function
     * @param {string} args_json
     * @returns {bigint}
     */
    spawn(module, _function, args_json) {
        const ptr0 = passStringToWasm0(module, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(_function, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ptr2 = passStringToWasm0(args_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len2 = WASM_VECTOR_LEN;
        const ret = wasm.wasmvm_spawn(this.__wbg_ptr, ptr0, len0, ptr1, len1, ptr2, len2);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return BigInt.asUintN(64, ret[0]);
    }
    /**
     * Spawn a cooperative actor whose request/reply logic is a JavaScript
     * function, returning its `u64` pid.
     *
     * `handler` is `reply = handler(request)`: the VM marshals each inbound
     * request term to a `JsValue` (the term codec), calls `handler`, and marshals
     * the returned value back to a reply term. The actor is a first-class beamr
     * process (pid, mailbox, supervision) driven by the cooperative `call_async`
     * surface, so [`WasmVm::call`] returns a real `Promise` over its reply. The
     * handler must return synchronously (it computes a value, not a `Promise`);
     * host *async* work belongs on the async-NIF seam ([`WasmVm::register_async_nif`]).
     *
     * The handler runs on the host thread during a pumped turn, so it stays alive
     * for the actor's lifetime in a per-VM registry rather than crossing the
     * `Send` actor boundary (a JS `Function` is `!Send`); the actor carries only a
     * small registry id.
     * @param {Function} handler
     * @returns {bigint}
     */
    spawn_actor(handler) {
        const ret = wasm.wasmvm_spawn_actor(this.__wbg_ptr, handler);
        return BigInt.asUintN(64, ret);
    }
    /**
     * Subscribe to connection lifecycle events (Up + Down; no catch-up),
     * mirroring the native `ConnectionManager` method name. Returns the
     * numeric `SubscriberId`. Subscribers are TOLD — callbacks run
     * synchronously with host-fed ingress; nothing polls (NO-POLLING).
     * @param {Function} callback
     * @returns {number}
     */
    subscribe_connection_events(callback) {
        const ret = wasm.wasmvm_subscribe_connection_events(this.__wbg_ptr, callback);
        return ret >>> 0;
    }
    /**
     * Subscribe with synthetic catch-up: the blessed late-subscriber path
     * (INV-NO-REPLAY). Before this returns, `callback` alone is invoked
     * SYNCHRONOUSLY with one synthetic `Up` per live peer — invisible to
     * other subscribers — then registered. Called reentrantly from inside a
     * subscriber callback, it registers WITHOUT catch-up (native rule,
     * verbatim).
     * @param {Function} callback
     * @returns {number}
     */
    subscribe_connection_events_with_snapshot(callback) {
        const ret = wasm.wasmvm_subscribe_connection_events_with_snapshot(this.__wbg_ptr, callback);
        return ret >>> 0;
    }
    /**
     * Consume and return the structured refusal reason for an errored pid
     * (WPORT-5 R2 item 7), or `null` when no interpreter error is retained.
     *
     * The shape distinguishes facility-absent `{"error":"badarg"}`,
     * `{"error":"undef","module":..,"function":..,"arity":..}`, and
     * `{"error":"unsupported_opcode","name":..}` (the wasm dirty-call
     * mapping); every other `ExecError` variant carries its snake_case name
     * plus a `detail` string. Errored completions carry the same shape in
     * their `reason` field without consuming the record.
     * @param {bigint} pid
     * @returns {any}
     */
    take_exit_error(pid) {
        const ret = wasm.wasmvm_take_exit_error(this.__wbg_ptr, pid);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return takeFromExternrefTable0(ret[0]);
    }
    /**
     * Consume and return the captured exit value for `pid`, if that process has exited.
     *
     * Hosts that serve many independent requests should prefer this over repeatedly
     * scanning `run_step().results`, because it releases the scheduler's retained
     * copy of the process result once the host has converted it.
     * @param {bigint} pid
     * @returns {any}
     */
    take_exit_result(pid) {
        const ret = wasm.wasmvm_take_exit_result(this.__wbg_ptr, pid);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return takeFromExternrefTable0(ret[0]);
    }
    /**
     * Non-consuming read of the terminal scheduler failure (WPORT-7 R1):
     * `null` before any failure; after the latch sets, the
     * `SchedulerFailureError`'s `data` JSON string
     * (`{"leg":…,"phase":…,"terminal":true}`), repeatably. The latch is
     * permanent — there is no clear/reset API (no-knob law).
     * @returns {any}
     */
    terminal_error() {
        const ret = wasm.wasmvm_terminal_error(this.__wbg_ptr);
        return ret;
    }
    /**
     * Called by tests or custom hosts to drive an already-fired timer manually.
     *
     * This is external host driving, not an admitted unified-deadline fire: the
     * record leaves the receive map and the unified arm is reconciled (moving
     * or clearing if this was the earliest deadline), but no admitted
     * execution is counted.
     * @param {bigint} pid
     * @param {bigint} timer_id
     * @returns {boolean}
     */
    timer_fired(pid, timer_id) {
        const ret = wasm.wasmvm_timer_fired(this.__wbg_ptr, pid, timer_id);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return ret[0] !== 0;
    }
    /**
     * Remove a connection-event subscription by its numeric id; `false` when
     * the id is unknown or already removed. Delivery stops from the next
     * event.
     * @param {number} id
     * @returns {boolean}
     */
    unsubscribe_connection_events(id) {
        const ret = wasm.wasmvm_unsubscribe_connection_events(this.__wbg_ptr, id);
        return ret !== 0;
    }
}
if (Symbol.dispose) WasmVm.prototype[Symbol.dispose] = WasmVm.prototype.free;

/**
 * Construct a new Beamr VM handle for JavaScript hosts.
 * @returns {WasmVm}
 */
export function create_vm() {
    const ret = wasm.create_vm();
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return WasmVm.__wrap(ret[0]);
}

/**
 * Register (or replace — last-wins) the process-global plain-JS panic
 * callback, invoked by the reporting-only panic hook with one string
 * argument (message + location) BEFORE the trap. `console.error` fires
 * regardless of registration. The slot is process-global, shared by every
 * `WasmVm` in the realm — see the module doc's recovery contract: post-panic
 * the panicking instance is bricked and must be replaced, so the callback is
 * a report channel, never a recovery channel.
 * @param {Function} callback
 */
export function register_panic_callback(callback) {
    wasm.register_panic_callback(callback);
}
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg___wbindgen_boolean_get_b131b2f36d6b2f55: function(arg0) {
            const v = arg0;
            const ret = typeof(v) === 'boolean' ? v : undefined;
            return isLikeNone(ret) ? 0xFFFFFF : ret ? 1 : 0;
        },
        __wbg___wbindgen_debug_string_56c147eb1a51f0c4: function(arg0, arg1) {
            const ret = debugString(arg1);
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg___wbindgen_is_function_147961669f068cd4: function(arg0) {
            const ret = typeof(arg0) === 'function';
            return ret;
        },
        __wbg___wbindgen_is_null_ced4761460071341: function(arg0) {
            const ret = arg0 === null;
            return ret;
        },
        __wbg___wbindgen_is_object_3a2c414391dbf751: function(arg0) {
            const val = arg0;
            const ret = typeof(val) === 'object' && val !== null;
            return ret;
        },
        __wbg___wbindgen_is_undefined_4410e3c20a99fa97: function(arg0) {
            const ret = arg0 === undefined;
            return ret;
        },
        __wbg___wbindgen_number_get_588ed6b97f0d7e14: function(arg0, arg1) {
            const obj = arg1;
            const ret = typeof(obj) === 'number' ? obj : undefined;
            getDataViewMemory0().setFloat64(arg0 + 8 * 1, isLikeNone(ret) ? 0 : ret, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
        },
        __wbg___wbindgen_string_get_fa2687d531ed17a5: function(arg0, arg1) {
            const obj = arg1;
            const ret = typeof(obj) === 'string' ? obj : undefined;
            var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            var len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg___wbindgen_throw_bbadd78c1bac3a77: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg__wbg_cb_unref_c2301a3c9b78104b: function(arg0) {
            arg0._wbg_cb_unref();
        },
        __wbg_apply_ea2acc70b42592e9: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = Reflect.apply(arg0, arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_call_c00e41735f66c175: function() { return handleError(function (arg0, arg1, arg2, arg3) {
            const ret = arg0.call(arg1, arg2, arg3);
            return ret;
        }, arguments); },
        __wbg_call_ec09a4cf93377d3a: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.call(arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_from_8a57180716c586ee: function(arg0) {
            const ret = Array.from(arg0);
            return ret;
        },
        __wbg_getRandomValues_76dfc69825c9c552: function() { return handleError(function (arg0, arg1) {
            globalThis.crypto.getRandomValues(getArrayU8FromWasm0(arg0, arg1));
        }, arguments); },
        __wbg_get_4b90d6d8c5deb5d5: function(arg0, arg1) {
            const ret = arg0[arg1 >>> 0];
            return ret;
        },
        __wbg_get_52a8a619f7b88df6: function() { return handleError(function (arg0, arg1) {
            const ret = Reflect.get(arg0, arg1);
            return ret;
        }, arguments); },
        __wbg_instanceof_ArrayBuffer_a581da923203f29f: function(arg0) {
            let result;
            try {
                result = arg0 instanceof ArrayBuffer;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_Error_cb5ebd65d798655e: function(arg0) {
            let result;
            try {
                result = arg0 instanceof Error;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_Object_34d30ae022f04c89: function(arg0) {
            let result;
            try {
                result = arg0 instanceof Object;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_Promise_aa24ea31000d4ee6: function(arg0) {
            let result;
            try {
                result = arg0 instanceof Promise;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_Uint8Array_b6fe1ac89eba107e: function(arg0) {
            let result;
            try {
                result = arg0 instanceof Uint8Array;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_isArray_139f48e3c057ede8: function(arg0) {
            const ret = Array.isArray(arg0);
            return ret;
        },
        __wbg_keys_bd51ff67a9b04698: function(arg0) {
            const ret = Object.keys(arg0);
            return ret;
        },
        __wbg_length_68a9d5278d084f4f: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_length_fb04d16d7bdf6d4c: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_message_5c6ab4dd6c4b34e8: function(arg0) {
            const ret = arg0.message;
            return ret;
        },
        __wbg_new_0b303268aa395a38: function() {
            const ret = new Array();
            return ret;
        },
        __wbg_new_20b778a4c5c691c3: function() {
            const ret = new Object();
            return ret;
        },
        __wbg_new_5fae30e6b23db8df: function(arg0, arg1) {
            const ret = new Error(getStringFromWasm0(arg0, arg1));
            return ret;
        },
        __wbg_new_b06772b280cc6e52: function(arg0) {
            const ret = new Uint8Array(arg0);
            return ret;
        },
        __wbg_new_b3334f9cd9f51d36: function(arg0, arg1) {
            try {
                var state0 = {a: arg0, b: arg1};
                var cb0 = (arg0, arg1) => {
                    const a = state0.a;
                    state0.a = 0;
                    try {
                        return wasm_bindgen__convert__closures_____invoke__h11b34e0e85bb73da(a, state0.b, arg0, arg1);
                    } finally {
                        state0.a = a;
                    }
                };
                const ret = new Promise(cb0);
                return ret;
            } finally {
                state0.a = 0;
            }
        },
        __wbg_new_typed_90c3f6c29ba36d19: function(arg0, arg1) {
            try {
                var state0 = {a: arg0, b: arg1};
                var cb0 = (arg0, arg1) => {
                    const a = state0.a;
                    state0.a = 0;
                    try {
                        return wasm_bindgen__convert__closures_____invoke__h11b34e0e85bb73da(a, state0.b, arg0, arg1);
                    } finally {
                        state0.a = a;
                    }
                };
                const ret = new Promise(cb0);
                return ret;
            } finally {
                state0.a = 0;
            }
        },
        __wbg_new_with_length_4b57a7a5dc67221c: function(arg0) {
            const ret = new Uint8Array(arg0 >>> 0);
            return ret;
        },
        __wbg_now_e7c6795a7f81e10f: function(arg0) {
            const ret = arg0.now();
            return ret;
        },
        __wbg_performance_3fcf6e32a7e1ed0a: function(arg0) {
            const ret = arg0.performance;
            return ret;
        },
        __wbg_prototypesetcall_956c7493c68e29b4: function(arg0, arg1, arg2) {
            Uint8Array.prototype.set.call(getArrayU8FromWasm0(arg0, arg1), arg2);
        },
        __wbg_push_ceb8ef046afb2041: function(arg0, arg1) {
            const ret = arg0.push(arg1);
            return ret;
        },
        __wbg_queueMicrotask_4698f900840e3286: function(arg0) {
            queueMicrotask(arg0);
        },
        __wbg_queueMicrotask_477a5533c7100338: function(arg0) {
            const ret = arg0.queueMicrotask;
            return ret;
        },
        __wbg_reject_7b8cb1939730b2a5: function(arg0) {
            const ret = Promise.reject(arg0);
            return ret;
        },
        __wbg_resolve_0183de2e8c6b1d54: function(arg0) {
            const ret = Promise.resolve(arg0);
            return ret;
        },
        __wbg_set_86698c227e5b9dad: function(arg0, arg1, arg2) {
            arg0.set(getArrayU8FromWasm0(arg1, arg2));
        },
        __wbg_set_a6ba3ac0e634b822: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = Reflect.set(arg0, arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_set_name_7741f9b6eb8fa74c: function(arg0, arg1, arg2) {
            arg0.name = getStringFromWasm0(arg1, arg2);
        },
        __wbg_static_accessor_GLOBAL_60a4124bab7dcc9a: function() {
            const ret = typeof global === 'undefined' ? null : global;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_static_accessor_GLOBAL_THIS_95ca6460658b5d13: function() {
            const ret = typeof globalThis === 'undefined' ? null : globalThis;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_static_accessor_SELF_4c95f759a91e9aae: function() {
            const ret = typeof self === 'undefined' ? null : self;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_static_accessor_WINDOW_44b435597f9e9ee7: function() {
            const ret = typeof window === 'undefined' ? null : window;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_then_254bab9b266a77a5: function(arg0, arg1, arg2) {
            const ret = arg0.then(arg1, arg2);
            return ret;
        },
        __wbg_then_3ea18602c6a5123b: function(arg0, arg1) {
            const ret = arg0.then(arg1);
            return ret;
        },
        __wbg_toString_b09619b263823abf: function(arg0) {
            const ret = arg0.toString();
            return ret;
        },
        __wbindgen_cast_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [Externref], shim_idx: 457, ret: Result(Unit), inner_ret: Some(Result(Unit)) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h98876e68430fdf15);
            return ret;
        },
        __wbindgen_cast_0000000000000002: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [], shim_idx: 92, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h3133c7dbfa19a55a);
            return ret;
        },
        __wbindgen_cast_0000000000000003: function(arg0) {
            // Cast intrinsic for `F64 -> Externref`.
            const ret = arg0;
            return ret;
        },
        __wbindgen_cast_0000000000000004: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./beamr_wasm_bg.js": import0,
    };
}

function wasm_bindgen__convert__closures_____invoke__h3133c7dbfa19a55a(arg0, arg1) {
    wasm.wasm_bindgen__convert__closures_____invoke__h3133c7dbfa19a55a(arg0, arg1);
}

function wasm_bindgen__convert__closures_____invoke__h98876e68430fdf15(arg0, arg1, arg2) {
    const ret = wasm.wasm_bindgen__convert__closures_____invoke__h98876e68430fdf15(arg0, arg1, arg2);
    if (ret[1]) {
        throw takeFromExternrefTable0(ret[0]);
    }
}

function wasm_bindgen__convert__closures_____invoke__h11b34e0e85bb73da(arg0, arg1, arg2, arg3) {
    wasm.wasm_bindgen__convert__closures_____invoke__h11b34e0e85bb73da(arg0, arg1, arg2, arg3);
}

const WasmVmFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmvm_free(ptr, 1));

function addToExternrefTable0(obj) {
    const idx = wasm.__externref_table_alloc();
    wasm.__wbindgen_externrefs.set(idx, obj);
    return idx;
}

const CLOSURE_DTORS = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(state => wasm.__wbindgen_destroy_closure(state.a, state.b));

function debugString(val) {
    // primitive types
    const type = typeof val;
    if (type == 'number' || type == 'boolean' || val == null) {
        return  `${val}`;
    }
    if (type == 'string') {
        return `"${val}"`;
    }
    if (type == 'symbol') {
        const description = val.description;
        if (description == null) {
            return 'Symbol';
        } else {
            return `Symbol(${description})`;
        }
    }
    if (type == 'function') {
        const name = val.name;
        if (typeof name == 'string' && name.length > 0) {
            return `Function(${name})`;
        } else {
            return 'Function';
        }
    }
    // objects
    if (Array.isArray(val)) {
        const length = val.length;
        let debug = '[';
        if (length > 0) {
            debug += debugString(val[0]);
        }
        for(let i = 1; i < length; i++) {
            debug += ', ' + debugString(val[i]);
        }
        debug += ']';
        return debug;
    }
    // Test for built-in
    const builtInMatches = /\[object ([^\]]+)\]/.exec(toString.call(val));
    let className;
    if (builtInMatches && builtInMatches.length > 1) {
        className = builtInMatches[1];
    } else {
        // Failed to match the standard '[object ClassName]'
        return toString.call(val);
    }
    if (className == 'Object') {
        // we're a user defined class or Object
        // JSON.stringify avoids problems with cycles, and is generally much
        // easier than looping through ownProperties of `val`.
        try {
            return 'Object(' + JSON.stringify(val) + ')';
        } catch (_) {
            return 'Object';
        }
    }
    // errors
    if (val instanceof Error) {
        return `${val.name}: ${val.message}\n${val.stack}`;
    }
    // TODO we could test for more things here, like `Set`s and `Map`s.
    return className;
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        const idx = addToExternrefTable0(e);
        wasm.__wbindgen_exn_store(idx);
    }
}

function isLikeNone(x) {
    return x === undefined || x === null;
}

function makeMutClosure(arg0, arg1, f) {
    const state = { a: arg0, b: arg1, cnt: 1 };
    const real = (...args) => {

        // First up with a closure we increment the internal reference
        // count. This ensures that the Rust closure environment won't
        // be deallocated while we're invoking it.
        state.cnt++;
        const a = state.a;
        state.a = 0;
        try {
            return f(a, state.b, ...args);
        } finally {
            state.a = a;
            real._wbg_cb_unref();
        }
    };
    real._wbg_cb_unref = () => {
        if (--state.cnt === 0) {
            wasm.__wbindgen_destroy_closure(state.a, state.b);
            state.a = 0;
            CLOSURE_DTORS.unregister(state);
        }
    };
    CLOSURE_DTORS.register(real, state, state);
    return real;
}

function passArray8ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 1, 1) >>> 0;
    getUint8ArrayMemory0().set(arg, ptr / 1);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('beamr_wasm_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
