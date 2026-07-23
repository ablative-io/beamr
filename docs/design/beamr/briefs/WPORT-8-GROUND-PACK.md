# WPORT-8 ground pack — async capability adapters (fetch + KV storage)

**Status:** v1 — evidence assembled, tear questions open
**Author:** Osiris Yogo (beamr seat), 2026-07-23
**Evidence pin:** `fb3efcf` (current main; carries 0.16.2)
**Pin discipline:** every §3 citation was gathered at `fb3efcf` by a
read-only sweep and spot-checked; the brief's build worker MUST re-verify the
pin-shift set at its actual base commit before writing a line — believed
state is not citable state. Note: the WPORT-3 flake-triage branch
(`triage/wport3-deadline-requests-flake`, in tear) moves
`crates/beamr-wasm/src/lib.rs` test-module lines and the CI carriers 65→66;
if it lands first, the §3.12 carrier counts shift by exactly that amendment.

## 1. The frozen boundary, verbatim

Arc scope (`docs/design/beamr/WASM-PORT-ARC.md:165` at `fb3efcf`):
"Implement the selected browser fetch/storage host NIFs as explicit
asynchronous capabilities. Completions re-enter through WPORT-4's
event-delivery seam and conform to WPORT-5's supported-service profile. This
brief does not emulate TCP, a native filesystem, or dirty pools and must not
acquire a threaded-runtime dependency."

Acceptance shape (`:169`): "Each selected adapter proves success, host
refusal, malformed response, and Promise rejection against a real browser
host seam. Completion from true idle schedules exactly one runtime turn and
delivers one durable result. Build and dependency checks prove the wasm
closure remains free of `threads`, native `net`, native `fs`, and dirty-pool
requirements. Tests fail if an adapter blocks, polls, silently maps errors
to success, or presents itself as TCP/filesystem emulation."

Open question 5 (`:221`): "Which fetch and storage operations constitute
WPORT-8's selected first capability set? … Decide with named target
workloads, least-authority capability requirements, browser compatibility
evidence, cancellation semantics, and the WPORT-5 support-profile decision."

Dependencies (`:163`): WPORT-4 and WPORT-5 — both CLOSED (`:98`, `:115`).

## 2. The OQ5 direction (Artemis Peach, domain owner, 2026-07-23 — direction, not a sealed ruling; the R-numbers must earn it)

Quoted from the rung-confirmation DM:

> first capability set = FETCH + KV STORAGE. Named workloads grounding it:
> the edge-worker example (fetch + KV is its native idiom) and the
> BROWSER-OTP north-star app class (fetch + storage). Fetch: request/response
> with typed refusal/malformed/rejection legs per the acceptance shape,
> cancellation via AbortController mapped onto the async-NIF completion seam
> (a cancelled request completes with a typed cancellation, never hangs,
> never polls); response streaming EXCLUDED from v1 (no consumer
> requirement; banked like WPORT-10). Storage: KV-shaped —
> get/put/delete/list-by-prefix — chosen because one shape is satisfiable by
> Workers KV, an IndexedDB adapter, and an in-memory test host without API
> drift. Least-authority law: both arrive as HOST-INJECTED capability
> objects (no ambient globals reached from Rust; the closure stays free of
> net/fs/threads — the WPORT-8 non-goals hold). Completions re-enter through
> the WPORT-4 seam; profile rows land in BROWSER-BIF-PROFILE's vocabulary.

## 3. Current-state evidence (all cites at `fb3efcf`)

### 3.1 The async-NIF registration seam is the adapters' substrate

`WasmVm::register_async_nif(module, function, arity, callback)`
(`crates/beamr-wasm/src/lib.rs:246-266`) interns the MFA, records the
callback in the async bridge keyed `(module_atom, function_atom, arity)`
(`:255-256`), and registers the MFA against the `wasm_async_nif_stub`
trampoline (`:257-265`). The stub (`:1594-1605`) reads the current MFA and
the wasm async-NIF facility from context and calls
`facility.start_async_nif`. `HostWasmFacility` (`:1565-1585`) routes
`wasm_ffi:js_callback` to the named-callback store and every other MFA to
`HostAsyncNifs`; installed into the scheduler at construction (`:111-119`).
`HostAsyncNifs::start_callback` (`:1342-1371`) marshals args via
`terms_to_js_array`, invokes the callback, and — when the return is
promise-like (`:1607-1611`) — starts promise completion and suspends the
calling process (`context.request_suspend(None)`).

