# Ablative Stack — Integration Topology

*As of July 2026. How the layers actually consume each other, and what that
implies about which APIs are load-bearing.*

## The layers

| Project | Version | Stage | License |
|---|---|---|---|
| beamr | 0.12.0 | Published, 1500+ tests | Apache-2.0 |
| haematite | 0.4.0 | Published; active (cluster/members substrate, resharding) | Apache-2.0 |
| liminal | 0.2.2 (`liminal-rs`) | Published; frontier = aion worker/push seam | AGPL-3.0 |
| aion | 0.8.0 | Published; most feature-rich app layer (17 crates) | AGPL-3.0 |
| norn | 0.1.0 | Substantial (~150K LOC, ~2,600 tests), in production | AGPL-3.0 |
| frame | 0.1.0-dev | Scaffold (two commits); docs are the substance | Apache-2.0 |

## beamr is consumed at two distinct tiers

**Tier 1 — high-level actor/process surface** (liminal, haematite,
frame-planned): `Actor`/`ActorContext`/`spawn_actor`/`spawn_actor_cooperative`/
`CallFuture`, `Scheduler`/`WasmScheduler`, `NativeHandler` processes,
`ExitReason`, `ModuleRegistry`, and the distribution surface
(`ConnectionManager`, `pg::PgRegistry`, resolver, control-frame codecs).
These consumers want the BEAM as a concurrency runtime: processes,
supervision, links, distributed process groups, and the wasm scheduler.
**liminal is the reference consumer** and the most exposed to breakage here.
Notable detail from the liminal deep-dive: it also hand-assembles bytecode
modules and uses `enqueue_atom_message` + shared queues — so the *module
assembly* surface (`Module`, `Instruction`, `ResolvedImportTarget`) is
load-bearing beyond aion.

**Tier 2 — low-level VM embedding** (aion only): `term::*` (incl.
`BinaryRef`, boxed accessors), `native::{ProcessContext, NativeFn,
NativeEntry}`, `atom::*`, `loader::*` (`prepare_module`, `lambda_unique_id`,
`Instruction`, `Operand`), `constant_pool`, `Module`/`ResolvedImport`, dirty
scheduler kinds, and `spawn_link_closure` — which aion drove into beamr
0.12.0 for its in-VM activity tier. **aion embeds and drives the VM** to
preload Gleam modules and expose Rust activities as NIFs. It depends on
internals, not the facade, and is the pace-setter for VM-embedding features.

**haematite** consumes beamr only through its optional wasm-runtime feature
(shard actors as native processes, especially in-browser); it is otherwise a
leaf. **norn** has zero dependency on any stack crate — aion drives it as a
`norn --protocol jsonrpc` subprocess through the `AgentHarness` trait in
`aion-integration-norn`. That inversion is deliberate and healthy: the agent
runtime is insulated from VM churn, and aion owns the only seam.

## Version skew (live concern)

- aion → beamr **0.12.0**
- liminal, haematite → beamr **0.11.0**
- frame → **path deps** on beamr/haematite/liminal (the only project not
  pinning crates.io versions)
- liminal → haematite 0.4.0; aion → haematite 0.4.0 + liminal 0.2.2
  (optional `liminal-transport`)

Anything that composes the stack (frame; aion's liminal transport) needs the
0.11↔0.12 actor/distribution surfaces compatible. **Recommendation**: a
cheap version-convergence pass moving liminal + haematite to beamr 0.12
before frame pins anything.

## Cross-cutting observations

1. **The actor facade held.** beamr's decision to re-export a stable actor
   surface at the crate root ("downstream depends on `beamr::`, not
   scheduler internals") is exactly why liminal/haematite survive beamr's
   internal churn. Protect that surface with semver discipline as 0.12+
   evolves.
2. **The distribution hook slot is contended.** beamr exposes a single
   connection-down hook, already owned by pg-purge; liminal's cluster
   membership therefore polls at 250ms. If more consumers need node-down
   signals (frame will), beamr should grow a multi-subscriber hook — a
   small, high-leverage API change.
3. **Two schedulers per process is the norm today.** liminal-server runs a
   connection scheduler + channel-supervisor scheduler; every liminal
   subsystem defaults to spinning its own. Fine standalone; frame will want
   one shared scheduler per node, which liminal already supports
   (`with_distribution` / shared-supervisor constructors) but doesn't
   default to.
4. **The observability drain is the template for cross-layer seams**: a
   pinned channel name constant + a default-no-op trait method, additive and
   byte-identical for non-participants. Frame's component taps should copy
   this shape.
5. **Licensing boundary**: beamr/haematite/frame are Apache-2.0;
   liminal/aion/norn are AGPL-3.0. Worth a deliberate check when frame
   (Apache) links liminal/aion (AGPL) — distribution-of-combined-work
   questions should be settled before frame publishes.
