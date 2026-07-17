# Beamr wasm port arc sizing

**Status:** Arc sizing document — 2026-07-14 — evidence pinned at `main` `d9de35e`  
**Owner:** Artemis Peach  
**Reviewer of record:** Waffles the Terrible

This arc makes the existing cooperative wasm substrate constructible, runnable, and conformant with the browser event loop without importing the native threaded service bundle. The substrate already contains caller-provided module loading, cooperative spawn/turn APIs, receive-timer registration, Promise completion, callbacks, and native actors (`crates/beamr-wasm/src/lib.rs:101-205`; `crates/beamr-wasm/src/lib.rs:220-315`; `crates/beamr-wasm/src/lib.rs:494-629` at `d9de35e`); its blocked-to-runnable edge already coalesces in Rust, but no cooperative wake schedules a JS turn (`crates/beamr/src/scheduler/wasm.rs:300-360` at `d9de35e`). WPORT-1 establishes a truthful executable baseline; WPORT-2 establishes the event-loop contract and removes polling; WPORT-3 through WPORT-8 supply deadlines, event sources, service truthfulness, loading, failure visibility, and selected async capabilities; WPORT-9 makes the result permanent in conformance CI.

## Positioning against the prior paper

[`docs/WASM-RUNTIME-PORT-DESIGN.md`](../../WASM-RUNTIME-PORT-DESIGN.md), dated 2026-06-27, planned native-actor support on `WasmScheduler`. That substrate has largely landed: the wasm binding exports `spawn_actor`, Promise-based `call`, and `cast` (`crates/beamr-wasm/src/lib.rs:220-280` at `d9de35e`), and the wrapper contains concrete native-actor, scheduler-turn, receive-timer, Promise-completion, and callback implementations rather than paper-only placeholders (`crates/beamr-wasm/src/lib.rs:220-315`; `crates/beamr-wasm/src/lib.rs:494-629` at `d9de35e`). Cooperative root native processes can be queued and executed with bytecode processes, and handler effects include deferred local sends and native spawning (`crates/beamr/src/scheduler/wasm_native.rs:192-207`; `crates/beamr/src/scheduler/wasm_native.rs:307-348`; `crates/beamr/src/scheduler/wasm_native.rs:350-425` at `d9de35e`).

This arc is the successor, not a restatement. It is scoped to real runnability and browser-event-loop conformance under the later NO-POLLING ruling, which the 2026-06-27 paper predates. It serves the vision in [`docs/BROWSER-OTP-NORTH-STAR.md`](../../BROWSER-OTP-NORTH-STAR.md) without duplicating either prior document.

The evidence pack proposed candidate work packages. Artemis's 2026-07-14 outline supersedes that candidate decomposition: the nine boundaries and numbering below are frozen.

## Binding laws

### NO-POLLING