### 3.2 The completion path already has both twins and the arbiter edge

`start_promise_completion` (`:1373-1408`) awaits the JS promise via
`spawn_local`/`JsFuture`. Fulfilment → `WasmAsyncCompletion::Ok(term)`
(`:1378-1385`); **rejection → `WasmAsyncCompletion::Error(term)` built from
the rejection value** (`:1386-1393`). It then calls
`scheduler.complete_async(pid, completion)` and, on a taken runnable edge,
`arbiter.request_external_turn(FailureLeg::Promise)` (`:1394-1402`); a wake
failure goes to `arbiter.fail` (`:1404`). Scheduler side
(`crates/beamr/src/scheduler/wasm.rs:266-275`): bytecode pids store the
completion and wake (x0 injection next slice); native pids route to
`deliver_native_async_completion`
(`crates/beamr/src/scheduler/wasm_native.rs:266-294`) which delivers
`{ok, Value}` / `{error, Reason}` (`:271-274`) as a mailbox message.
**Consequence: adapters add ZERO new wake-path call sites** — completions
ride the existing Promise leg, the WPORT-4 acceptance-closure pattern.

### 3.3 Facility injection today

Cooperative `NativeServices` supplies exactly five services
(`crates/beamr/src/scheduler/wasm.rs:792-811`): atom table, wasm async-NIF
facility, WPORT-3 timers, BIF registry, io_sink. The facility trait is
single-method (`crates/beamr/src/native/context/mod.rs:327-335`); the opcode
side installs it at `interpreter/opcodes/native_call.rs:168`.

### 3.4 The failure-leg vocabulary is closed at five

`LEG_SLUGS = ["queued","manual","deadline","promise","spawn_edge"]`
(`crates/beamr-wasm/src/failure.rs:63`), closed-set wall
`failure_leg_slug_set_is_closed_and_exact`
(`failure_tests.rs:274-295`), CI-pinned. Adapter completion wake failures
classify under the existing `promise` leg (§3.2) — the closed set need not
open.

### 3.5 Injected-capability precedent (WPORT-6)

`load_artifacts(manifest_url, fetch: Function)`
(`crates/beamr-wasm/src/artifact_loader.rs:88-95`): the fetch function is
INJECTED — "No global fetch is probed; explicit injection is the whole
contract" (doc `:75-79`). Invocation coerces a thenable and accepts
`Uint8Array`/`ArrayBuffer`, everything else a typed `fetch_protocol`
rejection (`:479-514`). This is the least-authority precedent the OQ5
direction generalizes.

### 3.6 Error-class house pattern (three instances)

Named `js_sys::Error` + `"{kind}: {detail}"` message; async-operational
classes carry ONE `data` property holding a JSON string:
`ArtifactLoadError` (8-slug closed kind set, `artifact_loader.rs:41-50`,
minter `:532-554`, closed-set wall `artifact_loader_tests.rs:556-572`);
`SchedulerFailureError` (`failure.rs:107-123`); sync caller-protocol
violations use `ConnectionEventProtocolError` (no data property,
`connection_events.rs:515-519`). The adapters' typed
refusal/malformed/rejection/cancellation legs have an exact mold to follow.

### 3.7 JS↔Term codec at the boundary

`convert.rs`: `js_value_to_owned_term` (`:25-35`),
`js_value_to_term_in_context` (`:38-43`), `term_to_js_value` (`:77-79`),
`terms_to_js_array` (`:68-74`). By-value copy; string→UTF-8 binary,
non-UTF-8 binary→`Uint8Array`, object→sorted map, max depth 256; `pid`
terms refuse conversion (`:242-244`). Response payloads (status, headers
map, body binary) fit this codec today.

