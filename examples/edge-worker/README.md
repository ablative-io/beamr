# Beamr Cloudflare Worker example

This example shows how to deploy a precompiled Beamr WASM bundle to an edge worker platform. It keeps the worker isolate warm by loading the bundle at module scope, then handles every HTTP request by spawning a fresh BEAM process and awaiting its arbiter-driven completion.

## Build the bundle

From the repository root, point the WASM bundle builder at a directory of compiled `.beam` files and build the package:

```sh
BEAMR_WASM_BUNDLE_DIR=crates/beamr/tests/fixtures/gleam_gate \
  wasm-pack build crates/beamr-wasm --target web --out-dir pkg

node crates/beamr-wasm/target/wasm32-unknown-unknown/release/build/beamr-wasm-*/out/beamr-wasm-bundle/package-bundle.mjs \
  crates/beamr-wasm/pkg
```

The build script emits deterministic bundle assets under Cargo `OUT_DIR/beamr-wasm-bundle/`:

- `bootstrap.js` imports the wasm-bindgen package, constructs a VM with `create_vm()`, loads each bundled module with `load_module(bytes)`, and exports `createPreloadedVm`, `spawnPreloaded`, and `awaitExit`.
- `modules.bin` uses the same `BEAMR_EMBED\0` archive format as the native runtime for the selected `.beam` files.
- `manifest.json` records `beamr.wasm`/`beamr_wasm_bg.wasm`, the bootstrap, archive, and sorted module list.
- `package-bundle.mjs` can turn `wasm-pack` output plus `bootstrap.js` into `beamr.bundle.mjs` for single-import deployment.

Copy or generate `crates/beamr-wasm/pkg/beamr.bundle.mjs` before running the worker. The Worker imports that one module; browser hosts may also load it from a single `<script type="module">` tag.

## Worker contract

`src/worker.js` converts a Cloudflare `Request` into a plain JS object:

```js
{
  method: request.method,
  url: request.url,
  headers: { ... },
  body: await request.text()
}
```

That object is passed through the existing B-146 copy-based JSON/Term conversion path by calling:

```js
vm.spawn(env.BEAMR_EDGE_MODULE, env.BEAMR_EDGE_FUNCTION, JSON.stringify([requestObject]))
```

The worker then calls `await awaitExit(vm, pid)`. External idle-to-runnable edges queue one microtask; an explicit fairness yield continues through `setTimeout(0)` so the host gets a real turn. The BEAM handler should return either a string body or an object shaped like `{ status, headers, body }`; the exited completion carries that converted value in `result`. There is no max-step option or repeated `run_step()` completion protocol.

## Local smoke test

Install the example-local tooling and run the Miniflare smoke test:

```sh
npm install
npm test
```

The smoke test verifies HTTP request/response handling, the explicit WebSocket rejection path, and cleanup of per-request exit results while reusing the preloaded VM. It runs the real `src/worker.js` with an in-test bundle stub so it can run without a prebuilt `.wasm` artifact; use the build steps above for an end-to-end bundle test.

## Boundaries

- Stateless per request: the isolate caches only the preloaded module bundle/VM; request data is copied into a freshly spawned BEAM process and is not persisted after the response.
- Bricked-VM recovery (WPORT-7): the module-level `preloadedVmPromise` caches one VM for the isolate's lifetime and is never invalidated. After a Rust panic the instance is latched (borrowed RefCells, stuck Draining) — every scheduler-touching call re-traps — and after a terminal scheduler failure every `await_exit` rejects with the latched `SchedulerFailureError` (`vm.terminal_error()` reads it non-consumingly; `register_failure_callback`/`register_panic_callback` push-report both). Either way the cached VM serves nothing useful afterwards: recovery is a fresh `WasmVm` on a fresh isolate — let the failing requests surface and the platform recycle the worker rather than retrying into the bricked instance.
- HTTP request/response only: WebSocket upgrades return `426` and Durable Objects or persistent state are intentionally not used.
- WASM-safe execution only: handlers must avoid dirty native calls, blocking I/O, OS threads, and distribution.