**NO-POLLING (Tom's ruling): event-loop-driven scheduling — run-to-idle, wake on message; a polling scheduler is a TEAR CONDITION (F-0d acceptance). Any timer whose job is “check whether something changed” is a design error.**

A one-shot timeout that delivers a known deadline is permitted. A recurring animation-frame, timer, or synchronous loop that merely checks whether progress, state, or time has changed is not.

### Citable state

**Believed state is not citable state: every beamr-state claim carries file:line at `d9de35e` or is a named socket/gate.**

`VERIFIED` and `INFERRED` retain the evidence pack's meanings. `VERIFIED` means the cited source bytes or command output were inspected at the pinned commit. `INFERRED` means the conclusion follows from cited source but the exact behaviour was not directly executed. Prospective acceptance shapes are requirements, not claims about current state.

## Current-state summary: compiles ≠ constructs ≠ runs

- **The CI closure proves type-check only — VERIFIED/INFERRED.** CI runs two `cargo check` commands for `wasm32-unknown-unknown`: `beamr` with defaults disabled and `cooperative,json`, and the `beamr-wasm` manifest (`.github/workflows/cooperative-wasm.yml:13-20` at `d9de35e`). The wrapper is a `cdylib`/`rlib` selecting only `beamr/cooperative,json` (`crates/beamr-wasm/Cargo.toml:10-25` at `d9de35e`). The workflow has no wasm test runner, wasm-bindgen packaging, browser, Node, module execution, or event-loop test step (`.github/workflows/cooperative-wasm.yml:13-20` at `d9de35e`).
- **The checked wrapper does not construct — VERIFIED, `RUNTIME-REFUSAL`.** `WasmVm::new` registers the wasm-safe BIF set and returns on registration error (`crates/beamr-wasm/src/lib.rs:59-70` at `d9de35e`). That set calls gate 1, gate 2, exception, and ETF registration (`crates/beamr-wasm/src/lib.rs:779-788` at `d9de35e`), while gate 1 already registers exception and ETF BIFs (`crates/beamr/src/native/bifs.rs:47-65` at `d9de35e`). The exception set includes `erlang:raise/3` (`crates/beamr/src/native/exception_bifs.rs:9-23` at `d9de35e`), and the registry rejects duplicate MFAs (`crates/beamr/src/native/mod.rs:251-268` at `d9de35e`). Both wasm VM tests fail at constructor calls (`crates/beamr-wasm/src/lib.rs:918`; `crates/beamr-wasm/src/lib.rs:973` at `d9de35e`) with `native function already registered for Atom(76):Atom(125)/3`.
- **`run_until_idle` is one snapshot, not quiescence — VERIFIED.** It snapshots `ready.len()`, runs at most that budget, defers every yielded PID, and restores yielded PIDs only after the loop (`crates/beamr/src/scheduler/wasm.rs:427-439`; `crates/beamr/src/scheduler/wasm.rs:495-538` at `d9de35e`). Yielded work therefore requires another host call through `run_step` or `pump_once` (`crates/beamr-wasm/src/lib.rs:192-205`; `crates/beamr-wasm/src/lib.rs:419-426` at `d9de35e`).
- **The cooperative wake edge exists in Rust but does not schedule JavaScript — VERIFIED.** `wake` removes a PID from `waiting`, changes it to running, and adds one ready entry; a PID no longer in `waiting` does not receive a duplicate ready edge (`crates/beamr/src/scheduler/wasm.rs:300-313` at `d9de35e`). No cooperative wake method invokes JS or requests a host callback; it only mutates scheduler state (`crates/beamr/src/scheduler/wasm.rs:300-360` at `d9de35e`). `send_message`, `cast`, Promise completion, and receive-timer completion likewise mutate Rust state without pumping or requesting a turn (`crates/beamr-wasm/src/lib.rs:120-129`; `crates/beamr-wasm/src/lib.rs:269-280`; `crates/beamr-wasm/src/lib.rs:458-480`; `crates/beamr-wasm/src/lib.rs:558-581` at `d9de35e`).
- **Two production polling drivers violate NO-POLLING — VERIFIED.** The rAF driver runs one turn, tests `has_pending_work`, and requests another animation frame while that flag remains true (`crates/beamr-wasm/src/lib.rs:349-374` at `d9de35e`). An armed native timer alone keeps `has_pending_work` true (`crates/beamr/src/scheduler/wasm.rs:578-596` at `d9de35e`), so the next frame merely ticks the passive timer wheel again (`crates/beamr/src/scheduler/wasm.rs:363-388`; `crates/beamr/src/scheduler/wasm.rs:427-434` at `d9de35e`). Separately, generated `runUntilExit` synchronously calls `run_step` up to 1,024 times (`crates/beamr-wasm/build.rs:277-293` at `d9de35e`).
- **The typed mailbox API from `2cf3085` is not the wasm primitive — VERIFIED.** `MailboxSendError` and `Scheduler::send_to_mailbox` are both `threads`-gated (`crates/beamr/src/scheduler/mod.rs:27-57`; `crates/beamr/src/scheduler/mod.rs:2102-2159` at `d9de35e`). The cooperative primitive is `WasmScheduler::send_owned`, returning `ExecError`, together with its coalescing blocked-to-ready edge (`crates/beamr/src/scheduler/wasm.rs:300-360` at `d9de35e`).

## Frozen briefs

The eventual brief files must scaffold the whole board: scope, dependencies, evidence, silence-attacking acceptance, and named sockets or gates. The entries below define acceptance shape, not full numbered requirements.

The citable-state law binds every boundary below: every current-state justification is pinned to file:line at `d9de35e` or identified as a named socket/gate.

### WPORT-1 — Baseline truthfulness

**Size:** S  
**Dependencies:** None.

**Scope.** Remove duplicate BIF registration so `WasmVm::new` constructs; preserve and pass the existing wasm-bindgen tests; make CI execute those tests rather than stopping at type-check. This brief establishes the first executable fact on which every later browser claim depends. It does not redesign scheduling.

**Boundary evidence.** CI currently runs only `cargo check` (`.github/workflows/cooperative-wasm.yml:13-20` at `d9de35e`). The constructor fails because the wrapper registers gate 1 and then separately registers exception and ETF BIFs already included by gate 1 (`crates/beamr-wasm/src/lib.rs:59-70`; `crates/beamr-wasm/src/lib.rs:779-788`; `crates/beamr/src/native/bifs.rs:47-65` at `d9de35e`), and duplicate MFA registration is refused (`crates/beamr/src/native/mod.rs:251-268` at `d9de35e`). Existing VM tests assert construction before workload execution (`crates/beamr-wasm/src/lib.rs:918`; `crates/beamr-wasm/src/lib.rs:973` at `d9de35e`).

**Acceptance shape.** A clean CI job builds the pinned cooperative closure, runs the wasm-bindgen test suite, constructs a VM, and reaches the existing actor workloads. The wall fails if CI only type-checks, if constructor errors are ignored or matched as expected failures, or if tests substitute a JavaScript VM.

**Named socket/gate filled.** None named by the frozen outline; this is the executable baseline prerequisite.

### WPORT-2 — Event-loop scheduling core

**Size:** L  
**Dependencies:** WPORT-1.

**Scope.** Define one coherent scheduling design covering the three candidate concerns that are frozen into this single brief: a real run-to-idle contract; an edge-triggered JS wake arbiter; and retirement of both polling drivers. The run result must distinguish immediate draining, an explicit fairness yield to the browser, and true idle. “Check again next frame” is not an idle protocol. The arbiter schedules one coalesced microtask or macrotask per idle-to-runnable transition and no duplicates while a callback is already queued. The rAF pump and generated synchronous `runUntilExit` loop must cease to be runtime progress mechanisms.

**Boundary evidence.** Current `run_until_idle` processes one ready-queue snapshot and restores yielded PIDs after the loop (`crates/beamr/src/scheduler/wasm.rs:427-439`; `crates/beamr/src/scheduler/wasm.rs:495-538` at `d9de35e`). Rust already coalesces the first blocked-to-ready transition (`crates/beamr/src/scheduler/wasm.rs:300-360` at `d9de35e`), but no wake path requests JS execution (`crates/beamr/src/scheduler/wasm.rs:300-360`; `crates/beamr-wasm/src/lib.rs:120-129`; `crates/beamr-wasm/src/lib.rs:458-480`; `crates/beamr-wasm/src/lib.rs:558-581` at `d9de35e`). The rAF driver rechecks scheduler state (`crates/beamr-wasm/src/lib.rs:349-374` at `d9de35e`), and generated `runUntilExit` repeatedly calls `run_step` (`crates/beamr-wasm/build.rs:277-293` at `d9de35e`).

**Binding law.** **NO-POLLING applies here as F-0d acceptance. A polling scheduler is a TEAR CONDITION. Any timer whose job is “check whether something changed” is a design error.**

**Acceptance shape.** One host entry drains runnable work until either true idle or an explicit, observable fairness boundary. A burst that changes the scheduler from idle to runnable queues exactly one host turn; more events before that turn do not queue duplicates; an event after the VM becomes truly idle queues a new turn. An idle VM with no known deadline produces **zero recurring host callbacks**. Instrumented tests fail on recurring rAF, interval, timeout, or synchronous `run_step` rechecks. Completion is Promise/event driven rather than `runUntilExit` polling.

**Named socket/gate filled.** F-0d, the NO-POLLING scheduler acceptance gate.

### WPORT-3 — Host deadline service

**Size:** L  
**Dependencies:** WPORT-2.
**Status (2026-07-15):** CODE GATE CLOSED — landed on `main` at `a17be58` (brief `cb066df`, fold `9d0c311`), landing gates GREEN. The deadline pillar remains **OPEN** until one real browser+Worker PROBE-THROTTLE run attaches observations to `probes/WPORT-3-PROBE-THROTTLE.md` — a ~15-minute manual run against the built bundle, on Tom's or Annabel's word.

**Scope.** Unify receive-after timers and native `Deliver` timers behind one browser deadline service. The host is told the earliest known deadline and arms a single one-shot `setTimeout`; it cancels or re-arms that callback when the earliest deadline changes. Fired deadlines deliver expiries and schedule a runtime turn through WPORT-2's arbiter. Inject the timer facility required by bytecode timer BIFs. Timers are told, never discovered by polling.

**Platform bound (UNVERIFIED-ON-PLATFORM, tear condition T2).** Browsers throttle timers in backgrounded tabs — `setTimeout` clamping at minutes scale exists in the wild, and Worker contexts differ again — so a one-shot armed at the earliest deadline delivers LATE under throttling, by design of the platform. The brief carries **PROBE-THROTTLE**: background the tab/worker with an armed deadline and assert delivery is late-but-delivered and that timer semantics tolerate late fire (BEAM receive-after semantics do; the probe makes that stated rather than believed). No acceptance test in this brief may assert timing precision that flakes under throttling, and no liveness claim rides deadline promptness.

**Boundary evidence.** Receive timers already produce scheduler records, host schedules, and cancellations (`crates/beamr/src/scheduler/wasm.rs:148-174`; `crates/beamr/src/scheduler/wasm.rs:618-633` at `d9de35e`), and the wrapper maps them to one-shot `setTimeout`/`clearTimeout` calls (`crates/beamr-wasm/src/lib.rs:429-480` at `d9de35e`). Their callbacks wake Rust state but do not schedule a turn (`crates/beamr-wasm/src/lib.rs:468-471` at `d9de35e`). Native `Deliver` timers use a shared wheel and are found by ticking that wheel at the start of scheduler turns (`crates/beamr/src/scheduler/wasm_native.rs:350-385`; `crates/beamr/src/scheduler/wasm.rs:363-424` at `d9de35e`). Bytecode `send_after`, `start_timer`, and `cancel_timer` require an injected timer wheel (`crates/beamr/src/native/bifs.rs:254-304`; `crates/beamr/src/native/context/mod.rs:1043-1129` at `d9de35e`), while cooperative bytecode `NativeServices` currently injects only the atom table and optional wasm async-NIF facility (`crates/beamr/src/scheduler/wasm.rs:610-615` at `d9de35e`).

**Binding law.** **NO-POLLING applies here. A one-shot callback for a known deadline is event delivery; a recurring callback that checks the timer wheel is a design error and a tear condition.**

**Acceptance shape.** Across both timer classes, the host has at most one active callback for the earliest deadline. Adding an earlier timer cancels and re-arms; cancelling the earliest timer clears or moves the callback; adding a later timer does not create a second callback. Firing delivers all due expiries, queues one scheduler turn, and arms the next known deadline. With timers pending in the future, no rAF or other recurring check runs. Bytecode timer BIF tests prove the facility is present rather than returning the current missing-service refusal. PROBE-THROTTLE passes in a backgrounded tab and in a Worker context: the deadline fires late but fires, delivery is complete, and no test asserts wall-clock promptness.

**Named socket/gate filled.** The NO-POLLING deadline gate; no additional named socket.

**Board finding (2026-07-17, probe-first rider — Waffles the Terrible's ruling on the timer-wall probe thread):** the shared `TimerWheel` strands behind-cursor inserts for a full 1024-tick revolution (~1.024s): insert never consults the cursor (`crates/beamr/src/timer.rs:142-174`), the sweep only moves forward (`:207-219`), and not-yet-due entries are skipped in place with no rounds counter (`:259-286`) — mechanism confirmed at the bytes by Artemis Peach's probe answering Apollo Biscuit's TTL P3 distributions (`artemis-artifacts/2026-07-17-sched-cert/beamr-timer-wall-probe.md`; native-side behavior is inside haematite's certified §4.4 budget, nothing changes there). The wasm-side exposure is INFERRED, not yet run: the cooperative scheduler drives the same wheel (`crates/beamr/src/scheduler/wasm.rs:455-477`, earliest-deadline feed `:667`) and WPORT-3 made `erlang:send_after(0, ...)` live, so a mid-turn 0ms arm may strand behind the cursor with the unified one-shot re-arming an overdue deadline repeatedly (~1s of macrotask churn). RULED: confirm-or-kill measurement rides the NEXT wasm-timer-touching brief (WPORT-7+; WPORT-6's loader touches no timers) in the WPORT-3 probe pattern — no behavior change without cert re-sign.

### WPORT-4 — Event-source integration

**Size:** M  
**Dependencies:** WPORT-2.
**Status (2026-07-17):** CLOSED — landed on `main` at `db9edb3` (brief `bfe9607`, fold `5ddf47e`; chain rebased onto `c6cd26a` before land), landing gates GREEN at the reviewer's hands. Delivered: the five R1 acceptance walls (cast, bytecode Promise fulfilment/rejection twins, native-completion e2e + direct-injection pair, arbiter-driven trapped exit) closing the six-source acceptance shape below; the two tear-ordered seam comments (Rulings 6/7); the host-fed browser connection-event hub (`crates/beamr-wasm/src/connection_events.rs`) filling socket v0.2 with the ruled subscription ABI, locally-minted generations, the seven-variant reason mapping, and the ten-invariant classification in the module contract; three subscription walls; CI wall extended to exactly 26 same-commit. Zero new wake-path call sites — the tear's acceptance-closure reframe held at the bytes.
**Board finding from the build (2026-07-17, arc-brief candidate):** wasm cooperative scheduler pid-counter collision — `WasmScheduler::spawn_owned` mints from `next_pid` while native spawns mint from `shared_next_pid`, both starting at 1 and never reconciled, so mixing `vm.spawn` and `vm.spawn_actor` can mint the same pid twice and the second insert silently clobbers the first process. Discovered by WPORT-4's cast wall (its dead-pid target clobbered the actor at pid 1). Production scheduler change, out of WPORT-4 scope, NOT fixed. The native threaded scheduler is unaffected (single shared monotonic mint — certified 2026-07-17 for haematite's TTL lane, which cites the native scheduler only). **SUPERSEDED (2026-07-17, at the WPORT-5 land):** the WPORT-5 build incidentally deleted the private counter (`spawn_in`/`spawn_in_owned` allocate via the shared `alloc_pid` at `1ec8ec9`), so the arc-brief candidate is REDUCED to a collision-pin regression test riding the next wasm brief as a line item — Waffles' ruling at the WPORT-5 build tear; see the WPORT-5 status block.
**Supersession note (2026-07-17, tear Ruling 8):** the boundary evidence below is pinned at `d9de35e` — five lands stale — and is SUPERSEDED: `briefs/WPORT-4.json` re-verified the current state at `2cfd6cf` and found all six wake sources already arbitered by WPORT-2/WPORT-3. The historical pins are kept unrewritten (this document never performed a re-verification; believed state is not citable state cuts both ways) — cite the brief, not the lines below, for current state. The brief's `depends_on` adds WPORT-3, superseding the dependency line above (tear-ratified). Arc-board line (tear Ruling 7): latent trapped-exit gap recorded — bytecode exits perform no link propagation, unreachable today because cooperative bytecode receives no link/spawn facility; the guarding wall belongs to a future bytecode-linking brief.

**Scope.** Route all six frozen wake sources through the WPORT-2 arbiter: `send_message`, `cast`, async-NIF/Promise completion, receive-timer fire, native completions, and trapped exits. This brief also fills Apollo Biscuit's socket v0.2 (upgraded from v0.1 by the 2026-07-14 primitive correction): the browser event-delivery seam must use one event vocabulary with the native `connection_events.rs` hub, with snapshot-at-subscribe as the precedent for initial state. The fill inherits socket v0.2's two readiness bounds — idle-VM resume is WPORT-2's arbiter, and runtime construction is WPORT-1 — which Apollo's R11 blocks on by name. The socket governs event delivery; it does not expand this arc into native membership or distribution work.

**Boundary evidence.** Host mailbox sends wake through the cooperative edge (`crates/beamr/src/scheduler/wasm.rs:315-360` at `d9de35e`), while the exported `send_message` and `cast` methods do not request a scheduler callback (`crates/beamr-wasm/src/lib.rs:120-129`; `crates/beamr-wasm/src/lib.rs:269-280` at `d9de35e`). Promise completion calls `complete_async` without requesting a host turn (`crates/beamr-wasm/src/lib.rs:558-581`; `crates/beamr/src/scheduler/wasm.rs:176-195` at `d9de35e`). Receive-timer fire changes scheduler state without continuing execution (`crates/beamr-wasm/src/lib.rs:458-480`; `crates/beamr/src/scheduler/wasm.rs:158-174` at `d9de35e`). Native async completion envelopes and trapped linked-process exits also feed cooperative wake paths (`crates/beamr/src/scheduler/wasm_native.rs:209-248`; `crates/beamr/src/scheduler/wasm_native.rs:475-507` at `d9de35e`).

**Acceptance shape.** Starting from a truly idle VM, each of the six sources independently causes execution without any external pump call. For each source, a single event queues one turn; a burst before that turn remains coalesced; the durable message/completion/exit is not lost. Subscription tests receive an initial snapshot and then events expressed in the shared vocabulary. Tests fail if any source changes Rust state but leaves the process ready and unexecuted.

**Named socket/gate filled.** **Apollo Biscuit socket v0.2 — browser event-delivery seam**, sharing one event vocabulary with the native `connection_events.rs` hub and using snapshot-at-subscribe as the initial-state precedent; the v0.2 readiness bounds (WPORT-1, WPORT-2) travel with the fill.

### WPORT-5 — Browser BIF/service profile

**Size:** L  
**Dependencies:** WPORT-1 and WPORT-2.

**Status (2026-07-17):** CLOSED — landed on `main` at `1ec8ec9` (chain: brief `e58e1ef` → tear fold `6f1b51b` → six build commits; Waffles the Terrible ff-landed on his own green battery at the final head). The 197-row sealed profile (`docs/design/beamr/BROWSER-BIF-PROFILE.md`), all seven R2 wiring items (the bytecode `Pid ! Msg` silent drop is dead; cooperative IO sink with console default; zeros→badarg; structured JS refusal reasons), and the CI extension to exactly 33 pinned Node names landed together. **Build-tear flag rulings (Waffles, folded here on his word):** (1) `spawn/4` refusal RATIFIED — `remote_spawn_impl` destructures a node argument at the bytes, so the OQ8 ruling's "/3,4" inherited the outline's arity error; the worker's conservative refusal was correct obedience to the ruling's intent: plain LOCAL MFA spawn only, nothing that invents an unruled remote-spawn seam. (2) `apply_common` non-wiring RATIFIED — it takes only `&ModuleRegistry` in shared interpreter code; wiring services there would break the threaded byte-identity law, which outranks acceptance-text breadth; the export-fun dispatch consult is wired and doubly fail-first-proven. (3) The pid-collision boarded candidate is REDUCED to a collision-pin regression test riding the next wasm brief as a line item (see the WPORT-4 board-finding supersession note): the fix is real — the private counter is deleted and `alloc_pid` on the shared mint is the sole allocator at `1ec8ec9` — but unpinned; the wall proves native and cooperative spawns mint distinct pids.

**Scope.** Define and test a truthful browser contract for registered BIFs versus facilities actually injected into cooperative execution. Supported functions receive working cooperative services. Unsupported functions return stable, explicit errors. Registration must not imply support, and absent services must not degrade into silent no-ops or accidental misbehaviour.

**Boundary evidence.** The wrapper registers gate 1, gate 2, exception, ETF, and a large stdlib stub table (`crates/beamr-wasm/src/lib.rs:779-788` at `d9de35e`). Gate registration includes broad process and service-dependent surfaces (`crates/beamr/src/native/bifs.rs:47-82`; `crates/beamr/src/native/process_bifs/mod.rs:19-58` at `d9de35e`), but cooperative bytecode `NativeServices` currently supplies only the atom table and optional wasm async-NIF facility (`crates/beamr/src/scheduler/wasm.rs:610-615` at `d9de35e`). The wider service model includes timers, local send, spawn, supervision, links, process information, group leader, code management, ETS, IO-message handling, system information, and registry facilities (`crates/beamr/src/interpreter/mod.rs:46-132` at `d9de35e`). Some registered stdlib entries are explicit approximations or no-ops; for example, `sys:debug_options/1` accepts its input and returns `[]` (`crates/beamr/src/native/stdlib_stubs/misc_bifs.rs:154-163` at `d9de35e`).

**Supersession note (2026-07-17, WPORT-5 fold):** the boundary-evidence claim above that cooperative bytecode `NativeServices` "currently supplies only the atom table and optional wasm async-NIF facility" is superseded since WPORT-3 landed at `a17be58`: at `2cfd6cf`, `WasmScheduler::native_services` supplies THREE services — the atom table, the wasm async-NIF facility (installed by the browser wrapper), and WPORT-3's unified deadline-service timers (`crates/beamr/src/scheduler/wasm.rs:720-731` at `2cfd6cf`). The `d9de35e` pins above are retained as frozen evidence; the WPORT-5 brief re-verifies the service-injection truth table at its build head.

**Acceptance shape.** A profile enumerates every registered browser BIF as supported, supported-with-defined-approximation, or unsupported. Tests execute each supported facility class and assert exact stable errors for unsupported classes. The wall fails on silent success, dropped side effects, environment-dependent behaviour, or a registered function whose required service remains absent and undocumented.

**Named socket/gate filled.** None named by the frozen outline; the profile is a contract consumed by WPORT-8 and WPORT-9.

### WPORT-6 — Browser artifact loader

**Size:** M  
**Dependencies:** WPORT-2.

**Scope.** Add runtime fetch/manifest orchestration over the existing `load_module(bytes)` primitive. The loader orders dependencies, reports fetch and decoding failures, and preserves unresolved-import reporting. This is runtime browser loading, not another build-time inline bundle path.

**Boundary evidence.** `load_module(bytes)` accepts caller-supplied `.beam` bytes, inserts them into the module registry, and returns module and unresolved-import information (`crates/beamr-wasm/src/lib.rs:101-118` at `d9de35e`). The build script can package a known module set into `modules.bin`, `manifest.json`, bootstrap JS, and a Node packaging script (`crates/beamr-wasm/build.rs:19-49`; `crates/beamr-wasm/build.rs:52-72` at `d9de35e`); generated bootstrap base64-decodes embedded modules and calls `vm.load_module` (`crates/beamr-wasm/build.rs:148-173`; `crates/beamr-wasm/build.rs:227-275` at `d9de35e`). **INFERRED:** these cited routes provide caller-buffer loading and build-time preload, not a packaged runtime artifact fetcher.

**Acceptance shape.** A browser test starts with a manifest URL, fetches real artifacts at runtime, loads dependencies before dependants, and executes a module through the real VM. Missing artifacts, malformed bytes, cycles or unsatisfied ordering, and unresolved imports produce structured failures naming the affected artifact/module. No progress depends on recurring checks; fetch completion re-enters through the event-driven scheduling contract.

**Named socket/gate filled.** None named by the frozen outline.

### WPORT-7 — Output and failure surface

**Size:** M  
**Dependencies:** WPORT-2.

**Scope.** Provide an intentional browser output sink and a complete observable failure surface: route `io:*` output, define the treatment of `erlang:display/1`, expose process execution errors, surface scheduler/pump failures, and install a browser panic hook. The addendum's corrected classification is binding: current output is mixed, not uniformly inert.

**Scope ruling (2026-07-17, WPORT-5 fold — Tom + Waffles, joint authority per the WPORT-4 Ruling 5 precedent):** WPORT-5 absorbs the browser output sink, the `erlang:display/1` treatment, and process-execution-error exposure (its R2 items 4 and 7). WPORT-7's REMAINDER: the scheduler/pump failure surface, the browser panic hook, and ordered-output guarantees.

**Boundary evidence.** **VERIFIED:** `io:put_chars`, `io:format`, and related IO BIFs route through `ProcessContext::write_to_io_sink` (`crates/beamr/src/native/stdlib_stubs/io_bifs.rs:11-33`; `crates/beamr/src/native/stdlib_stubs/io_bifs.rs:116-119` at `d9de35e`), and the non-thread implementation is a no-op (`crates/beamr/src/native/context/mod.rs:1475-1485` at `d9de35e`). **VERIFIED:** `erlang:display/1` bypasses that sink and calls Rust `println!` directly (`crates/beamr/src/native/bifs.rs:202-211` at `d9de35e`). **INFERRED:** neither is a deliberate browser-console contract: `io:*` drops output while `display/1` relies on unstructured wasm stdout. Scheduler summaries identify errored PIDs, but stored `ExecError` values have no public accessor (`crates/beamr/src/scheduler/wasm.rs:44-57`; `crates/beamr/src/scheduler/wasm.rs:68-75`; `crates/beamr/src/scheduler/wasm.rs:479-530`; `crates/beamr-wasm/src/lib.rs:813-821` at `d9de35e`). The rAF pump swallows `pump_turn` errors and only marks itself stopped (`crates/beamr-wasm/src/lib.rs:349-360` at `d9de35e`).

**Acceptance shape.** Browser tests capture ordered `io:*` and `display/1` output through an explicit sink rather than incidental stdout. A crashing process exposes its PID and structured `ExecError`; a scheduler-turn failure rejects or invokes a documented error channel; a panic reaches the configured panic surface. The wall fails if output disappears, if `display/1` bypasses the contract, if summaries expose only an errored PID, or if a pump/turn simply stops without an observable cause.

**Named socket/gate filled.** None named by the frozen outline.

### WPORT-8 — Async capability adapters

**Size:** L  
**Dependencies:** WPORT-4 and WPORT-5.

**Scope.** Implement the selected browser fetch/storage host NIFs as explicit asynchronous capabilities. Completions re-enter through WPORT-4's event-delivery seam and conform to WPORT-5's supported-service profile. This brief does not emulate TCP, a native filesystem, or dirty pools and must not acquire a threaded-runtime dependency.

**Boundary evidence.** The wrapper already exposes synchronous, Promise-like, and async-NIF host registration, including the wasm async dispatch trampoline (`crates/beamr-wasm/src/lib.rs:131-152`; `crates/beamr-wasm/src/lib.rs:154-175`; `crates/beamr-wasm/src/lib.rs:753-771` at `d9de35e`). Promise completion already converts and delivers results into cooperative scheduler state (`crates/beamr-wasm/src/lib.rs:558-581`; `crates/beamr/src/scheduler/wasm.rs:176-195` at `d9de35e`). Native distribution is `net`-gated and implemented with Tokio TCP (`crates/beamr/src/lib.rs:14-15`; `crates/beamr/src/distribution/connection.rs:1-20` at `d9de35e`). File BIF modules and registration are `fs`-gated, and file completion facilities are `threads`-gated (`crates/beamr/src/native/mod.rs:50-53`; `crates/beamr/src/native/bifs.rs:66-69`; `crates/beamr/src/native/mod.rs:103-106` at `d9de35e`). Dirty pools belong to the thread-only scheduler topology (`crates/beamr/src/scheduler/mod.rs:87-89`; `crates/beamr/src/scheduler/mod.rs:151-154` at `d9de35e`).

**Acceptance shape.** Each selected adapter proves success, host refusal, malformed response, and Promise rejection against a real browser host seam. Completion from true idle schedules exactly one runtime turn and delivers one durable result. Build and dependency checks prove the wasm closure remains free of `threads`, native `net`, native `fs`, and dirty-pool requirements. Tests fail if an adapter blocks, polls, silently maps errors to success, or presents itself as TCP/filesystem emulation.

**Named socket/gate filled.** No new named socket; this brief consumes Apollo Biscuit's WPORT-4 event-delivery seam.

### WPORT-9 — Browser conformance and NO-POLLING gate

**Size:** L  
**Dependencies:** WPORT-2 through WPORT-8.

**Scope.** Establish permanent browser/Worker conformance CI using the real generated wasm bundle and a real Beamr workload. Exercise construction, artifact loading, supported BIFs, output and failure paths, and every wake class. Make the NO-POLLING wall a permanent release gate: idle means no recurring callbacks.

**Boundary evidence.** The current workflow runs only `cargo check` and has no wasm runtime, packaging, browser, event-loop, timer, idle-CPU, or NO-POLLING test (`.github/workflows/cooperative-wasm.yml:13-20` at `d9de35e`). The crate has wasm-bindgen test dependencies and five Node-runnable tests (`crates/beamr-wasm/Cargo.toml:28-29`; `crates/beamr-wasm/src/lib.rs:894-1022`; `crates/beamr-wasm/src/convert.rs:340-404` at `d9de35e`), but current VM tests stop at the constructor failure. **VERIFIED addendum:** the edge-worker test replaces the bundle import with an in-memory JavaScript VM stub (`examples/edge-worker/test/worker.test.mjs:6-53` at `d9de35e`), and that stub implements its own `spawn`, `run_step`, `take_exit_result`, and synchronous `runUntilExit` loop (`examples/edge-worker/test/worker.test.mjs:8-48` at `d9de35e`). **INFERRED:** the present Miniflare test proves HTTP adaptation and an expected JS API shape, not wasm instantiation, Beamr bytecode execution, or cooperative scheduler behaviour.

**Binding law.** **NO-POLLING applies here as a permanent F-0d conformance gate. A polling scheduler is a TEAR CONDITION. Any timer whose job is “check whether something changed” is a design error.**

**Acceptance shape.** CI packages and instantiates `beamr_wasm_bg.wasm`, loads and executes real Beamr bytecode, and does not replace the VM with a JS stub. The suite independently proves mailbox send, cast, async-NIF/Promise completion, receive timeout, native deadline, native completion, and trapped-exit wakes; artifact fetching; supported and unsupported BIF behaviour; output; process error; pump/turn error; and panic surfacing. Callback instrumentation observes **zero recurring host callbacks while idle**, including while only future deadlines exist. The gate fails on rAF rechecks, intervals, repeating timeouts, synchronous bounded pump loops, or any ready process that waits for an external manual pump.

**Named socket/gate filled.** Permanent F-0d / NO-POLLING browser conformance gate.

## Dependency graph

```text
WPORT-1  baseline truthfulness
└── WPORT-2  event-loop scheduling core
    ├── WPORT-3  host deadline service
    ├── WPORT-4  event-source integration ──────┐
    ├── WPORT-5  browser BIF/service profile ───┴── WPORT-8  async capability adapters
    ├── WPORT-6  browser artifact loader
    └── WPORT-7  output + failure surface

WPORT-2 + WPORT-3 + WPORT-4 + WPORT-5 + WPORT-6 + WPORT-7 + WPORT-8
└── WPORT-9  browser conformance + permanent NO-POLLING gate
```

The frozen critical path is **WPORT-1 → WPORT-2**. WPORT-1 turns compilation into a truthful executable baseline; WPORT-2 settles the scheduling contract on which all browser-host integration depends. WPORT-3, WPORT-4, WPORT-6, and WPORT-7 may proceed after WPORT-2; WPORT-5 depends on WPORT-1 and WPORT-2; WPORT-8 joins WPORT-4 and WPORT-5; WPORT-9 joins WPORT-2 through WPORT-8.

## Non-goals

- **Distribution and native TCP.** Distribution is compiled only with `net`, and its current connection implementation uses Tokio TCP (`crates/beamr/src/lib.rs:14-15`; `crates/beamr/src/distribution/connection.rs:1-20` at `d9de35e`). `beamr-wasm` does not select `net` (`crates/beamr-wasm/Cargo.toml:13-21` at `d9de35e`). Browser transport is a separate host problem, not part of this arc.
- **JIT/Cranelift.** `jit` selects the Cranelift dependencies and structurally implies `threads` (`crates/beamr/Cargo.toml:14-20`; `crates/beamr/Cargo.toml:79` at `d9de35e`). The JIT module is `jit`-gated, and the wrapper selects neither `jit` nor `threads` (`crates/beamr/src/lib.rs:26-27`; `crates/beamr-wasm/Cargo.toml:13-21` at `d9de35e`). JIT is not on the browser critical path.
- **Native file BIFs.** File and metadata BIF modules and their registration are `fs`-gated, while completion facilities are `threads`-gated (`crates/beamr/src/native/mod.rs:50-53`; `crates/beamr/src/native/bifs.rs:66-69`; `crates/beamr/src/native/mod.rs:103-106` at `d9de35e`). The wrapper enables neither `fs` nor `threads` (`crates/beamr-wasm/Cargo.toml:13-21` at `d9de35e`). Browser storage belongs behind the explicit async host capabilities in WPORT-8, not filesystem emulation.
- **Membership event source.** The frozen membership-event-source ask remains native-only and is not part of this arc. The related native scheduler topology for connection lifecycle and distribution services is within the `threads`-gated scheduler surface (`crates/beamr/src/scheduler/mod.rs:85-150` at `d9de35e`). WPORT-4 fills only the named Apollo Biscuit v0.2 browser event-delivery socket and does not port native membership machinery.
- **Browser↔server sync carrier (SOCKET-CARRIER-v0.2) — externally filled, owner named.** Ruled outside this arc by the reviewer of record (2026-07-14): haematite-wasm's branch sync runs in the storage owner-worker, not inside the beamr VM, so an ordered-frames/backpressure/connection-identity carrier (browser WebSocket in that worker plus a native server endpoint) carries no VM dependency and must not be welded into this port. Owner: Apollo Biscuit, as a SYNC-CARRIER brief in the haematite-wasm arc, with no beamr types on either side of its carrier trait. The framing format, reconnect/backoff policy, and server-end placement are that brief's residue, not this arc's.

  **Banked future amendment (tear condition T1) — WPORT-10, browser carrier adapter (beamr half).** Recorded verbatim as the named path for VM-RESIDENT components wanting direct carrier access; explicitly not built in v1, because no v1 requirement consumes it. The shape: the JS host owns the socket (WebSocket-shaped or otherwise — beamr gains no new native dependency; the wasm closure stays free of `net`/`threads`); beamr's surface is ordered frame ingress delivered into a process mailbox through the WPORT-2 arbiter (frames are TOLD, arrival wakes the VM — NO-POLLING clean), egress via the existing async-NIF seam where completion doubles as the backpressure credit (a send resolves when the host accepts the frame — no ready-polling), and connection identity plus fate expressed in WPORT-4's event vocabulary (open/down/reconnect ride the same hub semantics, snapshot-at-subscribe for join state). Dependencies WPORT-2 and WPORT-4; size M. Opening it requires a consumer requirement and the reviewer of record's word.

## Open questions for the tear

1. **Which host-turn primitive implements WPORT-2: microtask, macrotask, or an explicit hybrid?** The boundary permits a coalesced microtask/macrotask but the inputs do not decide starvation and fairness policy. Decide with a browser prototype measuring long runnable bursts, Promise ordering, rendering opportunity, and the explicit fairness-yield result while asserting the F-0d idle wall.
2. **What is the exact Apollo Biscuit v0.2 event schema and subscription API?** The frozen socket requires one vocabulary with `connection_events.rs` and snapshot-at-subscribe, but the evidence pack does not contain that interface. Decide with the socket v0.2 specification plus cited native hub API/tests showing event names, payloads, ordering, subscription lifetime, and snapshot semantics.
3. **Which BIFs are in the first supported browser profile?** The current registry is broader than injected services (`crates/beamr/src/native/bifs.rs:47-82`; `crates/beamr/src/scheduler/wasm.rs:610-615` at `d9de35e`), so source inspection alone cannot choose product support. Decide after WPORT-1 with a generated registry-to-facility inventory and executable workload traces from the first browser applications.
4. **Does WPORT-6 standardise the existing generated manifest or introduce a versioned runtime manifest?** The build creates `manifest.json` and inline bootstrap assets (`crates/beamr-wasm/build.rs:19-49`; `crates/beamr-wasm/build.rs:52-72`; `crates/beamr-wasm/build.rs:148-173` at `d9de35e`), but the inputs do not establish that format as a runtime contract. Decide with the generated schema, its current consumers, representative dependency graphs, and failure cases for versioning, integrity, MIME, and unresolved imports.
5. **Which fetch and storage operations constitute WPORT-8's selected first capability set?** The async-NIF/Promise seam exists (`crates/beamr-wasm/src/lib.rs:154-175`; `crates/beamr-wasm/src/lib.rs:558-581` at `d9de35e`), but the frozen boundary intentionally does not choose the product API. Decide with named target workloads, least-authority capability requirements, browser compatibility evidence, cancellation semantics, and the WPORT-5 support-profile decision.