### 3.8 Dependency closure walls

`crates/beamr-wasm/Cargo.toml:18-21`: `beamr` with
`default-features = false, features = ["cooperative","json"]`;
`cooperative = ["std", "dep:crossbeam-queue"]`
(`crates/beamr/Cargo.toml:74`) pulls none of threads/net/fs/jit/embedded
(`:75-81`); no dirty-pool feature exists outside the threaded scheduler
topology. CI compiles both closure legs for wasm32
(`.github/workflows/cooperative-wasm.yml:17-20`) — native net/fs/threads
deps do not build for wasm32, so compilation is itself the wall — plus the
profile seal test (`crates/beamr-wasm/tests/profile_seal.rs`).

### 3.9 The sealed profile has no fetch/storage rows

`BROWSER-BIF-PROFILE.md`: 197 rows sealed (`:68`, `:78`); grep finds NO
fetch/http/file/inet/gen_tcp/storage BIF registered. Ports family:
`erlang:ports/0` WORKS (truthful `[]`, `:220`), `erlang:port_info/1` APPROX
(`:221`), `erlang:open_port/2` deliberately-unsupported badarg (`:222`).
net/fs families are cfg-compiled-out, not registered (`:406-416`).
**Consequence: WPORT-8's BEAM-facing MFAs are NEW registered surface — the
seal must move by design, same-commit with the rows (TQ2).**

### 3.10 Named workload: edge-worker — grounding with an honest gap

`examples/edge-worker`: stateless Cloudflare Worker, module-scope VM cache,
per-request spawn + `awaitExit` (`src/worker.js:6-21`, `:74-75`), HTTP
request marshalled as a plain object through the JSON/Term path
(`:31-39`). **It does NOT use fetch() or KV today** (`README.md:60-62`) —
the platform's native idiom (fetch + Workers KV bindings) is the workload
CLASS grounding the selection, but no existing wall exercises the adapters;
the brief must build that proof, not cite this example as if it already
consumed them. README's WPORT-7 recovery-contract note (`README.md:61`) is
the consumer-lockstep precedent for any adapter-visible failure behavior.

### 3.11 Native-actor delivery contract

Native handlers parked on async work receive completions as mailbox
messages `{ok, Value}` / `{error, Reason}`
(`wasm_native.rs:271-274`); bytecode callers resume with the completion in
x0. The adapters' cancellation completion must pick a term shape coherent
with BOTH paths (TQ4).

### 3.12 CI carriers

65 pinned test names (`cooperative-wasm.yml:41-107`), exact-summary grep
(`:115-119`), borrow wall (`:122-127`). The WPORT-3 triage amendment in tear
moves these to 66; the brief's arithmetic re-verifies at its base.

## 4. Decision analysis: earning the OQ5 direction

**Named target workloads.** (a) The edge-worker platform class: Cloudflare
Workers hand every request handler `fetch` and KV bindings as ambient
platform objects — the host-injected capability shape maps 1:1 onto passing
those bindings into the VM (§3.10, honest gap noted). (b) The BROWSER-OTP
north-star app class (`docs/BROWSER-OTP-NORTH-STAR.md`) is fetch + storage
shaped. No third workload is claimed.

**Least-authority.** Host-injected capability objects generalize the
WPORT-6 injected-fetch contract (§3.5): Rust reaches no ambient global; a
VM without a registered capability refuses with a stable typed error
(profile row: unsupported-until-injected — exact vocabulary TQ3); the
closure walls (§3.8) stay untouched because the adapters are pure JS-seam
consumers.

**Cancellation semantics.** AbortController lives host-side inside the
capability object; the async-NIF completion seam already delivers both
twins (§3.2), so a cancelled request is a completion — a typed cancellation
value through the SAME path, never a hang, never a poll. NO-POLLING is
structurally satisfied: request→suspend, completion→one coalesced turn via
the existing Promise leg.

