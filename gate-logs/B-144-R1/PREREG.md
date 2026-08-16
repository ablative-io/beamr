# B-144 R1 — pre-registration, written at `a4e2328` BEFORE any edit

GO from Waffles at seq=41-ack: "twenty errors, three root causes, the wasm32
command as the gate — pre-register the expected error count going to zero and
fold F4's 21-error std red in if it's genuinely the same shape at the bytes.
Small lane, own battery."

## GROUND, measured at `a4e2328` before touching anything

| command | rc | rustc tally |
|---|---|---|
| `cargo check --target wasm32-unknown-unknown -p beamr --no-default-features` (**R1's own criterion**) | 101 | **20** |
| `cargo check -p beamr --no-default-features --features std` (**F4**) | 101 | **20** |

### ⛔ ERRATUM CARRIED IN: F4 IS 20, NOT 21

`gate-logs/219/FINDINGS.md:83` and `B-144-EVIDENCE.md` both say F4 is **21**.
Measured here: rustc's own tally is **20**, and a naive `grep -c '^error'` on the
same log returns **21** — because the trailing `error: could not compile …`
summary line is itself an `error` line.

That is precisely the off-by-one the `nostd-ratchet` leg's counting rule was
written to prevent, and it was sitting in the same document that states the rule.

⭐ **LAW: A COUNTING RULE DOES NOT RETROACTIVELY FIX THE NUMBERS ALREADY ON THE
PAGE.** Writing the rule down is not applying it. Every figure already in the
document has to be re-measured under the new rule, or the document teaches the
rule and breaks it in the same breath. Both prior figures are corrected here.

### F4 IS NOT "THE SAME SHAPE" AS R1 — IT IS THE SAME FAILURE

Parsed from `--message-format=json` (parse-don't-match), comparing full
`(code, message, file:line:col)` triples:

```
FULL TRIPLES IDENTICAL: True     R1 = 20     F4 = 20
only in R1: 0        only in F4: 0
```

The two commands reach an identical error set at identical sites. Folding F4 in
is therefore free: **fixing R1 fixes F4 by identity, not by similarity.** No
separate F4 work exists.

### The 20, attributed by root cause

| root cause | count | codes |
|---|---|---|
| `crate::timer` | 12 | E0432 ×7, E0433 ×5 |
| `crate::replay` | 7 | E0432 ×5, E0433 ×2 |
| `crossbeam_queue` | 1 | E0432 |

⚠️ Correction to `B-144-EVIDENCE.md`'s table: its three rows (7 / 5 / 1) counted
the **E0432 imports only** and summed to 13, not 20. The seven E0433s were
counted in the prose but never attributed. The attribution above is the whole 20.

11 files: `interpreter/{mod,opcodes/{core,messaging,mod,native_call}}.rs`,
`mailbox/mod.rs`, `native/{bifs,context/mod,local_send,native_process}.rs`,
`scheduler/wasm.rs`.

## MECHANISM

`crates/beamr/src/lib.rs:51` and `:60` gate both modules:

```rust
#[cfg(any(feature = "threads", feature = "cooperative"))]
pub mod replay;
#[cfg(any(feature = "threads", feature = "cooperative"))]
pub mod timer;
```

Under `--no-default-features` neither feature is on, so the modules do not
exist — while 20 sites in 11 ungated files still name them. `mailbox/mod.rs:10`
imports `crossbeam_queue::SegQueue` from an optional dep both features carry.

## ⛔ THE FORK THIS GO DID NOT ANTICIPATE — DECLARED BEFORE IT CAN BE RESOLVED IN MY FAVOUR

`timer.rs` imports **no optional dependency at all** (`std::collections::HashMap`,
`std::time::Duration`, `web_time::Instant`, `crate::term::Term`). `replay/`'s only
optional-dep use is `zstd` in `replay::file`, which is **already** gated on
`net`+`fs` at `replay/mod.rs:10`. So both gates are removable, and that is by far
the smaller edit.

| | **Fix A — ungate the modules** | **Fix B — gate the 20 reference sites** |
|---|---|---|
| edits | ~3 | many, cascading (gating an import exposes every *use* of it) |
| R1 command | green | green |
| host `no_std` count (leg 9) | **expected to RISE** — `timer`/`replay` enter the host no-default build, where `no_std` IS applied and their `std::` paths break | unchanged or lower |
| agrees with R1's *spec* | ✗ | ✓ |

R1's spec is *"Identify and **gate** std-dependent code behind feature flags."*
**Fix A passes R1's named command while contradicting R1's own spec** — it
un-gates std-dependent code and pushes it into the no-std build. That is this
arc's trap wearing a new costume: a green command is not a satisfied requirement.

**I am not choosing between these on my own word.** I will measure Fix A's exact
ratchet cost, because a measured number makes the fork decidable instead of
speculative, and route the choice with that number attached.

## PRE-REGISTERED EXPECTATIONS

1. **R1 gate** `cargo check --target wasm32-unknown-unknown -p beamr --no-default-features` → **rc 0, tally 0**. Waffles' named criterion.
2. **F4** `cargo check -p beamr --no-default-features --features std` → **rc 0**, by identity with (1). If (1) goes green and (2) does not, the identity claim above is REFUTED and this file is wrong.
3. **Leg 9 `nostd-ratchet` WILL FIRE under Fix A.** Direction predicted **UP** from 1039. Magnitude unknown — stated as unknown rather than guessed. The leg is strict in both directions by design; this is its first real encounter and it is *supposed* to stop this.
4. **Canon axes UNCHANGED.** No test is added or removed by either fix, so `result-lines / passed / failed / ignored` must come back exactly `76/2160/0/0` on leg 8 and `76/2150/0/0` on leg 5, `2/86/0/0` on leg 4. Any movement is unexplained and blocks the lane.
5. **Falsifier**: if the R1 tally goes to 0 *without* leg 9 moving under Fix A, my mechanism above is wrong — `timer`/`replay` would not actually be entering the host no-default build — and the fix must be re-derived before it lands.

## SCOPE HELD

- The **host 1039** is NOT this lane. The ratchet holds it; nobody is committed to zero.
- The **test-suite portability** finding (7 targets, 15 imports) is NOT this lane; it needs its own word.
- ⛔ **Phase 4 (THE SEAL) preempts the moment Cally answers §8.2/§8.4.** This lane is droppable mid-stride by construction.
