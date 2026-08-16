# A real collection observable on the process heap — design note

**Lane #112. Base `origin/main = 42ea92d`. Instrument-first: this note precedes
code because the change touches core heap.**

Successor to the AR-1 fix lane's closing finding. Not a feature — an instrument
repair.

---

## 1. The defect this exists to cure

Every AR-1 control in the fix lane asserts `after > before` on
`Heap::total_capacity()` under a message that says *"this cell applied no
collection pressure."*

⭐ **`total_capacity()` witnesses a heap RESIZE. It does not witness a
COLLECTION.** Those come apart precisely where it matters, and the lane measured
them coming apart: a pre-fix replica failed *without the heap ever resizing*, so
a correctly-failing control was scored as an unpressed cell.

The tell was **non-movement under a knob** — raising the input 40 → 60 produced
the identical `466 -> 466`. A threshold problem moves when you move the
threshold; this one did not, because the model was wrong, not the magnitude.

The controls were not wrong to want a witness. There simply was not one to have.

## 2. Ground, measured at `42ea92d`

Each of these was read at the bytes, not inferred.

| fact | where |
|---|---|
| `GcStats` exists and is returned by every collection | `gc/mod.rs:32`, `:101`, `:118` |
| ⭐ **but the production path DISCARDS it** — `Ok(_stats) => {}` | `gc/mod.rs:155` |
| **No cumulative collection counter exists anywhere** | measured by grep across `crates/beamr/src`; zero hits |
| A telemetry hook exists — but is `#[cfg(feature = "telemetry")]`, records a **duration**, and is enabled by no build except the all-features legs | `gc/mod.rs:107-114` |
| ⭐ **Collection and growth are ADJACENT in one function**: collect at `:154`, early-return at `:162`, grow at `:167` | `gc::ensure_space` |
| Every non-GC caller of `collect_minor` is a **test** (`.expect("minor GC succeeds")`) | 10 call sites, all `#[cfg(test)]` |
| `minor::collect` / `major::collect` are **`pub(crate)`** — the public wrappers are today's only path, but are **bypassable by construction** | `gc/minor.rs:19`, `gc/major.rs:19` |
| `impl Clone for Process` is an **exhaustive struct literal with no `..` fallback** | `process/mod.rs:210-261` |
| `reduction_counter: u32` / `logical_clock: u64` establish the plain-counter-on-`Process` precedent, with `const fn` accessors | `process/mod.rs:180`, `:972`, `:994` |

**The adjacency in `ensure_space` is the whole mechanism.** Collect, and if that
was not enough, grow. A caller downstream sees only the capacity, so it cannot
tell "collected, then grew" from "grew" from "tried to collect, failed, returned
before growing." The proxy was never going to work.

⭐ **The clone hazard is already structurally prevented, and I checked rather
than assumed it.** A hand-written field-by-field `Clone` normally means a new
field gets silently dropped. Here the literal names every field with no `..`, so
**adding a field to `Process` is a compile error until `Clone` names it.** No
guard needs to be invented; one needs to be *not broken*.

## 3. Shape of the observable

Two `u64` counters on `Process`, beside `logical_clock`:

```rust
gc_attempts: u64,     // incremented on ENTRY, before the collection runs
gc_completions: u64,  // incremented ONLY on Ok
```

### ⛔ Why two, and this is the load-bearing decision

A single "collections completed" counter would be **silent in exactly the case
that produced this lane** — a collection that is attempted and *fails*. The AR-1
control's arm failed that way: the body refused, and a completion-only counter
would have read zero delta and reported "no collection happened."

**That is the proxy's own sin, rebuilt one level up.** A replacement instrument
that is blind in the same case as the instrument it replaces is not a
replacement.

With both, the three states are distinguishable and none is silent:

| observed | meaning |
|---|---|
| `attempts` moved, `completions` moved | a collection ran and finished |
| `attempts` moved, `completions` did **not** | ⭐ a collection was attempted and **FAILED** — the case that was invisible |
| neither moved | no collection was attempted; if capacity moved anyway, that was a **pure resize** |

Monotonic, saturating, never reset. A clone inherits both, so a region bracketed
across a clone cannot read a spurious zero.

## 4. Where it increments

⛔ **Inside `minor::collect` and `major::collect` — the implementations, not the
public wrappers.**

The wrappers are the only path *today*, but they are `pub(crate)` and a future
in-crate caller could reach past them. Incrementing in the implementation makes
a collection that does not count **unrepresentable**, which is the habit this
codebase already has (#86, #94, #103) and is cheaper to get right now than to
re-derive after the first bypass.

## 5. Cost

- **Space:** 16 bytes per `Process`.
- **Time:** two `u64` increments per collection — against a collection that
  copies live objects word by word. Unmeasurable in that shadow.
- **Hot path:** none. `gc::alloc` tries the nursery bump first (`gc/mod.rs:137`)
  and only reaches a collection on failure, so this is the rare path **by
  construction**, not by hope.
- **API:** additive. Two `const fn` accessors matching the `reduction_counter`
  precedent. No existing signature changes.

## 6. ⭐ The discriminating test pair — the point of the lane

The counter is the easy half. **The pair is what pins the confusion so no future
probe can rebuild it**, and it is a deliverable, not garnish.

- **ARM A — it MOVES on a collection.** Force a real collection; assert
  `gc_completions` increments.
- **ARM B — it does NOT move on a resize.** ⭐ *The arm that matters.* Call
  `Heap::grow_to_next_capacity()` **directly**, with no collection anywhere:
  assert `total_capacity()` **moved** and both counters **did not**. This is the
  exact false-positive the old guard produced, now asserted as a negative
  forever.

Arm B is the reason a single-arm test would be worthless here: a counter that
incremented on *both* events would pass Arm A perfectly while being precisely as
useless as the proxy it replaces.

⚠️ **A third arm is WANTED and is NOT promised:** the attempted-and-failed case
(`attempts` moves, `completions` does not). Reaching it honestly needs a
collection that fails, which means a deliberately corrupt object graph. That is
`unsafe` and fragile to construct, and a fragile arm that silently stops
constructing its own precondition would be another asleep instrument. It is
named here as an open want rather than quietly dropped — **if it ships, it ships
with its own proof that it still fails when the counter is broken; if it does
not, this note says so.**

## 7. Falsifier, pre-registered before the code exists

Break the increment on purpose — delete the `gc_completions` bump — and **both
Arm A and the retrofit consumers must go RED.** A guard that has never been shown
to refuse is not a guard.

Arm B's falsifier is the mirror: make the counter increment on *resize* as well,
and Arm B must go red. Both directions, because a checker that always says the
same thing satisfies either arm alone.

## 8. What this note does NOT claim

- Not a public stability commitment; not telemetry; not a scheduler-visible
  policy input.
- **Does not retrofit the AR-1 controls.** That is a separate row (#113),
  deliberately built only after the counter proves itself — and each rebind will
  need its own proof that the control still refuses, because a control that stops
  refusing when re-pointed has been silently disarmed.
- Does not claim the wasm AR-1 arms were *wrong in their verdicts*. Sites 13/17
  are fixed and green on evidence that stands. What was wrong was the **grade the
  control could give**, not the fix it was grading.
