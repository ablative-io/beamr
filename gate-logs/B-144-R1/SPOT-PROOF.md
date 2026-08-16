# Term 3 — buying the "revealed, not created" reading with samples

Ruled by Waffles at seq=42-ack: *"pick three of the newly-visible errors and show
each one's code path existed behind the gate in the pre-fix tree. You flagged the
inference as conveniently flattering yourself — so buy it with three samples. Any
sample fails, STOP and report."*

**No sample failed.** The proof came out stronger than three samples, for a
structural reason given below.

## FIRST — A CORRECTION TO THE DECOMPOSITION I ALREADY SENT

My seq=42 reported the +36 as **"−12 real fixes plus 48 newly-visible errors."**
That was the E0432-only subset generalised. Measured with a per-file instrument
that reconciles exactly (1039 and 1075, 100% coverage):

| | |
|---|---|
| **GROSS UP** | **+56** |
| **GROSS DOWN** | **−20** |
| net | **+36** |

The −12 was E0432 alone; other codes resolved too, taking the real down-move to
−20 and the up-move to +56. Same net, wrong parts. Corrected here and in the
message that follows.

## WHERE THE +56 LIVE — 100%, NOT MOSTLY

| file | delta |
|---|---|
| `crates/beamr/src/replay/driver.rs` | **+29** |
| `crates/beamr/src/replay/debugger.rs` | **+16** |
| `crates/beamr/src/timer.rs` | **+9** |
| `crates/beamr/src/replay/recorder.rs` | **+2** |
| | **+56** |

Every newly-visible error is inside `timer.rs` or `replay/` — the two modules
whose gates come off. Nothing appeared anywhere else. The −20 is spread across the
11 files that referenced them, which is the R1 fix working.

## THE THREE SAMPLES

Pre-fix tree = `5e3ee81`, working tree clean, no fix applied.

| # | error site | error | line at `5e3ee81` |
|---|---|---|---|
| 1 | `replay/debugger.rs:3:5` | E0433 cannot find module or crate `std` | `use std::collections::BTreeMap;` |
| 2 | `replay/driver.rs:4:5` | E0433 cannot find module or crate `std` | `use std::sync::{Arc, Mutex};` |
| 3 | `timer.rs:8:5` | E0433 cannot find module or crate `std` | `use std::collections::HashMap;` |

Each line is recovered from the **committed git object** at the pre-fix pin
(`git show 5e3ee81:<file>`), not from the working tree.

## WHY THIS IS A PROOF AND NOT A SAMPLE

The fix touches exactly three files — `lib.rs`, `error.rs`, `Cargo.toml`. It
touches **none** of the four files the +56 live in, so all four are byte-identical
before and after. The sampled lines are not merely *similar* pre-fix; they are the
same bytes.

And the exclusion is witnessed from both sides at the pre-fix pin:

- **12** pre-fix errors name `crate::timer` / `crate::replay` as **nonexistent** —
  the modules were not in the compile unit.
- **ZERO** pre-fix errors are located *inside* `timer.rs` or `replay/` — because
  the compiler never looked at them.

That second number is the load-bearing one. A gate did not merely *fail to fix*
this code; it kept the code out of the measurement entirely, so the 1039 was never
a count of beamr's no_std debt — it was a count of the part of the debt the gates
let the compiler see.

## VERDICT

**"Revealed, not created" is now PROVEN, not inferred.** The std dependencies
existed, in those bytes, at that commit, and a `#[cfg]` gate excluded them from
every measurement the ratchet has ever taken.

⭐ **A CFG GATE IS A HOLE IN EVERY MEASUREMENT TAKEN THROUGH IT.** The population a
compiler reports on is the population the cfg lets it compile — so a count taken
under one feature set is not a count of the codebase, and removing a gate can
raise a number without anything having got worse. Same family as F1, where a dev
edge silently re-enabled the features a flag excluded: both are measurements that
did not mean what they appeared to mean because something upstream chose the
population.

⇒ Term 2's ceiling re-pin 1039 → 1075 is recorded as a **POPULATION CHANGE**, not
drift, with this file as its evidence.
