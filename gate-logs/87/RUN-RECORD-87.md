# RUN RECORD — #87 Continue self-requeue re-entry latency, re-derived at beamr's bytes

Lane: #87 "8.5–10.6ms Continue self-requeue re-entry under contention vs
46–82µs real wake — size inherent-vs-defect, my call as owner."
Measured 2026-08-14 at beamr c524550 (+ instrument commit d38c926, local).

## Provenance of the relayed figure (Hermes Crumpet, DM 2026-08-14)

Measured in liminal's harness, NEVER at beamr's bytes: liminal branch
worktree-agent-adaf5ddccd511edb6 (base 4ed0562, tip 07942c5), real
liminal-server per iteration, beamr 0.16.3 registry bytes underneath.
"Re-entry" = host-monotonic gap between consecutive slices of the SAME
connection with NO wake-fire probe between — the Continue path by
elimination. 8.5–10.6ms was the arm-I slice-level band; per-boot spread
6–24ms. His thread-count 2x2 (4→16) was NOISE. History flag he supplied:
the finding this figure originally decorated (edge-triggered wake
starvation) was FALSIFIED in a controlled 2x2 (240+ boots); the surviving
residue is exactly the unmeasured-at-beamr question this lane closes.
Comparability verified: v0.16.3's Requeue arm is the same no-notify shape
as c524550 (`git show v0.16.3:…/core.rs` — `queue.push_with_priority` with
no notify_all).

## Instrument

`crates/beamr/tests/continue_reentry_latency.rs` (commit d38c926, sha
8df1fec6…), #[ignore]d measurement tests run by hand. Native-process probe:
`NativeOutcome::Continue` IS `SliceOutcome::Requeue`, so a handler that
timestamps each `handle()` entry measures the exact seam. Placement is
MEASURED per slice from `beamr-sched-N` thread names, never assumed from
spawn order; park counts sampled; spinner slice cost adjustable via
`BEAMR_87_SPIN_MS` (default 2ms). Five topologies: alone (floor), pair
(victim + 1 co-resident spinner), trio (+2), saturated (victim + 9 on 4
workers), episodic (40 fresh schedulers, collision per episode — the
burst-shaped workload's recurring case).

## Measured mechanism (shipped bytes, logs 87-arm-shipped*.log)

1. **Re-entry latency = sum of co-resident slices ahead in the FIFO lane.**
   Floor 15µs (alone). Pair/trio/episodic slow gaps ≈ 1 spinner slice;
   saturated ≈ k × slice where k = co-residents ahead.
2. **The single-entry no-steal rule glues collisions.** While a spinner
   runs, the victim waits ALONE on the queue (1 entry) — unstealable by
   design (steal.rs), so no wake rule can rescue that wait. Separation
   requires a steal landing in the brief 2-entry window after a requeue
   push; under shipped bytes (no notify on Requeue) that needs a 5ms timed
   wake to hit a µs-wide window — observed to take tens of alternations
   (trio: slow gaps at indexes 0–26 consecutive, then one victim-steal and
   µs forever). Separation, once it happens, is permanent.
3. **Saturation freezes placement.** Only idle workers steal, so with all
   workers busy NOTHING rebalances: spinner placement lines show x0
   migrations across every saturated run, parks-during-sampling 0, and the
   victim's sustained cost is decided by the placement lottery at
   saturation onset — observed 5×2ms=10.2ms (v1), 1×2ms=2.0ms (v2), and
   0 (victim alone, arm-S/arm-M lotteries) for the SAME topology.
4. **No IDLE_PARK_TIMEOUT signature anywhere.** Across every shipped-arm
   topology, zero victim gaps landed in the 3–7ms bands; the slow class
   sits at slice multiples, not park-timeout multiples.

## Discriminator arms (predictions pre-registered in PREDICTION-87.md
BEFORE either arm ran)

**Arm S — slice-cost scaling (BEAMR_87_SPIN_MS=7, shipped bytes):**
CONFIRMED the queue-arithmetic mechanism. Slow gaps moved 2.0ms → ~7.1ms in
every topology (pair 15 consecutive at 7.1ms; trio 64 at 7.1ms; episodic
155 in the 7–9ms band, per-episode max p50 7.09ms). Nothing clustered at
5ms or 10ms. The park-timeout mechanism (gaps pinned near 5/10ms
regardless of slice cost) is REFUTED as the driver of the slow class.

