# Known intermittents

A registry, not a excuse list. An entry here does **not** license dismissing a
red: if a test in this file fails a gated run, that run is RED. The purpose is
the opposite — an intermittent that is only ever discussed in chat becomes
folklore, and folklore is what lets the next person wave a failure through with
"oh, that one's flaky."

Rules for this file:

* **An entry records observations, not a verdict.** State how many times it was
  seen, out of how many runs, and what failed to reproduce it.
* **State the attribution honestly, including "unattributed".** If the control
  that would assign blame is underpowered, say so and say by how much.
* **Record refuted hypotheses.** The next person should not re-walk them.
* **Once-seen-and-unattributed does not become "known flake" at the moment it is
  inconvenient.** Promotion to a diagnosed cause requires a mechanism, not
  repetition of the shrug.

---

## `distribution::connection_events_tests::snapshot_subscriber_during_churn_misses_nothing_and_double_sees_nothing`

Source: `crates/beamr/src/distribution/connection_events_tests.rs:1213`

**Status: SEEN ONCE. UNATTRIBUTED. Not reproduced in ~60 subsequent runs.**

### The observation

One failure in the `--lib` population on x86_64 Linux (rocketfish), during
pre-flight for the aion#85 + #26 battery. The tree was
`artemis/beamr-85-and-26` @ `32073ab`, which had passed the same suite minutes
earlier — the only intervening change was `cargo fmt` reflow of unrelated JIT
files. The test is not in, and does not depend on, any file changed by that work.

```
test result: FAILED. 1840 passed; 1 failed
```

### Reproduction attempts — all green

| attempt | runs | result |
|---|---|---|
| the test alone, own process | 6 | green |
| full `--lib` suite | 3 | green |
| two-arm control, combined tree | 20 | green |
| two-arm control, `origin/main` (control) | 20 | green |
| cold-vs-warm paired cycles | 8 pairs | green |

Roughly **1 red in 60 `--lib` runs**.

### Refuted hypothesis — do not re-walk it

*"It fires on the first run after a rebuild"* — which is what the single
observation was. **Refuted: 0/8 cold reds**, in a paired design where each
rebuild-then-run cycle carried its own warm run as an in-cycle control.

### Why the control does NOT exonerate the change under test

The 20-vs-20 main/combined control came back clean on both arms, and that is
**not** evidence of no effect. At the observed rate (~1/45 at the time it was
seen), the probability of a clean 20 under the null of no change is ~0.64. The
control is **underpowered and cannot attribute**. Resolving a rate of that order
with confidence needs on the order of 150 runs per arm.

Recorded because a clean-but-underpowered control reads exactly like an
exoneration in a summary, and is not one.

### Disposition

Handed over rather than chased further or quietly dropped. The shape — a
subscriber snapshot racing connection churn — is the same family as the
suite-conditional races already seen elsewhere in the estate, where a test is
green in isolation and red only under full-suite concurrency. If it is seen a
second time, that changes it from an anomaly to a pattern and it should be
instrumented with full per-test capture rather than re-run.

Observed 2026-08-19 by Artemis Peach, beamr seat.
