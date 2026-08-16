# B-144 R1 — the fork, measured. Probe applied, measured, REVERTED.

Ground and pre-registration: `PREREG.md`, written and staged before any edit.
Tree at time of writing: clean except that pre-registration file. **No fix is
applied. Nothing is landed.**

## PRE-REGISTERED EXPECTATIONS — RESULT

| # | expectation | result |
|---|---|---|
| 1 | R1's named wasm32 command → rc 0, tally 0 | ✅ **rc 0, zero errors** |
| 2 | F4 → rc 0 by identity with (1) | ✅ **rc 0, zero errors** — identity claim HELD |
| 3 | leg 9 `nostd-ratchet` fires, direction **UP** | ✅ **fired, UP: 1039 → 1075 (+36)** |
| 4 | canon axes unchanged | not yet run — battery deliberately held, see below |
| 5 | falsifier: R1 green *without* leg 9 moving ⇒ mechanism wrong | not triggered; leg 9 moved as predicted |

Control: the **shipped** browser configuration
`cargo check -p beamr --no-default-features --features cooperative,json`
is **rc 0 before and after**. Nothing a consumer builds is affected.

## THE FIX THAT WORKS — 4 edits, not 20

1. `lib.rs:51` — drop `#[cfg(any(threads, cooperative))]` from `pub mod replay;`
2. `lib.rs:60` — drop the same from `pub mod timer;`
3. `error.rs:447` — drop the same from `impl From<replay::ReplayMismatch> for ExecError`
4. `Cargo.toml` — `crossbeam-queue` becomes a hard dependency (both features already pulled it)

`timer.rs` names **no optional dependency at all**. `replay/`'s only one is `zstd`
in `replay::file`, which is **already** gated on `net`+`fs`. Neither module's gate
was required by its dependencies.

### ⚠️ THE "THREE ROOT CAUSES" WAS INCOMPLETE — THERE IS A FOURTH

Clearing the first layer revealed 6× `E0277: ExecError: From<ReplayMismatch> is
not satisfied` from a **separately gated `From` impl** at `error.rs:447` — invisible
while the import errors masked it. 20 → 7 after two edits, then 7 → 0 after two more.

⭐ **ERRORS REVEAL IN LAYERS; AN ERROR COUNT IS A LOWER BOUND ON THE WORK, NEVER A
DESCRIPTION OF IT.** "20 errors, three root causes" was a fair reading of the
compiler's output and still under-described the job by one whole root cause.
Fortunately in the cheap direction — but the same reasoning that made 20 look like
4 edits could as easily have made it 40.

## ⛔ THE FORK, NOW WITH A NUMBER ON IT — AND IT IS NOT MINE TO SETTLE

| | **Fix A (measured above)** | **Fix B — gate the 20 reference sites** |
|---|---|---|
| edits | **4** | many, cascading |
| R1 + F4 | **green** | green |
| shipped config | **unaffected** | unaffected |
| leg 9 ratchet | **1039 → 1075 (+36)** | unchanged or lower |
| obeys R1's *spec* — *"**gate** std-dependent code behind feature flags"* | **✗ — it un-gates** | ✓ |

**Fix A passes R1's named command while contradicting R1's own spec.** That is this
arc's signature trap in a new costume, and I am not resolving it by preferring the
cheap side on my own word.

### What the +36 actually is — attributed at a COMPLETE-POPULATION instrument

| code | base | probe | delta |
|---|---|---|---|
| **E0432** unresolved import | 29 | 17 | **−12** ← the R1 fix working |
| E0425 cannot find value | 420 | 451 | +31 |
| uncoded | 178 | 192 | +14 |
| E0433 cannot find item | 412 | 415 | +3 |
| | **1039** | **1075** | **+36** |

Net +36 = **−12 genuine fixes + 48 newly-visible errors**, and those 48 carry the
same E0425/E0433 signature as the existing no_std debt — i.e. ordinary `std::`
paths in `timer.rs`/`replay/`, code a `#[cfg]` gate had been **excluding from the
measurement**.

⭐ **READING: the 48 are REVEALED, not CREATED.** The 1039 was never the true size
of the no_std debt; part of it sat behind cfg gates that kept it out of the count.
Same class as F1 — a measurement that did not mean what it appeared to mean because
a gate excluded things from it.

⛔ **BUT THAT READING IS AN INFERENCE FROM ERROR-CODE SIGNATURE, NOT A PER-ERROR
PROOF**, and it is a conveniently flattering one, so it is labelled as inference.
It does not by itself authorise moving a canon gate's ceiling.

### ⚠️ HOW I NEARLY GOT THE ATTRIBUTION WRONG

My first attribution instrument was a per-file histogram built from `^  --> `
lines. It found 311 errors against a rustc tally of 1039 — **it was measuring 30%
of the population** (the `-->` gutter indent varies with line-number width), and it
would have let me report a per-file story over a biased 30% subset. Caught by
comparing the instrument's own total against rustc's tally. The table above uses a
one-line-per-error regex whose totals reconcile **exactly** to 1039 and 1075.
⭐ **A HISTOGRAM THAT DOES NOT SUM TO THE KNOWN TOTAL IS NOT A COMPOSITION.**

## WHY THE BATTERY IS NOT RUN, AND WHY THAT IS NOT CAUTION

Leg 9 is **strict in both directions by design** — I wrote it that way so the
no-std number could not move silently. Under Fix A it fires, and the lane cannot
go green without re-pinning `CEILING` 1039 → 1075 **in the same commit**.

That re-pin is a governance act on a canon gate, not part of the declared workload
class ("make R1's 20 errors zero"). The two are inseparable — the lane cannot land
without it — so the whole lane goes back for the word rather than the ceiling alone.

A battery run before the fix shape is settled would be spent twice if the answer is
Fix B. Holding it is the cheaper order of operations, not a hesitation.

## WHAT IS ASKED

1. **Fix A or Fix B**, given: A is 4 edits and moves the ratchet +36 against R1's
   own spec; B honours the spec and is materially larger and unsized.
2. If **A**: confirm the ceiling re-pin 1039 → 1075, which I will not make on my
   own word.
3. If **B**: it needs sizing before it is a lane; the cascade is unmeasured.

Also carried, independent of the fork: **F4 is 20, not 21** — the same
`grep -c '^error'` off-by-one the ratchet's counting rule exists to prevent,
sitting in the document that states the rule. Corrected in `PREREG.md`.
