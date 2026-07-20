# The hosting ladder — design of record (destination)

- **Revision:** r1.1 — domain-owner verdict GREEN (Artemis Peach,
  2026-07-20, every ground-truth row byte-verified at her own hands in both
  repos); her amendment and both fold-size notes folded at the tear, plus
  her #13 consume-or-supersede ruling in §5. Recorded for Tom's veto, never
  queued. Originally r1, written at the coordination seat (Waffles,
  2026-07-20) from a vision conversation between Tom and the seat, under the
  standing keep-it-moving authority.
- **Kind:** DESTINATION document, tagged *design*, not briefed work. It
  records where beamr is going as a **generic host runtime** — the ladder of
  things beamr can host, the honest guarantee at each rung, what exists at
  the bytes today versus what is planned, the sequencing, and the named
  tensions — so the vision cannot drift and later arcs brief against it.
- **What this document does NOT do:** open arcs, allocate work, or specify
  implementations. Each rung briefs separately when its time comes, at the
  coordination seat, with Tom's veto standing.
- **Evidence base:** beamr `main` @ `c7073a3`; frame repo `main` @ `672a6c5`
  (`/Users/tom/Developer/ablative/frame`). Every EXISTS claim below was
  verified at those bytes; evidence paths are cited inline and in §0.

## The ladder at a glance

Beamr today is a BEAM runtime. The destination is a **generic host**: one
supervision tree, one mailbox discipline, one capability model — applied to
progressively more foreign guests. Each rung wraps a different kind of
executable thing in the same process identity (pid, mailbox, links,
supervision), and each rung states its own guarantee honestly rather than
borrowing the gold standard from rung 1.

| Rung | Guest | Status | Guarantee class |
|---|---|---|---|
| 1 | BEAM actors (Gleam/Erlang bytecode) | **EXISTS** | Gold standard: preemptive, isolated heaps, hot-loadable |
| 2 | Wasm module instances as processes | **PLANNED** | Memory-isolated by construction; preemption via epoch interruption; capability = import table |
| 3 | OS processes as supervised actors | **PLANNED** (cheapest to pull forward) | Crash isolation via the real OS boundary; mailbox bridged over stdio/bus |
| 4 | Remote peers (browsers, machines) over the bus | **PARTIALLY EXISTS** (via frame/liminal) | Same tree, longest wire; durable identity still open (frame D4) |
| 5 | The appliance profile (AOT, tree-shaken, one binary) | **PLANNED** (farthest) | Static single-binary deployment; dynamism becomes a granted capability |

## 0. Ground truth — what exists versus what is destination

Truth table, verified at the bytes (beamr @ `c7073a3`, frame @ `672a6c5`).
Beamr paths are repo-relative; frame paths are prefixed `frame:`.

