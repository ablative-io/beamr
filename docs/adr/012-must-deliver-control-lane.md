# ADR-012: Supervision Controls Ride a Must-Deliver Lane; Loss Converts to Connection-Down

**Status:** Accepted
**Date:** 2026-07-07

## Context

Cross-node supervision signals (LINK/UNLINK/EXIT/EXIT2) have a property
data messages do not: losing one silently violates the supervision
contract forever. A dropped data message is visible to the application
(it can retry); a dropped EXIT means a linked process never learns its
peer died — there is no retry, because the sender is dead.

The existing outbound path (`DistSender`) is a 1024-slot queue that
silently drops on overflow. That is an acceptable posture for data
(backpressure-by-loss, matching UDP-ish semantics embedders already
tolerate) but fatal for supervision traffic.

Three approaches were considered:

- **Share the data queue, make it lossless:** blocks scheduler workers
  on a slow peer — a wedged connection stalls unrelated local work,
  violating the no-blocking-in-workers rule (ADR-003 lineage).

- **Unbounded control queue:** no loss, but a wedged peer grows the
  queue without bound; memory pressure from a dead TCP session.

- **Bounded control lane; overflow tears the connection down:** controls
  get a dedicated 256-slot queue, drained with priority over data, each
  entry pinned to the connection generation it was enqueued against. If
  the lane overflows, the connection is marked down
  (`ConnectionDownReason::ControlOverflow`) — and the connection-down
  backstop delivers `noconnection` to every process holding a link over
  that node.

## Decision

The third: a bounded, generation-pinned, biased-drain control lane whose
overflow is converted into a connection-down event.

The load-bearing insight is that the backstop makes loss *equivalent to*
delivery of a coarser signal. The delivery contract (DC-1..DC-6 in
DIST-CONTROL-WIRE-SPEC §5) is "for every established link, exactly one
of {wire EXIT, `noconnection`}" — never zero, never two. A lane that
cannot write the precise signal downgrades the whole session to the
imprecise one, which is exactly what a real network partition would have
produced. No silent arm exists: every failure path (overflow, encode
failure, connection already down) lands in a state where the backstop
owns delivery.

Generation pinning (each queued control holds an `Arc` of the connection
it was enqueued against, not a node name resolved at drain time) keeps a
control from leaking onto a redialed session it logically predates.

## Consequences

- Scheduler workers never block on a slow peer; `link_remote` and exit
  propagation stay non-blocking.
- A peer slow enough to overflow 256 pending controls loses the session
  — deliberately. That is treated as the network partition it
  effectively is.
- Exactly-once depends on the connection-down backstop scanning process
  link state that is *current* — which forced the store-back merge fix
  (Executing-process link removals must survive checkout, 0.13.0) and
  the post-establish recheck on inbound LINK apply.
- The backstop is node-keyed, not generation-keyed; the residual
  cross-generation window is a recorded known limitation pending a
  design ruling on per-link session pinning.
- Remote monitors (BEAMR_MONITOR=102, reserved) will reuse the same lane
  and the same loss-converts-to-DOWN argument when they land.
