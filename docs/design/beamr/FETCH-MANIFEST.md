# beamr fetch manifest — schema v1 (WPORT-6)

The runtime fetch manifest drives `WasmVm::load_artifacts(manifest_url, fetch)`
(`crates/beamr-wasm/src/artifact_loader.rs`): runtime browser loading of
`.beam` artifacts over the existing `load_module(bytes)` primitive, with
dependency ordering, structured failures, and preserved unresolved-import
reporting. The manifest is independently authored (or emitted by future
tooling — out of scope here); it is fetched at runtime through the injected
fetch capability, never read from disk by the VM.

## Format

| Field | Type | Meaning |
|-------|------|---------|
| `format` | string, required | Discriminator; exactly `"beamr-fetch-manifest"`. |
| `version` | number, required | Schema version; exactly `1`. |
| `modules` | array, required | One entry per loadable artifact. |
| `modules[].name` | string, required | The BEAM module name. Used for dependency edges, duplicate detection, and post-batch verification. |
| `modules[].url` | string, required | The artifact URL, absolute or relative (see URL resolution). Each names one `.beam` byte artifact. |
| `modules[].deps` | array of strings, optional | Declared dependency edges (module names that should load first). Absent means no declared edges. Every named dep MUST appear in the manifest (`dependency_missing` otherwise). |
| `modules[].integrity` | string, reserved | RESERVED, UNENFORCED in v1 (OQ-A ruling, 2026-07-17): the field is legal and ignored. Enforcement lands whenever the haematite-digest trust root reaches this surface. |

Duplicate `modules[].name` values are fatal (`manifest_malformed`): dedupe is
per-batch only. Across separate `load_artifacts` calls the documented
clobber-with-generation+1 reload semantic stands (`ModuleRegistry::insert_version`) —
re-fetching a manifest is the hot-reload path, not an error.

## Worked example

```json
{
  "format": "beamr-fetch-manifest",
  "version": 1,
  "modules": [
    { "name": "fetch_chain_c", "url": "fetch_chain_c.beam", "deps": [] },
    { "name": "fetch_chain_b", "url": "fetch_chain_b.beam", "deps": ["fetch_chain_c"] },
    { "name": "fetch_chain_a", "url": "/mods/fetch_chain_a.beam", "deps": ["fetch_chain_b"] }
  ]
}
```

Served at `https://example.test/mods/manifest.json`, this loads
`fetch_chain_c` then `fetch_chain_b` (both resolved to
`https://example.test/mods/…` by directory-relative resolution) then
`fetch_chain_a` (root-relative), each fetched through the injected capability
and loaded with `ModuleOrigin::Fetched` provenance.

## URL resolution (minimal, deliberate)

- A URL containing `://` is absolute and used as-is.
- A URL with a leading `/` resolves against the manifest URL's scheme+authority.
- Anything else resolves against the manifest URL's directory (everything up
  to and including the last `/` of its path).
- Dot segments (`../`) are passed through, not normalised; this is not a full
  RFC 3986 resolver. Hosts needing more control emit absolute URLs.

## Ordering and cycles

Load order is Kahn's algorithm over the declared `deps` edges: dependencies
load before dependants for the acyclic part. A dependency cycle is NOT a load
failure — mutual recursion is legal BEAM reality and deferred imports heal at
call time — but each strongly-connected component (size > 1, or a self-edge)
is reported in the success report's `cycles` array with its members listed in
manifest order, which is also the order they load in (OQ-B ruling: the named
`cycles` entry IS the structured diagnosis).

## Failure vocabulary

Operational failures reject the returned Promise with an `ArtifactLoadError`
(`"{kind}: {detail}"`) whose `data` property holds the JSON string
`{"artifact","url","stage","loaded"}` (stages: `manifest`, `order`, `fetch`,
`load`). The closed kind-slug set is pinned by test:
`manifest_fetch_failed`, `manifest_malformed`, `dependency_missing`,
`artifact_fetch_failed`, `fetch_protocol`, `artifact_invalid_format`,
`artifact_decode_failed`, `artifact_validation_failed`. Batch semantics are
fail-fast, and `loaded` is honest about the no-unload reality: modules loaded
before the failure stay loaded (the code-management facility is deliberately
absent on wasm).

Per-module anomalies on success are report data, never failures: the
`unresolved` array preserves the existing `load_module` entry shape
byte-for-byte, with additive `deferred` / `denied` siblings per module and
batch-level `cycles` / `missing_dependencies` arrays. Post-batch, every
deferred import is re-checked against the live registry (export lookup — the
only honest healed-observable, since `resolved_imports` is immutable inside
`Arc<Module>`); targets still absent are reported in `missing_dependencies`.

## Non-goals

- **No build-time coupling.** The `beamr-wasm-bundle` manifest that
  `crates/beamr-wasm/build.rs` emits is NOT extended and has zero runtime
  consumers; `BEAMR_WASM_BUNDLE_DIR` never enters the runtime path.
- **`modules.bin` is not a fetch target.** The embedded archive's linear-scan
  container would double-represent modules already coming in per-module
  (hazards 19/21); fetch targets are per-module `.beam` artifacts only.
- **No integrity enforcement in v1** (field reserved, above).
- **No global-fetch probing.** The fetch capability is explicitly injected; a
  browser host passes a one-line adapter over its own `fetch()`.
- **No name-agreement enforcement.** The manifest `name` is trusted for
  ordering and URL selection only; reporting keys off the module name actually
  decoded from the artifact. A mismatch surfaces as unhealed deferred imports
  in `missing_dependencies` data.

## Hazards

- **Reload can poison dependants (hazard 7).** Order matters across batches:
  if a stale dependency is loaded, a dependant links against it, and the
  dependency is then upgraded to a version that dropped an export, the
  dependant's already-resolved import keeps the dropped target permanently
  (`Undef` at call time) until the DEPENDANT is reloaded too. Replacing a
  dependency whose export set changed requires re-fetching its dependants —
  documented, deliberately not mechanized.