| Piece | Status today | Evidence |
|---|---|---|
| BEAM bytecode interpreter + preemptive reduction-based scheduling | **Landed.** | `crates/beamr/src/interpreter/`, `crates/beamr/src/scheduler/mod.rs` (per-slice `reductions_consumed` accounting) |
| Per-process heaps and GC | **Landed.** | `crates/beamr/src/process/heap.rs`, `crates/beamr/src/process/gc.rs` |
| Mailboxes with selective receive | **Landed.** | `crates/beamr/src/mailbox/mod.rs`, `crates/beamr/src/mailbox/selective.rs` |
| Links, monitors, exit-signal trapping (the substrate supervision trees execute on) | **Landed.** | `crates/beamr/src/supervision/link.rs` (`trap_exit` semantics), `crates/beamr/src/supervision/monitor.rs` |
| Hot code loading with `on_load` and purge | **Landed.** | `crates/beamr/src/scheduler/module_management.rs` (`hot_load_module`, `PurgeResult`) |
| Cranelift JIT | **Landed** behind the `jit` feature. | `crates/beamr/Cargo.toml:16-20,79` (cranelift-codegen/frontend/jit/module/native), `crates/beamr/src/jit/` |
| Dirty pools (CPU and IO) | **Landed, integration partial.** `DirtyPool` with CPU/IO defaults exists; the spawn-path integration is scaffolded, per the in-tree comment. | `crates/beamr/src/scheduler/dirty.rs` (`DirtyPool`, CPU/IO constructors); caveat at `crates/beamr/src/scheduler/spawning.rs:315` ("dirty pool integration is scaffolded") |
| AOT | **Precursor only — NOT object emission.** The current AOT bundle (`BEAMR_AOT` v1) is a host-target-validated *cache envelope* around the original BEAM bytes plus the identities of functions that compiled; native pointers are not persisted, and loading recompiles through the demand-JIT path. Rung 5's Cranelift *object emission* does not exist. | `crates/beamr/src/jit/aot.rs` module doc (states exactly this), `crates/beamr/src/jit/aot_format.rs` |
| Embedded module archive (load modules baked into the binary) | **Landed.** | `crates/beamr/src/scheduler/module_management.rs` (`load_embedded_module`) |
| Capability system (audit + sandbox) | **Landed.** | `crates/beamr/src/capability/audit.rs`, `crates/beamr/src/capability/sandbox.rs` |
| **Wasm-module hosting in beamr** | **Does not exist. None anywhere in the repo.** `beamr-wasm` is beamr *compiled to* wasm for the browser; its `load_module` loads **BEAM** modules via the BEAM loader. No wasm engine (wasmtime or otherwise) appears in any manifest. | `crates/beamr-wasm/src/lib.rs:184` (`load_module` → `load_module_with_origin`, the BEAM chunk loader); repo-wide grep for `wasmtime` in all `Cargo.toml`s: zero hits |
| Erlang-port-style supervised OS processes | **Does not exist.** `std::process` appears only in FFI helpers and tests (e.g. a one-off `Command::new("sh")`), never as a monitored, restartable, mailbox-bridged port facility. | grep `std::process|open_port` across `crates/beamr/src`; `crates/beamr/src/native/meridian_ffi.rs:137` |
| Browser as a direct bus participant | **Landed in frame.** The page opens the liminal WebSocket itself; frame-host carries no feed bytes. | `frame:docs/design/fragments/DESIGN.md` §0 truth table; `frame:crates/frame-host/src/server.rs` module doc |
| Durable remote-peer identity / reconnect | **Open.** Self-minted per-pageload identity, empty auth tokens, no reconnect — register finding D4, riding the lease-authority arc. | `frame:docs/COHERENCE-REGISTER.md` §D4 (line 49) |
| Single-binary embedding precedent | **Landed in frame.** The scaffold's host template embeds component bytecode via `include_bytes!` out of `OUT_DIR` (build.rs compiles the Gleam); frame-host is one binary embedding a liminal server. | `frame:crates/frame-cli/templates/host-lib.rs:19-20`, `frame:crates/frame-cli/templates/build.rs`; `frame:crates/frame-host/Cargo.toml` (`liminal-server` dep; crate description: "embedded liminal component") |
| The wasm-capability meeting point | **Named on the frame side.** frame-capability declares itself enforcer-agnostic: "Native and future WASM enforcers share these values." | `frame:crates/frame-capability/src/lib.rs:3-4` |
| Class-(b) fragment mount (rung 2's first consumer) | **Named exclusion, designed seam.** Fragments v1 excludes `stage-frame`/class-(b) mounts; they arrive as "a v2 payload addition once the module-loader shell exists." | `frame:docs/design/fragments/DESIGN.md` §1.3 ("Deliberately not in v1") and "Explicitly NOT in this design" |

Plain statement, so no reader can mistake the destination for the present:
**no wasm-module hosting exists in beamr today** (verified — the name
`beamr-wasm` means beamr *running in* wasm, not beamr *hosting* wasm), **no
OS-port facility exists**, and **no object-emitting AOT exists**. Rungs 2, 3,
and 5 are destination. Rung 1 is bytes. Rung 4 is bytes on the frame/liminal
side with the identity half open.

---

## 1. Rung 1 — BEAM actors (EXISTS)

The floor of the ladder and the standard everything above it is measured
against. Beamr executes BEAM bytecode with:

- **Preemptive scheduling** — reduction-budgeted slices; no guest can starve
  the scheduler (`crates/beamr/src/scheduler/`).
- **Per-process isolation** — separate heaps, per-process GC
  (`crates/beamr/src/process/`).
- **Supervision** — links, monitors, exit-signal propagation and trapping;
  OTP supervision trees run as library code on this substrate
  (`crates/beamr/src/supervision/`).
- **Mailboxes** with selective receive (`crates/beamr/src/mailbox/`).
- **Hot code loading** — `hot_load_module` with `on_load` hooks and purge
  (`crates/beamr/src/scheduler/module_management.rs`).
- **Cranelift JIT** — demand compilation behind the `jit` feature
  (`crates/beamr/src/jit/`).
- **Dirty pools** for long-running native work (`scheduler/dirty.rs`; spawn
  integration still scaffolded — see §0).

**The guarantee (gold standard):** a crashing process cannot corrupt a
sibling; a looping process cannot starve one; a message send costs a heap
copy into the receiver's mailbox; code can change under a running system.
Every later rung states its delta from this, honestly, per §6.

## 2. Rung 2 — wasm processes (PLANNED)

A hosted wasm module instance wrapped as a beamr process: it gets a pid, a
mailbox, links, and supervision **identical to a Gleam process**. From the
tree's point of view there is no difference — you link to it, monitor it,
restart it under policy.

The design pins four properties:

1. **Its capability grant IS its import table.** A wasm instance can only
   call what its import table names. No grant → the function does not exist
   in its world — not "denied at call time," *absent*. This is the
   capability model beamr already carries (`crates/beamr/src/capability/`)
   expressed structurally rather than checked procedurally.
2. **Memory isolation by construction.** Linear memory is the sandbox; the
   guest cannot address host memory. This is the one place a rung-2 guest
   matches rung 1's isolation without effort.
3. **Preemption restored via wasmtime-style epoch interruption — on the
   server engine only.** Wasm has no reduction counter; epoch interruption
   gives the server-side scheduler back its right to preempt. Preemption
   granularity is epoch-quantized, not reduction-fine — stated per §6(c),
   never papered over. The browser face does NOT get this property: its
   preemption class is stated separately in Browser symmetry below.
4. **Wasm calls scheduled with dirty-CPU discipline.** A wasm export call is
   a native-length operation from the scheduler's view; it runs under the
   dirty-pool rules (`scheduler/dirty.rs`), keeping the normal schedulers
   honest. This property inherits the §0 caveat: the dirty pools' spawn-path
   integration is itself still scaffolded (`scheduler/spawning.rs:315`), so
   the rung-2 brief depends on that rung-1 maintenance item completing — it
   must not treat dirty discipline as landed.

Four build pieces when this rung briefs: the **embedded engine** (wasmtime),
the **process wrapper** (mailbox pump bridged to exports/imports — one ABI),
the **scheduling discipline** (epoch + dirty-CPU), and the
**capability-checked import table**.

**Browser symmetry.** On the browser side, beamr-wasm hosts modules the same
way, so the page's module loader and the server host are **the same
concept** — one ABI, two engines (wasmtime on the server, the browser's own
wasm runtime on the page). Today beamr-wasm loads only BEAM modules
(§0); this rung extends both sides together. **Symmetry of concept is not
symmetry of guarantee** (§6(c)): the browser engine has no epoch
interruption — a running export on the page cannot be preempted by the
host; control returns only when the call returns. The browser face's
preemption class is therefore **run-to-completion per export call —
cooperative always, Worker isolation at best** — and rung-2 briefs state
the two faces' guarantees separately, never as one number. The browser
face's mailbox pump rides the landed WPORT-2 arbiter contract —
edge-triggered, no recurring pump — so rung-2 briefs inherit the NO-POLLING
law from that contract rather than rediscovering it.

**First consumer:** frame's class-(b) fragment mount. Fragments v1 excludes
`stage-frame`/class-(b) modules (GraphMother, Iridium) as a named v2 seam
behind "the module-loader shell" (`frame:docs/design/fragments/DESIGN.md`
§1.3). frame-capability already names "future WASM enforcers"
(`frame:crates/frame-capability/src/lib.rs:3-4`). **The two designs meet at
this ABI:** the class-(b) loader shell is the browser face of the rung-2
process wrapper, and frame-capability's enforcer-agnostic verdicts are what
the import table enforces.

## 3. Rung 3 — OS processes as supervised actors (PLANNED, cheapest to pull forward)

The Erlang **port** concept: beamr spawns an external OS process — any
language, any binary — monitors it, restarts it per supervision policy, and
bridges a mailbox to it over stdio or the bus. The port gets a pid; the tree
treats it like any child.

**Why the guarantee is real:** crash isolation comes from the OS process
boundary, which actually exists — a segfaulting guest takes out its own
process, the host observes the exit, supervision policy runs. Nothing is
simulated.

**Why it is cheapest:** no engine to embed, no ABI to design — process
spawning, exit watching, and stdio piping are commodity operations. The
engineering is strictly simpler than rung 2. It is sequenced after rung 2
only because nothing needs it yet; it **lands whenever first needed and may
pull forward** without disturbing the ladder.

Today nothing of it exists (§0): no port facility, no monitored external
process anywhere in the tree.

## 4. Rung 4 — remote peers (PARTIALLY EXISTS via frame/liminal)

Browser pages and remote machines as **linked participants over the bus** —
same supervision tree, longest wire. A remote peer is a process whose
mailbox transport is the liminal bus instead of shared memory.

**What is landed:** the browser as a *direct* bus participant. The frame
page opens the liminal WebSocket itself; frame-host serves bytes but carries
no feed traffic (`frame:docs/design/fragments/DESIGN.md` §0, ruled at frame
D3 — the browser is a liminal *participant*, not a proxied client).

**What is open:** durable identity and reconnect. Today's peer identity is
self-minted per pageload with empty auth tokens and no reconnect story —
frame coherence-register finding **D4**, which rides the lease-authority arc
(`frame:docs/COHERENCE-REGISTER.md` §D4). Rung 4's identity work is **that**
work; this ladder adds no second design for it.

## 5. Rung 5 — the appliance profile (PLANNED, farthest)

The far rung: a deployment profile that produces **one binary**.

- **AOT compilation via Cranelift object emission.** Today's `BEAMR_AOT`
  bundle is a cache envelope that recompiles on load (§0,
  `crates/beamr/src/jit/aot.rs`); the appliance profile emits real object
  code and links it.
- **Tree-shaking as reachable-closure walking from the boot set.** Walk the
  call graph from the boot modules; drop unused modules, unused BIFs, and
  unreferenced runtime subsystems.
- **Static-link the runtime core + compiled code + embedded assets** into a
  single artifact.

**Precedent in-tree, both halves already proven small:**

- Beamr's scheduler already loads modules from an **embedded archive**
  (`load_embedded_module`, `crates/beamr/src/scheduler/module_management.rs`).
- Frame's scaffold **embeds component bytecode via build.rs +
  `include_bytes!`** (`frame:crates/frame-cli/templates/host-lib.rs:19-20`),
  and **frame-host is one binary embedding a liminal server**
  (`frame:crates/frame-host/Cargo.toml`). The appliance profile generalizes
  what those already do by hand.

The tension between AOT and hot loading is a law, not a footnote — §6(b).

**Relation to the prior AOT arc (task #13, ruled by the domain owner at her
seat, 2026-07-20):** the ladder **consumes** that arc's A1 (JIT wire-up) and
A2 (coverage) — they are demand-JIT completeness work on the rung-1
maintenance track that rung 5 depends on, dispatchable on Tom's word
independent of ladder timing — and **supersedes** its A3 (the standalone AOT
design document): this section plus the eventual rung-5 brief ARE that
document. Task #13 is re-scoped accordingly at the domain owner's seat.

---

## 6. The honest boundaries (laws)

These are stated as laws of the ladder. Any future brief that contradicts
one of them is wrong until this document is revised at the seat.

**(a) Native code loaded in-process never gets true isolation.** A segfault
in in-process native code kills the host — every guarantee on every rung
dies with it. Beamr **refuses to pretend** otherwise: there is no rung where
"trust me" native code runs inside the VM with a claimed sandbox. Native
code that needs crash isolation runs at **rung 3**, behind a process
boundary that the operating system actually enforces. (In-process native
BIFs/NIFs remain what they are today: trusted runtime extensions, not
guests.)

**(b) Full AOT and hot code loading are in tension.** A fully static binary
cannot hot-swap what was compiled and linked into it. The resolution is the
**hybrid appliance profile**: AOT the core; keep the loader compiled in for
exactly those rungs granted runtime dynamism; **the capability system
decides what may change at runtime** — dynamism becomes a grant like any
other, absent unless named. The dev profile stays fully dynamic. **Same
code, two build profiles** — never two codebases.

**(c) Per-rung costs and preemption quality differ and are stated per rung,
never averaged.** A rung-1 send is a heap copy; a rung-2 boundary crossing
serializes through one ABI; a rung-3 message transits stdio or the bus; a
rung-4 message crosses a network. Preemption is reduction-fine at rung 1,
epoch-quantized at rung 2, OS-scheduler-grade at rung 3, and cooperative at
best at rung 4. Documentation, benchmarks, and briefs speak about **a
specific rung's** numbers. A blended "beamr messaging costs X" claim is
banned by this law.

## 7. Sequencing (pinned)

1. **Fragments arc** — in flight, frame repo.
2. **Class-(b) stage mount** — the designed seam (fragments v2 payload
   addition; `frame:docs/design/fragments/DESIGN.md`).
3. **Rung 2** — wasm host + capability ABI, as the companion to frame's
   browser-peer arc; the loader shell and the server host land against one
   ABI.
4. **Rung 3** — whenever first needed; explicitly allowed to pull forward
   ahead of rung 2 if a consumer appears first.
5. **Rung 5** — last.

Rung 4's identity work is not a step in this list: it **rides frame's
D4/lease-authority design** and lands on that arc's schedule.

This document opens no arcs. Each rung briefs separately when its time
comes, at the coordination seat, with Tom's veto standing.

**Sequencing ratified (2026-07-20, Tom, via coordination seat):** the
pinned order above is the schedule of record — rung 2 briefs when frame
reaches its class-(b) loader seam (the ABI co-design partner). This is on
the docket, not shelved: the dirty-pool spawn-path completion (§2's named
rung-1 dependency, `spawning.rs`) opened the same day at the beamr seat as
independent maintenance. The server-only first-leg alternative (wasmtime +
wrapper + scheduling + import tables against a synthetic consumer,
deferring the browser face and joint ABI) remains available on Tom's word
if priority shifts ahead of frame's seam.

## 8. Strategic context (brief)

This ladder is the named path toward the longer-term sightings, nothing
more: Meridian's agents and publishers running as supervised beamr processes
is the appliance step (rungs 3 and 5 doing their ordinary jobs), and beamr
as a general host OS is the far comet, reached — if it is reached — by
climbing these rungs in order, each one earning its guarantee before the
next borrows it. No rung exists to serve the comet; each must justify itself
to its first consumer.

## Explicitly NOT in this design (seams named)

- **The rung-2 ABI itself** — the mailbox-pump/export-import contract is the
  first artifact of the rung-2 brief, designed jointly with frame's
  class-(b) loader shell. This document names the meeting point, not the
  signatures.
- **Wasmtime configuration, fuel-vs-epoch specifics, engine versioning** —
  rung-2 brief.
- **Port wire protocol (stdio framing vs bus bridging), restart policies for
  external processes** — rung-3 brief.
- **Durable peer identity, reconnect auth, lease authority** — frame
  register D4's arc; rung 4 consumes its outcome.
- **The tree-shaker's reachability analysis and the AOT object format** —
  rung-5 brief; today's `BEAMR_AOT` cache envelope is not that format and is
  not extended by this document.
- **Dirty-pool spawn-path completion** — an existing rung-1 gap
  (`scheduler/spawning.rs:315`), owned by beamr's ordinary maintenance
  track, not by this ladder.
- **Any Meridian-side integration** — the strategic section names the
  direction; no Meridian work is designed or implied here.