**Why KV (get/put/delete/list-by-prefix).** One shape satisfiable by
Workers KV, an IndexedDB adapter, and an in-memory test host without API
drift — the acceptance's "real browser host seam" can then be a thin
adapter over any of the three, and the walls run against the in-memory host
under Node with the browser probe carrying platform confirmation (TQ5).

**Why no streaming.** No consumer requirement names it; banked verbatim
like WPORT-10 (arc `:213` pattern): opening it requires a consumer
requirement and the reviewer of record's word.

## 5. Hazards and boundaries

- **Non-goals restated as walls** (arc `:165`, `:169`): no TCP or
  filesystem presentation; dependency checks prove the closure free of
  threads/net/fs/dirty-pools; an adapter that blocks, polls, or silently
  maps errors to success fails its wall.
- **Zero new wake-path call sites** — completions ride the existing
  `FailureLeg::Promise` edge (§3.2, §3.4); the failure-leg closed set stays
  closed; any apparent need for a new call site is a STOP.
- **Seal movement** — new profile rows move the sealed 197-row table and
  its seal test same-commit (§3.9); silent registration without profile
  rows is the WPORT-5 registration-must-not-imply-support violation.
- **Threaded byte-identity** — adapters live entirely in `beamr-wasm` and
  the cooperative facility seam; shared interpreter/native code stays
  byte-identical (the WPORT-5 `apply_common` precedent, arc `:115`).
- **No new dependencies** — js-sys/wasm-bindgen suffice (WPORT-6/7
  precedent); AbortController is host-side JS, invisible to Rust.
- **Marshalling hazards** — non-UTF-8 bodies arrive as `Uint8Array`→binary
  (§3.7); depth-256 and pid-refusal limits apply to response terms; header
  maps are sorted-key maps by codec construction.
- **Consumer lockstep** — any edge-worker-visible surface change moves its
  README same-commit (§3.10 precedent).

## 6. Open tear questions

- **TQ1 — BEAM-facing MFA surface.** Module/function/arity names for the
  fetch and KV NIFs (e.g. a `wasm_fetch` / `wasm_kv` module pair vs one
  `wasm_capability` module), and whether bytecode and native actors share
  the registration path unchanged (§3.1 suggests yes).
- **TQ2 — Seal-move procedure.** The exact commit discipline for opening
  the sealed 197-row profile: rows + seal test + doc tally in one commit,
  and who signs the seal move (domain owner at tear vs build commit).
- **TQ3 — Error vocabulary.** One new closed slug set per adapter (fetch:
  refused / malformed_response / rejected / cancelled + protocol legs; KV:
  its own) delivered BEAM-side as what term shape, and JS-side via a fourth
  named error class following the `ArtifactLoadError` data-property mold —
  or reuse of an existing class. Coherence with the `{ok,V}/{error,R}`
  native mailbox contract (§3.11).
- **TQ4 — Cancellation surface.** Who requests cancellation from the BEAM
  side (a cancel BIF taking a request ref? scope-exit auto-abort? both),
  the cancellation completion's term shape, and whether an
  already-completed request's late cancel is a counted no-op (the WPORT-3
  stale-token pattern).
- **TQ5 — Browser acceptance split.** Node walls against the in-memory
  host + AUTHORED-NOT-RUN browser probe (house pattern, §3.10 of the
  WPORT-7 precedent) vs any CI browser leg now; WPORT-9 keeps the
  permanent gate either way.
- **TQ6 — Workload wall.** Extend edge-worker to exercise fetch+KV
  end-to-end (closing §3.10's honest gap) or a new example; either moves
  docs same-commit.
- **TQ7 — Refusal-before-injection.** The exact behavior of a fetch/KV MFA
  called with no capability registered: stable badarg-class refusal with
  reason vocabulary (WPORT-5 pattern) — profile row category
  supported-when-injected needs Artemis's naming.
- **TQ8 — Response term shape.** `{status, headers_map, body_binary}` vs a
  map; size/limit posture for bodies (depth-256 interacts with header
  maps); whether request bodies accept `Uint8Array` symmetrically.
