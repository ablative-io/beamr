# beamr — Full Domain Ledger

*The complete work list and opportunity map, beyond the Frame v1 critical
path. Companion to `beamr-gaps-and-sizing.md` (which carries the v1-path
detail) and `beamr-assets-pack.md` (the pipeline toolkit). State as of
v0.12.0, July 2026. Sizing in briefs.*

## A. Performance tier

### A1. JIT wire-up (S: 1–2 briefs) — highest payoff-per-brief in the repo
Connect `JitProfiler::record_call` at the interpreter's call edges, submit
`CompilationJob`s when thresholds trip, and have the CLI load `.beamr_native`
companions. Everything else exists. **Unlocks:** the headline performance
feature, real; hot aion workflow steps run native. **Interacts:** generation
keying already handles hot-reload invalidation; telemetry should gain
compile/deopt counters; replay mode must keep the JIT *disabled* (replay
validates reduction counts, and JIT charges differ).

### A2. JIT full coverage (L: 12–20 briefs)
Ordered: (1) Y-register ABI fix — register file must carry X+Y (or the ABI
grows a Y-base pointer); today `Y(n)` would silently index past the 1024-word
x-block into adjacent `Process` fields. (2) Stack-frame ops
(`Allocate`/`Deallocate`/`Trim`/`InitYregs`/`TestHeap`). (3) `*Last`
tail-call variants (needs dispatch-model change: JIT entry currently
simulates whole-call-plus-return). (4) `Catch`/`CatchEnd`/`Raise`,
`SelectTupleArity`, `UpdateRecord`. (5) PC-based safepoint offsets so GC can
walk native frames. (6) `jit_send_message` beyond self-send. **Unlocks:**
whole-module compilation; the AOT north star stops being blocked.
**Verification:** every new lowering must extend the differential proptest
(interpreter vs JIT equivalence) — that harness exists and is the right
gate.

### A3. AOT north star (XL: design-first, months)
Whole-program compilation to a self-contained native binary: delete the
interpreter/loader at build time, tree-shake the runtime via demand-driven
BIF registration + the embedded archive. Explicitly R&D
(docs/AOT-NORTH-STAR.md). Gated on A2 = 100%. **Unlocks:** serverless/edge
cold-start (the edge-worker example becomes a product), tiny deployables,
"Gleam workflow → native binary" as a distribution format. Do not brief
until A2 lands and a design doc survives adversarial review.

### A4. Term-layer perf (S–M, independent items)
- Hashmap variant for large maps (flatmaps are linear-scan; Gleam `dict`
  usage will feel this at scale).
- REFC binary threshold (64B) tuning with benchmarks.
- `pg` exit-path global mutex (documented accepted contention) — revisit if
  process churn grows.
- Per-node sub-channels in `DistSender` (FUTURE note in code) for fan-out
  throughput.

## B. Determinism tier

### B1. Replay recording (M: 4–6 briefs)
Instrument the seams replay already consumes (message delivery order,
selective-receive indices, schedule slices, timer expiry, native results).
Single-threaded recording first — replay already forces `thread_count = 1`,
so this is symmetric, not a compromise. **Unlocks:** time-travel debugging
(`ReplayDebugger` already exists), deterministic CI reproduction of
concurrency bugs, and a VM-altitude determinism substrate under aion.
**Synergy — needs a design ruling:** the stack now has three replay layers
(beamr events, liminal durable conversation replay, aion workflow event
history). A one-page "determinism altitudes" doc should pin which layer
owns what before recording lands, or we'll build overlapping guarantees.

### B2. Reduction-boundary hook consumer (S, joint with norn)
The hook (ADR-009) is built, tested, and has zero in-repo consumers. It is
the natural seam for norn's observability at the lowest altitude —
"diagnostics as a live conscience" per doc 11. A norn-side consumer +
a beamr-side event-shape stabilization is a small joint brief with outsized
demo value for agents-supervising-workflows.

## C. Distribution tier

