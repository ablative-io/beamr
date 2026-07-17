# WPORT-6 PROBE-FETCH — real-browser artifact-loader run

**Status:** `PROBE-FETCH: NOT RUN — UNVERIFIED-ON-PLATFORM`
**Authored by:** WPORT-6 (D12: the probe artifact is a deliverable of that
brief; the run is NOT)
**Gating word:** the run happens post-land, on Tom's or Annabel's word — a
short manual browser sitting against a real HTTP server. Observations are
appended here as evidence attached to this artifact; they are **never** CI
acceptance. Ops note carried from the brief: this sitting may share a browser
session with the still-OPEN WPORT-3 deadline probe
(`WPORT-3-PROBE-THROTTLE.md`); sequencing stays unowned by both briefs.

## Why this probe exists

The WPORT-6 CI walls run under the pinned Node wasm-bindgen runner with an
injected fetch **double** resolving fixture bytes — the only hermetic shape
(Node's undici fetch rejects `file://` URLs and CI serves no HTTP). Node
therefore proves ordering, failure vocabulary, report shape, healing, and real
VM execution — but not a real browser `fetch()`, real HTTP status handling
through the adapter, real `ArrayBuffer` bodies off the network, or real
same-origin URL resolution. Those are this probe's territory.

## The one-line adapter

The loader takes an explicitly injected fetch capability (no global probing).
A browser host passes exactly this adapter:

```js
(url) => fetch(url).then(r => { if (!r.ok) throw new Error(String(r.status)); return r.arrayBuffer(); })
```

## Serving recipe

1. Build the wasm bundle and its generated bootstrap for a browser target
   (the real `beamr-wasm` build output, not the Node test runner).
2. Stage a directory with the five chain/cycle fixtures from
   `crates/beamr-wasm/fixtures/` (`fetch_chain_{a,b,c}.beam`,
   `fetch_cycle_{ping,pong}.beam`) and a `manifest.json` following
   `docs/design/beamr/FETCH-MANIFEST.md` v1, e.g. the chain manifest with
   relative URLs and declared deps.
3. Serve the staged directory plus the bundle over real HTTP from one origin
   (any static server, e.g. `python3 -m http.server`), so the manifest URL is
   a real `http://…/manifest.json`.
4. In the page: construct a `WasmVm`, call
   `vm.load_artifacts("http://…/manifest.json", adapter)` with the one-line
   adapter above, then `vm.spawn("fetch_chain_a", "run", "[]")` and
   `await vm.await_exit(pid)`.

## Expected observations (record each, verbatim)

1. The resolved batch report (a JSON string): `ok: true`; `loaded` order
   `fetch_chain_c`, `fetch_chain_b`, `fetch_chain_a`; empty
   `unresolved`/`deferred`/`denied` per module; empty `cycles` and
   `missing_dependencies`.
2. Network panel: exactly four requests (manifest + three artifacts), each
   artifact fetched once, dependencies before dependants.
3. The spawned entry completes settled with `result` `42` — real execution
   through runtime-fetched code, zero manual pumps, zero recurring callbacks.
4. A deliberately wrong artifact URL (edit the manifest) rejects with an
   `ArtifactLoadError` whose message starts `artifact_fetch_failed: ` (the
   adapter throws on `!r.ok`) and whose `data` JSON names the artifact, URL,
   stage `fetch`, and the already-loaded list.
5. `module_info`-visible provenance: fetched modules report source `fetched`
   (`ModuleOrigin::Fetched`).

Late observations append below this line; the probe stays authored-not-run
until then.
