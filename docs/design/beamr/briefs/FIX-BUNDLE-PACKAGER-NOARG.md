# FIX — single-file bundle packager: no-argument `init()` path

**Lane:** `fix/bundle-packager-noarg-init` off main `1f957f0`.
**Routing (Waffles the Terrible, 2026-07-23):** the packager micro-lane is
the WPORT-8 seat's, opened after the sitting runsheet reached Tom; Artemis
Peach tears as domain owner. Red-first; fix stays inside `PACKAGE_SCRIPT`
(`crates/beamr-wasm/build.rs`) — the packager owns its own scope problem;
NO change to the `init` API.

## Provenance (the discovery story, folded per the routing rider)

The WPORT-8 sitting kit's first real-browser smoke (2026-07-24) was the
single-file bundle's **first honest consumer** — the first driver ever to
call the packaged bundle's `init()` path with no argument. It failed
immediately, and inspection at main's bytes confirmed the defect is
latent-since-authoring: every earlier driver passed wasm bytes explicitly
(the kit's own Node smoke did; the Miniflare suite uses a stub bundle; the
WPORT-6 sitting served the wasm-pack pkg + generated `bootstrap.js`
directly). Confirmed independently at the bytes by Anubis Le Snak and
Waffles the Terrible before routing. The sitting itself was unblocked by
serving the pkg + bootstrap shape (recorded in its evidence
`serving_shape` field); the kit README and the arc's WPORT-8 status block
carry the board line.

## The defect, at `1f957f0` bytes

`PACKAGE_SCRIPT` (`crates/beamr-wasm/build.rs`, the `main()` body around
`:328`) emits a bundle whose `importWasmBindgen()`:

1. rewrites the wasm-bindgen glue, replacing
   `new URL('beamr_wasm_bg.wasm', import.meta.url)` with
   `decodeEmbeddedBase64(WASM_BASE64)`, then
2. imports the rewritten glue via a **blob URL**.

Both substituted names are defined only in the OUTER bundle module scope;
the blob-URL import evaluates the glue as a SEPARATE module that cannot
see them. Two host-visible failure signatures, both at the first no-arg
`init()`:

- **Browser:** `ReferenceError: decodeEmbeddedBase64 is not defined`
  (the blob module evaluates, then the no-arg init path evaluates the
  substituted expression).
- **Node:** `ERR_UNSUPPORTED_ESM_URL_SCHEME` — Node's ESM loader does not
  import `blob:` URLs at all, so the same call dies one step earlier.

So the single-file bundle — the artifact the edge-worker README's build
steps produce for single-import deployment — has never worked through its
own embedded-wasm path in ANY host.

## Requirement

R1 — the packaged bundle's no-argument init path works in BOTH a real
browser context and a Node host: `createPreloadedVm()` with no argument
resolves to a constructed VM (embedded wasm, embedded modules loaded),
using one host-portable import mechanism. The `init`/bootstrap API is
unchanged; the fix is confined to `PACKAGE_SCRIPT`'s emitted
`importWasmBindgen()` (make the rewritten glue source self-contained —
helper + embedded base64 constant prepended — and import it via a `data:`
URL, which both hosts' ESM loaders accept).

## Acceptance (red-first, the harness is the wall)

`docs/design/beamr/briefs/evidence/fix-bundle-packager-noarg/noarg-init-harness.mjs`
drives the packaged bundle's no-arg init BY NAME in both hosts — the Node
leg in-process, the browser leg in headless Chrome against a served page —
and fails at current bytes with exactly the two signatures above
(committed red outputs beside the harness). After the fix: both legs
resolve a VM whose `spawn` surface exists and whose module loads report
arrives; outputs committed green. The harness never approximates the
no-arg path by passing bytes — that approximation is precisely how the
defect stayed latent.

Gate bar: the four-leg battery COLD at the final head (build.rs is
beamr-wasm crate source); the wasm suite stays 80/80 untouched.

## Successor (pointer only — WPORT-9, 2026-07-24)

The no-arg init path this fix restored is now driven as **permanent CI**
by the WPORT-9 conformance driver (`conformance/driver.mjs`, brief
`docs/design/beamr/briefs/WPORT-9.json` R3): the `node-singlefile-noarg`
and `browser-singlefile-noarg` legs call `createPreloadedVm()` with no
argument — never bytes — in both hosts on every push. This lane's
harness and its committed red/green outputs remain immutable as the
discovery-time evidence; the driver's legs are their permanent successor.
