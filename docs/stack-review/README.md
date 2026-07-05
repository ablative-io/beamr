# Ablative Stack Review — July 2026

A written-up review of beamr and liminal, plus readiness feedback for
beginning frame development. Produced by Artemis Peach (Claude, Fable 5) from
a deep multi-agent exploration of both repos on 2026-07-05/06.

The series is split across the relevant repositories:

| Doc | Repo | Contents |
|---|---|---|
| [beamr-architecture.md](beamr-architecture.md) | beamr (here) | Subsystem map, execution core mechanics, developer invariants and gotchas |
| [beamr-gaps-and-sizing.md](beamr-gaps-and-sizing.md) | beamr (here) | The four incomplete areas (JIT/AOT, replay, distribution, capabilities): gaps and effort sizing |
| [stack-integration.md](stack-integration.md) | beamr (here) | How the stack layers consume beamr: two API tiers, version skew, cross-cutting observations |
| liminal-review.md | liminal `docs/stack-review/` | Liminal's runtime model, solid-vs-seam split, latent traps |
| frame-readiness.md | frame `docs/stack-review/` | Frame build-plan feedback and recommended sequence |

State inspected: beamr `main` @ 58987bb (v0.12.0), liminal v0.2.2, frame
v0.1.0-dev. Status claims (especially "built but unwired") go stale fast —
verify against git before acting on any specific gap.

## Headline findings

**beamr is architecturally complete and disciplined, with three subsystems
whose machinery is finished but missing a final wire**: the JIT pipeline is
built end-to-end but nothing in the production path records calls, so the
cache stays empty; replay consumption is fully wired but live recording
writes an empty event vec; distribution's handshake and pg groups are done
while remote link/monitor controls are buffered but never hit the wire.

**liminal's core is real and more sophisticated than its README suggests**,
but several advertised guarantees are seams: backpressure machinery exists
but is not wired into the publish path, server-side schema validation is
stubbed, the Gleam SDK's transport FFI does not exist, and SDK embedded mode
is a scaffold.

**frame can start now** — nothing blocks Phases 0–2 — but its plan carries a
stale premise (the beamr wasm-runtime port it lists as "designed, unbuilt"
has in fact landed) and Phase 3's backpressure acceptance criterion depends
on unlisted upstream liminal work.
