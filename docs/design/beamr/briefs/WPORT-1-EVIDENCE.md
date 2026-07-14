# WPORT-1 executed evidence — the pinned wasm run at d9de35e

> Committed per the WPORT-1 tear (blocker 2 fold, 2026-07-14): the brief's executed-evidence
> citations must point at a tracked, immutable artifact. This is a fresh execution of the
> pinned command by the domain owner's hands (not a copy of the untracked research pack),
> run in a detached worktree at d9de35e. Immutable record — do not edit; a re-run belongs
> in a new file.

```text
=== command ===
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test --manifest-path crates/beamr-wasm/Cargo.toml --target wasm32-unknown-unknown --locked -- --nocapture
=== commit ===
d9de35e4e753c9a2c5dbbe7b31dcde5bf0fe2349
=== toolchain ===
rustc 1.94.1 (e408947bf 2026-03-25)
binary: rustc
cargo 1.94.1 (29ea6fb6a 2026-03-24)
wasm-bindgen-test-runner 0.2.125
v26.5.0
=== date ===
2026-07-14T10:51:28Z
=== output ===
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.19s
     Running unittests src/lib.rs (target/wasm32-unknown-unknown/debug/deps/beamr_wasm-f8f724c1d3d5ae85.wasm)
running 5 tests
Invoking test: convert::tests::documents_boolean_atom_round_trip_as_atom_names
test convert::tests::documents_boolean_atom_round_trip_as_atom_names ... ok
Invoking test: convert::tests::converts_terms_to_js_values
test convert::tests::converts_terms_to_js_values ... ok
Invoking test: convert::tests::converts_complex_nested_js_object_to_term
test convert::tests::converts_complex_nested_js_object_to_term ... ok
Invoking test: tests::pump_once_drives_an_actor_call_to_completion
panicked at crates/beamr-wasm/src/lib.rs:973:36:
VM constructs: JsValue("native function already registered for Atom(76):Atom(125)/3")

Stack:

Error
    at /private/var/folders/8l/4k65z22577g2h8_lt87qhx600000gn/T/.tmpTPnk2f/wasm-bindgen-test.js:779:25
    at logError (/private/var/folders/8l/4k65z22577g2h8_lt87qhx600000gn/T/.tmpTPnk2f/wasm-bindgen-test.js:1164:18)
    at __wbg_new_3887def06d2d5d1a (/private/var/folders/8l/4k65z22577g2h8_lt87qhx600000gn/T/.tmpTPnk2f/wasm-bindgen-test.js:778:57)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::panic_handling::Error::new::__wbg_new_3887def06d2d5d1a::h208ee6c2c8e00147 externref shim (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[15612]:0x34aff7)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::panic_handling::Error::new::h4ff5b6694587e62b (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[9421]:0x30703d)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::panic_handling::h2b2276656d671416 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[4075]:0x2634af)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::{{closure}}::{{closure}}::h52ced7fd745d40e5 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[10459]:0x317d77)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.std::panicking::panic_with_hook::h77afe0ddfda2cb89 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[4004]:0x25fc00)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.std::panicking::panic_handler::{{closure}}::hf36efc37fdd11196 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[6549]:0x2c3162)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.std::sys::backtrace::__rust_end_short_backtrace::h46ab1174c51ef229 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[15947]:0x34c012)


test tests::pump_once_drives_an_actor_call_to_completion ... FAIL
Invoking test: tests::await_vm_call_resolves_with_js_handler_reply
panicked at crates/beamr-wasm/src/lib.rs:918:36:
VM constructs: JsValue("native function already registered for Atom(76):Atom(125)/3")

Stack:

Error
    at /private/var/folders/8l/4k65z22577g2h8_lt87qhx600000gn/T/.tmpTPnk2f/wasm-bindgen-test.js:779:25
    at logError (/private/var/folders/8l/4k65z22577g2h8_lt87qhx600000gn/T/.tmpTPnk2f/wasm-bindgen-test.js:1164:18)
    at __wbg_new_3887def06d2d5d1a (/private/var/folders/8l/4k65z22577g2h8_lt87qhx600000gn/T/.tmpTPnk2f/wasm-bindgen-test.js:778:57)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::panic_handling::Error::new::__wbg_new_3887def06d2d5d1a::h208ee6c2c8e00147 externref shim (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[15612]:0x34aff7)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::panic_handling::Error::new::h4ff5b6694587e62b (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[9421]:0x30703d)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::panic_handling::h2b2276656d671416 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[4075]:0x2634af)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::{{closure}}::{{closure}}::h52ced7fd745d40e5 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[10459]:0x317d77)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.std::panicking::panic_with_hook::h77afe0ddfda2cb89 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[4004]:0x25fc00)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.std::panicking::panic_handler::{{closure}}::hf36efc37fdd11196 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[6549]:0x2c3162)
    at beamr_wasm-f8f724c1d3d5ae85.wasm.std::sys::backtrace::__rust_end_short_backtrace::h46ab1174c51ef229 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[15947]:0x34c012)


test tests::await_vm_call_resolves_with_js_handler_reply ... FAIL

failures:

---- tests::pump_once_drives_an_actor_call_to_completion output ----
    error output:
        panicked at crates/beamr-wasm/src/lib.rs:973:36:
        VM constructs: JsValue("native function already registered for Atom(76):Atom(125)/3")
        
        Stack:
        
        Error
            at /private/var/folders/8l/4k65z22577g2h8_lt87qhx600000gn/T/.tmpTPnk2f/wasm-bindgen-test.js:779:25
            at logError (/private/var/folders/8l/4k65z22577g2h8_lt87qhx600000gn/T/.tmpTPnk2f/wasm-bindgen-test.js:1164:18)
            at __wbg_new_3887def06d2d5d1a (/private/var/folders/8l/4k65z22577g2h8_lt87qhx600000gn/T/.tmpTPnk2f/wasm-bindgen-test.js:778:57)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::panic_handling::Error::new::__wbg_new_3887def06d2d5d1a::h208ee6c2c8e00147 externref shim (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[15612]:0x34aff7)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::panic_handling::Error::new::h4ff5b6694587e62b (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[9421]:0x30703d)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::panic_handling::h2b2276656d671416 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[4075]:0x2634af)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::{{closure}}::{{closure}}::h52ced7fd745d40e5 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[10459]:0x317d77)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.std::panicking::panic_with_hook::h77afe0ddfda2cb89 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[4004]:0x25fc00)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.std::panicking::panic_handler::{{closure}}::hf36efc37fdd11196 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[6549]:0x2c3162)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.std::sys::backtrace::__rust_end_short_backtrace::h46ab1174c51ef229 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[15947]:0x34c012)
        
        
    
    JS exception that was thrown:
        RuntimeError: unreachable
            at beamr_wasm-f8f724c1d3d5ae85.wasm.__rustc[16f1505adc47261a]::__rust_abort (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[16025]:0x34c248)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.__rustc[16f1505adc47261a]::__rust_start_panic (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[16010]:0x34c1f2)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.__rustc[16f1505adc47261a]::rust_panic (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[15688]:0x34b44c)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.std::panicking::panic_with_hook::h77afe0ddfda2cb89 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[4004]:0x25fc35)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.std::panicking::panic_handler::{{closure}}::hf36efc37fdd11196 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[6549]:0x2c3162)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.std::sys::backtrace::__rust_end_short_backtrace::h46ab1174c51ef229 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[15947]:0x34c012)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.__rustc[16f1505adc47261a]::rust_begin_unwind (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[12347]:0x32f261)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.core::panicking::panic_fmt::h6651313c3e2c6c2f (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[10435]:0x3177ea)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.core::result::unwrap_failed::h8a0dea2fe721e8ce (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[7917]:0x2e7a4b)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.core::result::Result<T,E>::expect::h1afc7a98230704ef (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[4804]:0x284914)

---- tests::await_vm_call_resolves_with_js_handler_reply output ----
    error output:
        panicked at crates/beamr-wasm/src/lib.rs:918:36:
        VM constructs: JsValue("native function already registered for Atom(76):Atom(125)/3")
        
        Stack:
        
        Error
            at /private/var/folders/8l/4k65z22577g2h8_lt87qhx600000gn/T/.tmpTPnk2f/wasm-bindgen-test.js:779:25
            at logError (/private/var/folders/8l/4k65z22577g2h8_lt87qhx600000gn/T/.tmpTPnk2f/wasm-bindgen-test.js:1164:18)
            at __wbg_new_3887def06d2d5d1a (/private/var/folders/8l/4k65z22577g2h8_lt87qhx600000gn/T/.tmpTPnk2f/wasm-bindgen-test.js:778:57)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::panic_handling::Error::new::__wbg_new_3887def06d2d5d1a::h208ee6c2c8e00147 externref shim (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[15612]:0x34aff7)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::panic_handling::Error::new::h4ff5b6694587e62b (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[9421]:0x30703d)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::panic_handling::h2b2276656d671416 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[4075]:0x2634af)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.wasm_bindgen_test::__rt::Context::new::{{closure}}::{{closure}}::h52ced7fd745d40e5 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[10459]:0x317d77)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.std::panicking::panic_with_hook::h77afe0ddfda2cb89 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[4004]:0x25fc00)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.std::panicking::panic_handler::{{closure}}::hf36efc37fdd11196 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[6549]:0x2c3162)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.std::sys::backtrace::__rust_end_short_backtrace::h46ab1174c51ef229 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[15947]:0x34c012)
        
        
    
    JS exception that was thrown:
        RuntimeError: unreachable
            at beamr_wasm-f8f724c1d3d5ae85.wasm.__rustc[16f1505adc47261a]::__rust_abort (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[16025]:0x34c248)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.__rustc[16f1505adc47261a]::__rust_start_panic (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[16010]:0x34c1f2)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.__rustc[16f1505adc47261a]::rust_panic (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[15688]:0x34b44c)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.std::panicking::panic_with_hook::h77afe0ddfda2cb89 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[4004]:0x25fc35)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.std::panicking::panic_handler::{{closure}}::hf36efc37fdd11196 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[6549]:0x2c3162)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.std::sys::backtrace::__rust_end_short_backtrace::h46ab1174c51ef229 (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[15947]:0x34c012)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.__rustc[16f1505adc47261a]::rust_begin_unwind (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[12347]:0x32f261)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.core::panicking::panic_fmt::h6651313c3e2c6c2f (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[10435]:0x3177ea)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.core::result::unwrap_failed::h8a0dea2fe721e8ce (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[7917]:0x2e7a4b)
            at beamr_wasm-f8f724c1d3d5ae85.wasm.core::result::Result<T,E>::expect::h1afc7a98230704ef (wasm://wasm/beamr_wasm-f8f724c1d3d5ae85.wasm-0806e832:wasm-function[4804]:0x284914)

failures:

    tests::pump_once_drives_an_actor_call_to_completion
    tests::await_vm_call_resolves_with_js_handler_reply

test result: FAILED. 3 passed; 2 failed; 0 ignored; 0 filtered out; finished in 0.02s

Error: Node failed with exit_code 1
error: test failed, to rerun pass `--lib`

Caused by:
  process didn't exit successfully: `wasm-bindgen-test-runner /Users/annabel/Developer/ablative/stack/beamr/.wt-evidence/target/wasm32-unknown-unknown/debug/deps/beamr_wasm-f8f724c1d3d5ae85.wasm --nocapture` (exit status: 1)
=== exit: 1 ===
```
