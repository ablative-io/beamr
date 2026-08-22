# beamr-wasm

WebAssembly bindings for [beamr](https://crates.io/crates/beamr) — run compiled `.beam` bytecode inside a JavaScript host, driven cooperatively by that host's event loop.

Native beamr schedules preemptively across a thread pool. A browser has no threads to give it, so `beamr-wasm` selects beamr's cooperative single-threaded runtime instead and hands scheduling authority to JavaScript: the host advances the VM, and the VM asks for a turn when it has work.

## The feature profile is deliberate

```toml
beamr = { version = "0.19.0", default-features = false, features = ["cooperative", "json"] }
```

beamr's defaults are `std`, `threads`, `net`, `fs`, `jit`, `embedded` and `readiness`; between them they pull tokio, io-uring, mio, cranelift and a zstd C build, none of which compile for `wasm32-unknown-unknown`. Turning defaults off drops all of them, and `cooperative` brings `std` back on its own.

`cooperative` is a *substitute* for `threads`, not a companion to it — every one of the cfg sites guarding the cooperative scheduler surface is `any(threads, cooperative)`, so this feature is what keeps that surface compiling when `threads` is off. It pulls no dependencies of its own; the slimming comes entirely from not enabling `threads`. `json` is the term codec the host bridge converts through. The pair is the whole selection — adding to it is how this crate stops building.

The crate is a `cdylib` plus `rlib`: the `cdylib` is what `wasm-bindgen` consumes, the `rlib` is what the native-side tests link against.

## Usage from JavaScript

`create_vm()` returns a `WasmVm` handle. Load modules into it, spawn a function, and let the host drive:

```js
import init, { create_vm } from "./beamr_wasm.js";

await init();
const vm = create_vm();
vm.load_module(beamBytes);              // a .beam byte buffer
const pid = vm.spawn("my_module", "main", "[]");   // args are a JSON array string
const result = await vm.await_exit(pid);
```

`await_exit` settles on process exit, on error, or on settled idle when no receive one-shot remains armed. `run_step` performs one bounded cooperative drain if you would rather step the VM yourself.

### Host seams

The VM reaches the outside world only through seams the host registers, so a host that registers nothing can run bytecode and nothing else:

| seam | registration |
|---|---|
| JS functions callable as `wasm_ffi:js_callback/N` | `register_js_callback`, `register_js_callback_nif` |
| Promise-returning natives bound to module/function/arity | `register_async_nif` |
| `fetch` and `kv` capabilities | `register_fetch_capability`, `register_kv_capability` |
| stdout/stderr drain (defaults to the browser console) | `register_io_sink` |
| scheduler failure reporting | `register_failure_callback`, `terminal_error` |
| distribution connection state | `connection_up`, `connection_down`, `connection_replaced` |
| connection event subscription | `subscribe_connection_events`, `..._with_snapshot`, `unsubscribe_connection_events` |
| messaging | `send_message`, `call`, `cast`, `spawn_actor` |
| receive-timer completion | `timer_fired` |

An unregistered capability is not a silent no-op: calls into it return a typed BEAM error such as `{error, {capability_missing, fetch}}`.

### Nothing polls

Scheduling is edge-triggered and host-fed. A one-shot timeout that delivers a known deadline is fine; a recurring frame callback or loop whose job is to *check whether something changed* is a design error in this crate, and the generated bootstrap is test-sealed against reintroducing one. `web-time` is used for receive-after deadlines so that cooperative receives and native `Deliver` deadlines share one monotonic clock domain with beamr's timer wheel.

## Embedding modules at build time

`build.rs` will bake `.beam` modules into the artifact. Point it at a directory and every `.beam` file there is zstd-compressed into an archive alongside a generated loader:

```bash
BEAMR_WASM_BUNDLE_DIR=./build/dev/erlang/my_app/ebin \
  cargo build -p beamr-wasm --target wasm32-unknown-unknown
```

This writes into `OUT_DIR/beamr-wasm-bundle/`:

- `modules.bin` — the compressed archive
- `manifest.json` — module names, source filenames, uncompressed sizes
- `bootstrap.js` — an ES module exporting `initBeamr`, `createPreloadedVm`, `spawnPreloaded`, `awaitExit`, `bundledModules` and `WasmVm`, with the modules embedded as base64 and preloaded for you
- `package-bundle.mjs` — packs a `wasm-pack` output directory into one self-contained `beamr.bundle.mjs` with the wasm binary inlined

```bash
node "$OUT_DIR/beamr-wasm-bundle/package-bundle.mjs" ./pkg
```

The sweep filter is extension-only, so the build verifies the BEAM IFF container magic (`FOR1` at bytes 0–3, `BEAM` at bytes 8–11) before embedding anything. A file that is named `.beam` but is not one fails the build here rather than at the consumer's first `load_module`.

Set `BEAMR_WASM_BINDGEN_IMPORT` to change the import specifier the generated bootstrap uses; it defaults to `./beamr_wasm.js`. `BEAMR_EMBED_DIR` is accepted as an alias for `BEAMR_WASM_BUNDLE_DIR`. With no bundle directory set, the archive is generated empty and the crate builds normally.

## Testing

The binding's own suite runs under `wasm-bindgen-test`:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli    # must match the version in Cargo.lock

cargo check -p beamr-wasm --target wasm32-unknown-unknown --locked
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
  cargo test --manifest-path crates/beamr-wasm/Cargo.toml --target wasm32-unknown-unknown --locked
```

Two further tests run natively rather than in the browser, because what they check is the build's own output:

- `tests/generated_bootstrap.rs` reads the bootstrap `build.rs` just generated and asserts it awaits exit rather than running a step loop — the NO-POLLING seal.
- `tests/profile_seal.rs` builds the real browser BIF registry and asserts exact two-way set equality between the registered `(module, function, arity)` set and the sealed table in `docs/design/beamr/BROWSER-BIF-PROFILE.md`. A registered-but-undocumented BIF and a documented-but-unregistered row both fail, so the profile document cannot drift away from the code.

The `fixtures/` directory carries the `.erl` sources and compiled `.beam` files the artifact-loader tests use for import chains, cycles, unresolved imports, and the deliberately malformed module.

## License

Apache-2.0