### C1. Remote link/monitor finish (M: 6–10 briefs) — named in v1 rulings
Encoders for LINK/UNLINK/EXIT/MONITOR_P/DEMONITOR_P/MONITOR_P_EXIT, wire
`ControlRouter` onto `DistSender`, scheduler-side remote-link state,
node-down cleanup, kill-9 e2e tests. **Unlocks:** cross-node supervision;
liminal crash-linked participants across nodes; frame's browser-peer
conversation model.

### C2. Browser transport (S–M: ~4 briefs)
WebSocket (or WebTransport) framing under the existing connection manager.
**Unlocks:** browser nodes join the mesh; liminal TS SDK gets a real
transport for free (it already ships the codec); frame Phase 9 unblocks.

### C3. Liveness + membership (joint with haematite, hangs off #146)
Enable/tune `HeartbeatConfig`; **multi-subscriber connection-down hook**
(the single slot is owned by pg-purge; three named consumers already).
The hook is the only beamr API change — small and high-leverage.

### C4. Distribution hygiene (S, background)
Atom-per-peer interning leaks under discovery-driven churn (CSOT-4
watchlist item on haematite's side); options: scoped atom tables or
peer-id indirection. Default cookie `"beamr-cookie"` should fail loudly in
non-dev builds. Heartbeats should arguably default ON.

## D. Sandbox and capability tier

### D1. Capability parameterization (S–M: 2–5 briefs, co-design with frame F-1b)
From six boolean authorities to parameterised grants ("read namespace X",
"reach host Y", "spawn under supervisor Z"), per-component grant tables,
deny-by-default, inspectable at runtime. The audit seam (sink + violation
handler) already exists — add a persistent sink so frame's audit-trail
premise ("a click and a tool call are indistinguishable") has VM-level
evidence. **Interacts:** Wave-1 ruling #5 (wasm-host location) decides which
engine enforces the same grant table for wasm components.

### D2. WasmScheduler maturation (S–M)
Cooperative runtime landed (WR-0..10) but: `DirtyCall` is rejected outright
(long-running native work in the browser needs an async-offload story),
supervision parity with the threaded scheduler deserves a dedicated
conformance suite, and the rAF pump has no backpressure signal to the host.

## E. Tooling and hygiene tier

- **CI (S)**: no `.github/` exists; encode the gate bar (fmt, check, test,
  clippy -D warnings, wasm build) as Actions. The pipeline dogfood makes
  local-only gates a liability.
- **Docs debt (S)**: `docs/files/` essays predate the crate layout (old
  "bearmr" spelling, a `beamr-loader`/`beamr-meridian` split that never
  happened); superseded specs (ACTOR-MIGRATION, MESSAGING-FIX,
  LOCAL-SEND, DISTRIBUTION-*) should gain a one-line "landed, historical"
  header. RELEASE_CHECKLIST still references a nonexistent `differential`
  feature. `ExecError::GcNeeded` is a documented dead variant.
- **gleam-types expansion (M, opportunistic)**: richer type descriptors
  would widen JIT typed specialization beyond int arithmetic; and see the
  cross-domain opportunity below.

## F. Cross-domain synergies spotted during orientation

1. **gleam-types × frame-mcp**: frame wants MCP tool schemas derived
   mechanically from component action declarations. Gleam function
   signatures via gleam-types sidecars could generate those schemas at
   package time — typed Gleam actions become typed MCP tools with no
   hand-written schema. Worth a spike when frame Phase 6 approaches.
2. **Replay × norn sessions × aion history**: the determinism-altitudes
   design doc (B1) should be co-authored across the three domains.
3. **Capability audit sink × frame identity (Phase 8)**: one audit path
   from VM violation events to frame's participant audit trail.
4. **Edge-worker example × frame deployment**: the Cloudflare bundle
   pipeline (BEAMR_EMBED archive + wasm-pack) is a working prototype of
   frame's "component bundle" distribution format.

## Explicitly deferred (agreed, revisit post-v1)
JIT full coverage beyond wire-up (A2), AOT (A3), replay recording (B1)
unless the determinism doc pulls it earlier, hashmap maps (A4) until a
consumer feels the linear scan, atom-table GC (C4) until discovery-driven
churn is real.
