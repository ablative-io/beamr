# Pre-registered predictions — #87 discriminator arms

Written 2026-08-14, AFTER the shipped-arm v2 baseline (87-arm-shipped-v2.log)
and BEFORE running either discriminator arm. Beamr tree at c524550 +
uncommitted instrument (tests/continue_reentry_latency.rs).

Shipped-arm v2 facts these predictions build on:
- No gap in ANY topology landed in the 3-9ms bands: no IDLE_PARK_TIMEOUT
  (5ms) signature anywhere.
- Sustained slow gaps == (co-residents ahead on the victim's queue) x
  SPIN_SLICE (2ms observed: pair-glue 2ms; saturated v1 5x2=10.2ms;
  saturated v2 1x2=2.0ms — placement lottery frozen by saturation).
- Separation events are rare victim-steals in the brief 2-entry window;
  once separated, permanently fast (~15us).

## Arm S — slice-cost scaling control (BEAMR_87_SPIN_MS=7, shipped bytes)

7ms chosen because it is NOT the 5ms park timeout: the two mechanisms
predict different numbers.

- QUEUE-ARITHMETIC mechanism (mine): slow gaps move to multiples of ~7ms
  (7 / 14 / 21...). Saturated p50 lands on k x 7ms for whatever k the
  placement lottery yields; pair/trio/episodic slow gaps land ~7ms.
- PARK-TIMEOUT mechanism (the #106 cross-feed candidate): slow gaps stay
  clustered at ~5ms / ~10ms regardless of spin cost.
- alone: unchanged us floor (no spinners involved).

## Arm M — notify_all added to the Requeue push (execution/core.rs:83)

The mutation models the sleeper-aware-notify fix candidate (parked at
Waffles+Vesper signatures). Committed-first rail: instrument commit lands
before the mutation is applied; restore is checkout-safe and sha-verified.

Predictions at SPIN default 2ms, all five measurements:
- alone: unchanged (no siblings to notify).
- pair: unchanged us bulk (separation may happen marginally sooner; the
  glued-pair 2ms alternation itself is UNTOUCHED by notify because the
  waiting queue holds ONE entry and a 1-entry queue is never stolen).
- trio: the leading 2ms transient run SHORTENS (the second spinner's
  rescue no longer waits for a timed wake, and each requeue push fires a
  wake during 2-entry windows, so victim-steal separation comes sooner)
  but does NOT vanish: gaps behind the glued co-resident remain ~2ms.
- saturated: UNCHANGED (nobody parks; notify_all with no sleepers is a
  no-op on latency; placement stays frozen).
- episodic: per-episode max stays ~2ms class (the glued-pair cost);
  fraction of slow gaps may drop somewhat via earlier separation.

HEADLINE PREDICTION: the mutation does NOT collapse the ~2ms/slice-sum
class anywhere, i.e. the notify gap is NOT the mechanism of the relayed
8.5-10.6ms figure; that figure is co-resident slice-sum queue time.

If instead pair/episodic slow gaps collapse to us under M, the notify gap
IS load-bearing and the #106 cross-feed stands.

## Named axes

"Slow gap" = victim re-entry gap >= 1ms. Bands as printed by the
instrument histogram. Placement facts read from the instrument's own
spinner-placement and parks-during-sampling lines, never assumed.
