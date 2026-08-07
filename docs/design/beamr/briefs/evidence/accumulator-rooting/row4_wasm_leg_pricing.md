# AR-1 ROW 4 — the wasm leg, priced

Measured at `308b448aad19523889ac298133bab962905318ab` on 2026-08-07.
Artefacts in this folder: `row4_probe.rs.txt` (the probe source, verbatim),
`row4_red_at_parent.txt` (the run transcript, with hashes and load figures).

---

## The row's premise was wrong, and wrong in the expensive direction

Row 4 says the wasm leg is **"a build leg nobody has costed."** That is an
assertion about the estate that was never checked against the estate. The leg
was costed and landed before this lane existed:

| | |
|---|---|
| `wasm32-unknown-unknown` target | installed, and **pinned** in `rust-toolchain.toml` |
| `wasm-bindgen-test-runner` | on PATH at **0.2.123**, matching `Cargo.lock`'s `wasm-bindgen` 0.2.123 |
| `wasm-bindgen-test` | already a **dev-dependency** of `beamr-wasm` |
| node | v26.4.0 |
| the gate itself | **`gates.json:7`, named `wasm-tests`** — runner env var set, version check prepended |
| CI | `.github/workflows/cooperative-wasm.yml` |
| `target/wasm32-unknown-unknown/` | 610 MB already built |

**Setup cost: zero.** ⭐ **AN ABSENCE CLAIMED WITHOUT A SEARCH IS NOT A FINDING,
IT IS A DEFAULT.** Row 4 priced a leg it had not looked for.

## Run cost

Measured on a **contended** box — 10 cores, 1-min load 10.00–14.15 throughout.
Contention **inflates** wall-clock, so every figure below is an **upper bound**,
which is the safe direction for a pricing decision.

| leg | wall | result |
|---|---|---|
| `wasm32-check` (`gates.json:6`) | **7.79 s** | rc 0 |
| `wasm-tests` full (`gates.json:7`) | **28.19 s** | rc 0, **80 passed** |
| incremental single-test re-run | ~3.5 s (warm) / ~23 s (after a lib edit) | — |

Figures are **warm-cache**. A cold `target/` would be substantially larger and
has **not** been measured; do not quote one.

---

## ⛔ The part that actually needed pricing — and it is not the toolchain

**All 80 existing wasm tests are structurally incapable of failing on this
defect,** for **two independent reasons**. Either alone is sufficient, so fixing
one would not have helped.

### 1. No process ⇒ no collection, at any input size

Every test in `convert.rs` builds its context with `ProcessContext::new()`,
which sets `process: None` (`crates/beamr/src/native/context/mod.rs:514`).
`ensure_heap_space` (`:788`) early-returns `Ok(())` when there is no process, so
`gc::ensure_space` is **never reached**. No collection, no move, nothing to
catch. This is not a matter of the tests being too small — **no input size
whatsoever would make them red.**

### 2. Immediates ⇒ no allocation between the bind and the use

My first probe used an array of 400 **small integers** and went **green**. A
small integer is an *immediate*: `value_to_term` returns it without allocating,
so the accumulator `tail` is never live across an allocating call and the defect
never fires. The elements must be **strings** (the `alloc_binary` path) or nested
containers.

⭐ **THIS GREEN WAS MY OWN, AND IT LOOKED EXACTLY LIKE A PASS.** A test can name
the right function, hit the right line, allocate the right number of cells and
still never exercise the mechanism it claims to cover. That is why the probe now
carries a **positive control** asserting the heap actually grew — without it a
green cannot distinguish *"the accumulator survived a move"* from *"no move ever
happened."*

---

## Red-at-parent: DEMONSTRATED

Both probes root a real `Process` via `attach_process` and assert
`heap().total_capacity()` grew before checking anything else.

### Site 13 — `array_to_term`, carrier `tail` — **RED**

400 string elements. The positive control **passed** (the heap grew), and then:

```
panicked at crates/beamr-wasm/src/convert.rs:383:40:
converted JavaScript array is a proper list
```

The list is no longer a proper list after the collection. The accumulator was
not rooted across `value_to_term`'s allocation.

### Site 17 — `value_to_term`'s object arm, carrier `pairs` — **RED**

60 string entries:

```
panicked at crates/beamr-wasm/src/convert.rs:582:18:
object of 60 string entries converts: JsValue("failed to allocate map term")
```

This one fails **at the conversion**, not at a corruption check — so on its own
it is **not attributable** to the rooting defect; it could have been an allocator
limit. It was separated by a **two-armed control**:

| entries | collection? | conversion |
|---|---|---|
| **5** | **no** — control reported `heap never grew (466 -> 466)` | **SUCCEEDS** |
| **60** | yes | **FAILS** — `failed to allocate map term` |
| **200** | yes | **FAILS** — same message |

⭐ **THE FAILURE IS COLLECTION-DEPENDENT, AND THAT IS WHAT MAKES IT
ATTRIBUTABLE.** The size arm alone proved nothing; the *no-collection* arm is
what rules out an allocator limit. A single red would have been a claim; the
pair is a measurement.

`total_capacity()` starts at **466**, not 233 — `DEFAULT_HEAP_SIZE` is the
nursery and `DEFAULT_OLD_HEAP_SIZE` equals it.

### Sites 12 and 16 — `json_value_to_term` — **NOT PROBED**

Same file, same leg, same two carriers (`tail`, `pairs`), but reached through
`serde_json::Value` rather than `JsValue`. They are **not** demonstrated red and
must not be recorded as such. ⭐ **A SAMPLE OF TWO IS NOT A CLASS.**

---

## What this settles, and what it does not

**Settled.** The leg exists, runs, costs ~28 s, and **can produce a genuine
red-at-parent** for sites 13 and 17. Row 4's `UNPRICED` is retired and its
sign-blocking condition — *"price it, or the site's disposition is
`FIXED-UNVERIFIED`"* — is **discharged for 13 and 17**.

**Not settled.**

1. Sites **12** and **16** have no demonstrated red. Priced-by-analogy is not
   priced.
2. The probes are **banked, not landed**. They are red at HEAD, so landing them
   now would leave the branch broken; they land **with the fix**, and row 4's
   per-site commit granularity still applies.
3. The `wasm-tests` gate's **80 green tests certify nothing about this class**
   and did not before this lane. That is a standing property of the suite, not a
   finding against the fix.

⭐ **A GATE THAT RUNS, PASSES, AND IS WIRED INTO CI CAN STILL BE BLIND TO THE
ENTIRE DEFECT CLASS IT APPEARS TO COVER.** The wasm leg was never the missing
piece. The missing piece was a context with a process attached — and nothing in
the row, the gate, or the ledger had noticed that the difference existed.
