# #113 — falsifiers, pre-registered BEFORE they were run

**Waffles' rebind term, adopted verbatim into the row: each rebind carries its
own proof that the control still refuses, because a control that stops refusing
when re-pointed has been silently disarmed.**

Three controls were re-pointed from `Heap::total_capacity()` onto
`Process::gc_attempts()`:

| control | site |
|---|---|
| `assert_pressed`, called by `array_round_trip` (AR-1 site 13) | `crates/beamr-wasm/src/convert.rs` |
| `assert_pressed`, called by `object_round_trip` (AR-1 site 17) | same |
| the inline guard in `ar1_sites_13_17_nested_scopes_open_inside_one_another` | same |

## M0 — the measurement that came first, and what it settled

The rebound guard was run **unconditional** — demanded on EVERY cell, on both
arms, whatever the outcome. **Result: 86 passed / 0 failed.**

⭐ So every cell of all three controls really does collect, including the
`Refused` CONTROL cells that showed `466 -> 466` under the old proxy. That is the
mechanism the #112 note predicted, now confirmed from the other side:
`ensure_space` collects first and grows **only if the collection was not
enough** (`gc/mod.rs:154` then `:162`). The refusing cells collected and the
collection freed enough, so capacity never moved. **The cells were pressed the
whole time; the instrument could not see it.**

⛔ **Therefore the `if *outcome != Outcome::Clean { return; }` conditioning is
LIFTED, not carried forward.** Its only reason was the proxy mis-scoring the
Refused arm. That reason is now measured dead, and a weakening whose reason has
died is not kept "just in case" — it is exactly the shape that survives as an
unexplained hole. The rebound control is strictly stronger than the one it
replaces: it grades every cell, not only the clean ones.

## Predictions — written before either mutation was applied

### M1 — break the observable

Delete `process.note_gc_attempt();` from `minor::collect`
(`crates/beamr/src/gc/minor.rs`).

**Predicted: all three re-pointed controls go RED.** If any stays green, that
control is not actually reading the observable it claims to read.

⚠️ Named in advance: this assumes the wasm conversion path collects **minor**,
not major. If the majors are what run, M1 fires on fewer than three and the
right response is to record the miss and re-aim at `major::collect` — **not** to
call a partial red a pass.

### M2 — prove the guard can still tell pressed from unpressed

Shrink the site-13 array cell to `count = 1` — an input genuinely too small to
press anything.

**Predicted: the site-13 control goes RED** with the NO COLLECTION message.

This is the arm that matters. M1 only shows the guard notices a broken counter.
M2 shows the guard is **not vacuously true** — that a real conversion can fail to
collect, so demanding a collection is a live requirement rather than a fact of
any conversion whatsoever. Without M2 a future shrink of these inputs would sail
through a guard that always says yes.

⚠️ **If M2 comes back GREEN the retrofit is WEAKER than it looks and I will say
so**: it would mean every conversion collects regardless of size, the guard
cannot discriminate, and the pressure question needs a different axis. That
outcome is recorded, not smoothed over.

## Non-claims

- This does not re-grade the AR-1 fixes themselves. Sites 13/17 are fixed and
  their verdicts stand on their own evidence. What changes is the **grade the
  control is capable of giving**.
- `gc_completions` is deliberately NOT the asserted axis — what breaks an
  unrooted carrier is a collection ENTERED and moving objects, and one that fails
  partway has still moved them. Completions is printed in the failure message so
  an attempted-and-failed collection reads off the failure directly.