**Arm M — notify_all added to the Requeue push (execution/core.rs:83),
committed-first rail, exact-match surgery asserted matched-once, restore
sha-equal 1625d40b…:** my HEADLINE prediction ("the mutation does NOT
collapse the slow class — the glued pair is notify-immune") was **REFUTED**;
the pre-registered alternative branch fired instead. Every slow gap
vanished from every topology: episodic per-episode max p50 2134µs → 55µs
(40 independent collisions per arm), pair/trio 100% <100µs. Mechanism of
my error: the notify fires AT the requeue push — the exact moment the
queue holds 2 entries and IS stealable — so every alternation cycle
carries an aimed wake inside the steal window, and separation completes
within ~one alternation instead of waiting for a timed wake to hit a
µs-wide window. Per the pre-registration: **the notify gap IS load-bearing
and the #106 cross-feed stands** — not via 2×5ms park arithmetic (refuted
by Arm S), but by governing how long collisions PERSIST.

Cost signal in the same log: pair parks-during-sampling 3 → 729 (bare
notify_all on the requeue hot path wakes every sleeper on every slice —
the idle-cost explosion #106's ruling anticipated). The viable fix remains
the sleeper-aware variant, and remains a Waffles+Vesper signature question
(SIGNED §3.8 constant semantics) — unchanged by this lane.

## Verdict (mine as owner)

Two-part, matching the two mechanisms measured:

- **Idle-capacity contention (Hermes's shape — siblings park): DEFECT-CLASS,
  not inherent.** The VALUE of a slow re-entry is co-resident slice-sum
  arithmetic; its PERSISTENCE across a burst is the no-notify Requeue push.
  With notify the same workload pays ~55µs. The fix is already parked at
  the signature-holders' desks with the cost half now quantified
  (729-vs-3 park storm says bare notify_all is NOT the shippable form).
- **True saturation (no idle workers): INHERENT under current design.** No
  wake rule helps (nobody is asleep); re-entry = k×slice with k frozen by
  the placement lottery until load drops. Busy-worker rebalancing would be
  a separate, larger design question — flagged, not proposed.

Reconciliation of the relayed figure: 8.5–10.6ms at liminal = the
slice-sum arithmetic of HIS co-resident actors' slice costs, held in place
for whole bursts by the notify gap. Both halves now measured at beamr's
own bytes; his 46–82µs real-wake band matches the woken-path pushes, which
DO notify (#106 coverage measurement).

## Evidence

- 87-arm-shipped.log (v1), 87-arm-shipped-v2.log (v2 baseline)
- 87-armS-spin7.log (scaling control), 87-armM-notify.log (mutation arm)
- PREDICTION-87.md (pre-registered before both discriminator arms)
- Instrument: commit d38c926 (unpushed; landing shape NOTE below)

Landing note: the instrument was born as 5 #[ignore]d tests — first
nonzero `ignored` axis in the battery if landed that way. Flagged to
Waffles rather than landed unilaterally; RULED 2026-08-14 (~13:46Z):
neither land-as-ignored nor local-only — an ignored axis that stays ZERO
is itself a wall (the next illegitimate #[ignore] cannot hide among
legitimate ones), and a local-only instrument dies with the checkout while
being the evidence's provenance. Instrument RE-HOMED as an explicit
non-test target: `crates/beamr/examples/continue_reentry_probe.rs`
(`cargo run --example continue_reentry_probe`), body identical modulo the
example scaffold (feature-gated main, mod wrap, clippy doc-indent fixes).
Post-move confirmation run (87-probe-rehomed-confirmation.log) reproduced
the shipped-arm distributions — episodic per-episode max p50 2131µs vs the
test-shape's 2134µs, an additional independent shipped-bytes replicate.
Battery shape unchanged: result-lines/passed/failed/ignored priors carry
(75/2115/0/0 and 75/2125/0/0), ignored stays 0. The d38c926 test-shape
commit remains in history as the provenance of the four measurement logs;
the tip carries the example shape only.

Cross-repo residue (Hermes, 2026-08-14): attribution narrowed at his seat
with a reader census (his era record now carries this mechanism verbatim;
Waffles' ledger row superseded; no liminal-main text carries the old
attribution). Banked downstream: when a sleeper-aware-notify release
reaches liminal via the haematite chain, liminal's starvation pins
re-measure the band at consume — a field-side confirmation instrument
nobody has to build.
