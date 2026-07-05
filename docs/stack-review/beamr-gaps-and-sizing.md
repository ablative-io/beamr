# beamr — Gaps and Completion Sizing

*State as of v0.12.0 (main @ 58987bb, July 2026). Sizing is in briefs (one
dispatched agent-workflow unit) and is an estimate, not a commitment. Verify
each gap against git before dispatching — these go stale.*

The recurring pattern across all four areas: **the machinery is built; the
last wire is missing.** That makes most of this work unusually well-bounded —
the existing code defines the contract the missing piece must satisfy.

## 1. JIT — two very different jobs

### 1a. Wire-up (small: 1–2 briefs)

The profiler (`JitProfiler`, per-MFA counters, INTERPRETING→PENDING→COMPILED
state machine, tunable threshold), off-thread compilation on the dirty-CPU
pool (`CompilationJob`/`submit_jit_compilation`), and the generation-keyed
`JitCache` all exist and are tested. **Nothing in the production run path
calls `record_call` or submits compile jobs.** The scheduler owns a profiler
it never feeds; benches inject the cache manually; the CLI never loads AOT
companions. Connecting this loop is the best effort-to-story ratio in the
whole backlog: it makes "Cranelift JIT for near-native performance" true at
runtime.

### 1b. Full coverage (large: 12–20 briefs)

- **Fix the Y-register ABI hazard first.** `register_offset` maps `Y(n)` to
  word `1024+n` past the register-file base, but the interpreter passes
  exactly 1024 words (`x_regs_mut()`); a Y access would silently read/write
  adjacent Process fields. Currently unreachable only *because* framed
  functions don't compile. Any frame-op work makes it reachable.
- **Lower the stack-frame ops** (`Allocate`/`AllocateHeap`/`AllocateZero`/
  `Deallocate`/`Trim`/`InitYregs`/`TestHeap`) — today only frameless leaf
  functions compile (~42/64 instruction variants lowered).
- **Lower `CallLast`/`CallExtLast`/`ApplyLast`** — blocked on the dispatch
  model (JIT entry currently replaces the whole call and simulates return).
- **`Catch`/`CatchEnd`/`Raise`**, `SelectTupleArity`, `UpdateRecord`.
- **Real PC-based safepoint offsets.** Stack maps currently record BEAM
  instruction indices, not machine-code offsets — GC cannot walk native
  frames. Required before compiled functions can allocate across a GC.
- **Fix `jit_send_message`** — currently handles self-send only; sends to any
  other pid silently return without delivering (known, documented bug).

Nothing in frame Phases 0–8 needs any of this; it's pure performance.

## 2. AOT (defer: multi-month R&D, design-first)

Today's "AOT" (`beamr compile`, `.beamr_native` bundles) is a **warm-start
cache**: bundles store the original bytecode + checksums + type sidecars and
recompile through the JIT at load. No machine code is persisted (Cranelift
addresses are process-local). The north star (docs/AOT-NORTH-STAR.md,
explicitly R&D) is whole-program AOT to a self-contained native binary —
delete the interpreter, tree-shake the runtime, GraalVM-Native-Image analogy.
It is gated on 100% opcode lowering (1b) and the named correctness bugs. Not
briefable yet; needs a design phase. Park it.

## 3. Replay recording (medium: 4–6 briefs)

Asymmetric in our favour: **consumption is fully wired** — replay-mode
scheduler (forces single thread, disables distribution/timers), memoized
native calls (natives are not re-executed on replay), strict cursor with
hard `ReplayMismatch` on divergence, plus a single-step `ReplayDebugger`.
The consumer side precisely defines the event vocabulary a recorder must
emit: `Select`, `MessageDelivery` (total order + logical clocks),
`Schedule`, `TimerExpiry`, `NativeCall`.

The gap: `beamr record` writes a log with an **empty event vec** (CLI
transcript only). Recording means instrumenting the seams replay already
consumes. One honest constraint: capturing a total delivery order on the
multi-threaded scheduler is invasive — record single-threaded first (replay
already forces `thread_count = 1`, so this is symmetric, not a compromise).

Not frame-blocking (aion's durability is its own event-sourcing layer), but
this is the deeper determinism substrate and a strong debugging story.

## 4. Distribution finish (medium: 6–10 briefs + optional transport)

**Done and tested**: OTP v6 handshake (sync + async twins, MD5 cookie
challenge, constant-time compare, simultaneous-connect tie-break with a
timing-independent canonical-direction survivor rule), connection manager
keyed by authenticated peer name, bounded outbound sender with per-node FIFO,
pg process groups end-to-end (join/leave propagation, node-down purge,
2-node e2e test), simplified `global` name registry.

**The gap**: remote link/monitor controls (`LINK`/`UNLINK`/`EXIT`/
`MONITOR_P`/`DEMONITOR_P`/`MONITOR_P_EXIT`) are structurally complete —
`ControlRouter` and `control_monitor` correlate them — but they **buffer
in-memory and never reach the wire**. Cross-node EXIT signals do not fire.
Work: encoders for the missing control ops, wiring onto `DistSender`,
scheduler-side remote-link state, node-down cleanup, e2e tests.

This matters downstream: liminal's crash-linked participants across nodes,
and frame's browser-as-peer conversation model, both assume remote exit
propagation.

**Additional item (~4 briefs, separate)**: a WebSocket/WebTransport transport.
Current transport is raw TCP; browser nodes cannot join the mesh without it.

Also worth noting: `SPAWN_REQUEST`/`SPAWN_REPLY` are encoded/decoded and
handled, but remote link/monitor metadata is not yet plumbed into the
scheduler on the receiving side.

## 5. Capabilities (small: 2–5 briefs, design-first with frame)

What exists is sound but coarse: a six-variant authority enum (`Pure`,
`ProcessLocal`, `Clock`, `Entropy`, `ExternalIo`, `Spawn`), sandbox presets,
two enforcement points (load-time import resolution → `Denied` import slots;
runtime call-time audit with pluggable sink/violation handler), all in ~500
lines. Namespaces (`NamespaceId` → per-namespace `ModuleRegistry`) isolate
module registries only — atom table, BIFs, and the process table are shared.
**The capability layer, not namespaces, is the security boundary.**

Frame's F-1b wants per-component grants, deny-by-default, an inspectable
grant table, and *parameterised* capabilities ("read this namespace", "reach
this host") rather than boolean `ExternalIo`. Do the design jointly with
F-1b before either side builds, or frame will bolt a second capability model
on top of beamr's.

**Scoping fact that needs an explicit decision**: beamr's capability sandbox
governs bytecode/native processes. It is **not a WASM sandbox**. Frame's
RM-002 (wasm32 component host) needs an actual wasm engine (wasmtime or
similar) as its enforcement boundary — new frame work, not latent beamr work.
Decide: embed a wasm host in frame-core, or teach beamr a wasm-host native
process type.

## Priority relative to frame

| Item | Size | When |
|---|---|---|
| Capability co-design with F-1b | S (2–5) | First — cheap, prevents rework |
| JIT wire-up | S (1–2) | Opportunistic — best story-per-brief |
| Distribution link/monitor finish | M (6–10) | Before any multi-node / browser-peer milestone |
| WebSocket transport | S-M (~4) | With/after distribution finish, before Phase 9 |
| Replay recording | M (4–6) | Background track — debugging + determinism value |
| JIT full coverage | L (12–20) | Background track — performance only |
| AOT north star | XL (months) | Parked pending design phase + 100% lowering |
