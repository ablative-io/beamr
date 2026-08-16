# #112 COLLECTION OBSERVABLE — RESULTS

Successor to the AR-1 fix lane's closing finding. Design note precedes code at
`docs/design/beamr/briefs/COLLECTION-OBSERVABLE-NOTE.md`.

## What was wrong

`Heap::total_capacity()` witnesses a heap **RESIZE**. Every AR-1 control was
reading it as a **COLLECTION**. The two come apart in both directions, and the
lane measured them coming apart before writing a line of the remedy.

## The observable

`gc_attempts` / `gc_completions`, two `u64` on `Process`, incremented inside
`minor::collect` and `major::collect` — **the implementations, not the public
wrappers**, so an uncounted collection is unrepresentable rather than merely
unlikely. A clone inherits both; a reset would produce a spurious zero for a
reason unrelated to collection, which is the resize proxy's failure again.

⭐ **TWO counters, and this is the load-bearing decision.** A completion-only
counter is silent in exactly the case that produced this lane — a collection
attempted and **FAILED** reads as zero delta and reports "nothing happened."
That is the proxy's own sin rebuilt one level up. `attempts > completions` is
the only way to see it.

## The discriminating pair

| test | what it pins |
|---|---|
| `arm_a_collection_counter_moves_on_a_collection` | it MOVES on a collection — minor and major, accumulating |
| `arm_b_collection_counter_does_not_move_on_a_pure_resize` | ⭐ it does NOT move on a resize |
| `a_fresh_process_has_collected_nothing` | the zero point is real |
| `a_collection_can_happen_with_capacity_unchanged` | the converse — counted with capacity flat |

**ARM B is the point.** It grows the heap directly and asserts capacity moved
while the counters did not — the exact false positive the old guard produced,
asserted as a negative, forever. A counter that incremented on *both* events
would pass ARM A perfectly and be precisely as useless as the proxy.

ARM B carries **its own precondition assert** that a resize really happened.
Without it the test would pass trivially on a heap that never grew — the
asleep-instrument shape this whole arc is about.

The last row exists so independence is pinned from **both** sides: ARM B shows
capacity can move while the counters do not; that row shows a collection is
counted with capacity unchanged. Neither direction is left for a future reader to
re-derive.

## ✅ Falsifiers — pre-registered in the design note BEFORE the code existed

Transcript: `gate-logs/112/falsifiers.log`.

| mutation | expected | measured |
|---|---|---|
| **M1** remove `note_gc_completion()` from `minor::collect` | arm_a RED | arm_a **FAILED**, arm_b ok ✅ |
| **M2** redefine `gc_attempts()` as `heap.total_capacity()` | arm_b RED | arm_b **FAILED** — and arm_a **also** failed |

⚠️ **M2 went further than I predicted, and the extra red is recorded rather than
trimmed to match.** I predicted arm_b only; arm_a fell too, because capacity does
not move during that collection so `attempts + 1` fails as well. A prediction
edited afterwards to fit its result is not a prediction.

Both mutations reverted, tree re-verified green at the bytes (32 passed).

## The battery

Pin `c918936`. Prediction committed **before** the runner, with the pin wrinkle
declared **up front** this time rather than reconciled afterwards.

| leg | predicted | measured | |
|---|---|---|---|
| 4 `wasm-tests` | 2 / 86 / 0 / 0 | 2 / 86 / 0 / 0 | ✅ unchanged |
| 5 `tests` | 76 / 2148 / 0 / 0 | 76 / 2148 / 0 / 0 | ✅ +4 |
| 8 `tests-all-features` | 76 / 2158 / 0 / 0 | 76 / 2158 / 0 / 0 | ✅ +4 |

8/8 rc 0 · `SCORED == DECLARED == 8` · marker **COMPLETE** · pin identical at
open and close · `--untracked-files=no` **EMPTY**.

The `+4` was derived at the bytes (`#[test]` in `gc/tests.rs` **26 → 30**, all
four named in advance), never off a diff — this repo's external diff pager makes
a `^+`-anchored grep silently return zero.

Prediction pin `f04773b` vs battery pin `c918936`: one commit, `--numstat` 61/0,
**zero Rust**.

## ⚠️ The census did NOT move, and that is not the reassurance it looks like

`tree pre: 11` → `tree post: 11`. **Nine files were written during this run.**
The count is flat because **git collapses an untracked DIRECTORY to a single
line** — `gate-logs/112/battery/` was already one entry at launch (the redirect
had created `BATTERY.log` in it) and stayed one entry with ten files in it.

Recorded because "census unchanged" reads as "nothing was written," and here it
means the opposite. The census that carries evidence is the **tracked** one,
which is empty.
