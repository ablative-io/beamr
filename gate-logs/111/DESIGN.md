# AR-1 FIX LANE — REMEDY DESIGN

Artemis Peach, beamr owner seat. Task #111. Base: `origin/main 8d64fd3` (the
row-4 floor). Allocated by Waffles (dm `cc700cae`), denominator confirmed
(dm `d4efac49`), word given: **Go.**

Predecessors that this design consumes rather than repeats:
`docs/design/beamr/briefs/AR-1-LANDING-GATE.md` (685 lines, read whole) ·
`AR-1-REMEDY-PROPOSAL.md` (r1.0, landed `a4e8802`) ·
`evidence/accumulator-rooting/REMEDY-PROBE-RECORD.md` (landed `fcd1be2`) ·
`gate-logs/110/RESULTS.md` (the per-site red-at-parent floor).

---

## 0. THE ONE-LINE SHAPE OF THE DEFECT, RE-DERIVED AT THE BYTES

A heap `Term` is held in an ordinary Rust local **across a call that can
collect**. The collection moves the object; the local keeps the old pointer.

⭐ **The sinks are not the defect, and this is the load-bearing fact of the
whole design.** `alloc_tuple` (`alloc.rs:33`), `alloc_cons` (`:122`),
`alloc_list_with_tail` (`:235`) and `alloc_map` (`:270`) each open
`with_rooted(&args, ..)` *before* `ensure_heap_space`. A term is safe for
exactly as long as it is an argument to a sink. **The unsafe region is the
accumulation loop that runs before the sink is ever called.**

⇒ the remedy has to cover the accumulation window. Nothing needs doing to the
sinks to make them *safe*; the sinks change only to make the unsafe form
*unwritable* (§4, phase 4).

## 1. WHAT LANDED FIRST — `TermAccumulator`

`crates/beamr/src/native/context/accumulator.rs`. Elements live in the process
native root stack, so any collection during the accumulation traces and
forwards them, and every read returns the current post-GC value.

Surface: `ProcessContext::with_accumulator(body)` → `push` · `get` · `set` ·
`len`/`is_empty` · terminals `to_list` · `to_list_with_tail` · `to_tuple` ·
`to_map_pairs` · `sort_pairs_by_key`.

Three decisions worth their own lines:

- **The alternating key/value run is how S3e dies.** `Vec<(Term, Term)>` puts
  both halves of a pair inside a tuple no collector can see. Pushing key and
  value into one rooted run keeps both traced, and `to_map_pairs` reads the run
  back as pairs. Odd length **refuses with `badarg`** rather than dropping the
  unpaired tail — gate row 6, no silent arm.
- **`sort_pairs_by_key` sorts in place, still rooted.** Reading out, sorting and
  writing back is sound because none of those steps allocates: no collection can
  run between the read and the write-back. Site 15 needs it and the sort must
  not be a hole in the rooting.
- ⛔ **`snapshot` is private.** Its one legitimate use is handing a whole run
  straight to a sink that roots its own arguments, with no allocation between —
  which is what the `to_*` terminals do. A public snapshot would re-admit
  exactly the shape this lane exists to remove.

### ⚠️ A CONSTRAINT NOBODY HAD WRITTEN DOWN — found at the bytes, not inherited

**`with_rooted` returns `badarg` when no process is attached**
(`context/mod.rs:816`). A remedy that reached for it unconditionally would turn
working **detached-context** calls into refusals. That is a behaviour change
wearing a fix's clothes, and sites 12/16 are *documented* as running on a
detached context in production (ledger, tier 2).

So `TermAccumulator` carries **two backings** — rooted when a process is
attached, plain owned `Vec` when not — for the same reason `alloc_tuple` and
`alloc_cons` already carry both: a detached context pushes a fresh owned block
per allocation into `detached_allocations`, and those blocks are never moved,
freed or collected. **There is nothing to root, so rooting is not merely
unnecessary there, it is unavailable.**

⭐ The remedy proposal never mentions this. It is the first thing the compiler
and the API docs told me that the design ahead of them did not.

## 2. SCOPE OF THE CLAIM — the accumulator is the vehicle, not the wall

A site migrated to `TermAccumulator` is **`SAFE-ROOTED`**. It is **not**
`STRUCTURALLY-ELIMINATED`: the bare form is still writable, so the gate's
criterion (*the shape CANNOT BE WRITTEN*) is **not** met by phases 1–3. It is
met only when the sinks stop accepting a bare `&[Term]` (phase 4).

⛔ Stated here so no per-site commit message can imply otherwise, and so that
Waffles' constraint — **no remedy is ratified just because a probe inverts** —
has a written counterpart at my own seat.

## 3. ⭐⭐ THE CENTRAL HAZARD OF THIS LANE: INVERTING A PROBE KILLS ITS OWN CONTROL

Every row-4 probe carries a two-way control. Site 3's is exemplary:

```
assert!(a_ok  > 0, "or the reader is broken rather than the site defective");
assert!(a_red > 0, "no LENGTH cell corrupted ... UNRESOLVED, not defended");
```

`a_red > 0` is the **positive control**: it proves the sweep actually applied
heap pressure. After the fix, no cell corrupts — so `a_red` goes to zero **and
the control dies with it.** The inverted probe would then assert `0 corruption`
on an instrument that can no longer demonstrate it is awake, which is the
asleep-instrument family exactly: *a green without a positive control is ZERO
evidence, not weak — same value whether the thing is safe or the instrument is
dead.*

⇒ **EVERY INVERTED PROBE MUST CARRY A SYNTHETIC POSITIVE THAT SURVIVES THE
FIX** — a local replica of the pre-fix accumulation, driven through the *real*
allocator under the *same* pressure regime, asserted still to corrupt. If it
stops corrupting, the pressure regime is gone and the green on the fixed path
is uninterpretable, whatever it says.

This is R10's law one level down, and it was already paid for once:
`ar1_shape_control.rs` exists because *a known-positive control keyed on a live
defect is destroyed by the repair it exists to survive.* The row-4 probes are
that mistake's shape again, in the probes rather than in the hunt — **not an
error in the floor, which was correct to pin the defect it measured, but a debt
the floor necessarily hands forward.**

## 4. THE ORDERING, AND WHY IT IS THIS ONE

Per-site commit granularity (gate row 4) and a structural remedy pull in
opposite directions: flipping five sink signatures forces ~52 call sites in one
commit. The resolution is to make the structural change **last**, after the
sites are already safe:

| phase | contents | disposition reached |
|---|---|---|
| 1 | `TermAccumulator`, additive, own tests + synthetic positive | — |
| 2 | the **11 native RED sites**, one commit each, probe inverted, ledger row per commit | `SAFE-ROOTED` |
| 3 | sites **12 + 16** (`beamr-wasm/src/convert.rs`) | `SAFE-ROOTED` |
| 4 | the **seal**: sealed `TermSource` at the five sinks + the ~52 measured migrations | `STRUCTURALLY-ELIMINATED` |
| 5 | row-2 accounting by name, shape-hunt re-run with controls, battery, landing | — |

Nothing in phases 1–3 breaks a caller, so each is landable on its own evidence.

## 5. R10-a — MEASURED, AND IT DOES **NOT** FIRE FOR THIS REMEDY

The ruled hazard is that a structural remedy stops the five `CONTROL-FIXTURE`
rows compiling. **Read at the bytes** (`ar1_shape_control.rs:1-60`): the
fixtures use `FixtureHeap` and `FixtureTerm`, **local types with no
relationship to `crate::term::Term` or the real allocator**, and the module
documents this as deliberate — "a change to the real types cannot reach them."

⇒ Candidate B changes `ProcessContext`'s sink signatures, which the fixtures
never call. **R10-a is defeated by construction for this remedy shape**, and
would fire only for a remedy that changed `Term` itself.

⚠️ It is **not** defeated for the *synthetic positives* of §3, which must use
the real allocator to be worth anything. Those are the constructs phase 4 may
kill — and by Waffles' advance ruling that is **a finding he wants, not a patch.**

## 6. TWO RULINGS STILL OPEN AT CALLY'S SEAT — proceeding, not blocking

`AR-1-REMEDY-PROPOSAL.md` §8 asked four. Waffles **sampled at his own hands and
discharged §8.1's contingency** — his half is unconditional. Open: **§8.2** (the
bracket call: measured cost is 5 signatures + ~52 migrations, against her
pre-set "20 ⇒ yes, 200 ⇒ no") and **§8.4** (ratify S2+wall as a disposition
class).

The proposal's own clause says *"until ruled: no sink signature changes."*
**That clause binds phase 4 and nothing earlier.** Phases 1–3 change no sink
signature, relax no criterion, and move no site out of a disposition the ledger
already allows. ⭐ Holding phases 1–3 for a ruling that governs phase 4 would be
[[phantom-gate-law]] exactly: *a blocker that outlives its own resolution holds
ruled work while looking like diligence.* Waffles allocated the lane and said
Go; the bracket question meets the work at phase 4, where it actually bites.

## 7. ADJACENT FINDINGS — recorded, NOT chased

- ⚠️ **`term/json.rs:385` sorts flatmap keys by raw `Term` value**
  (`pairs.sort_by_key(|(key, _)| *key)`). For boxed binary keys that is a sort
  by **heap address**, not by term order — and addresses change under exactly
  the collections this lane is about. Whether flatmap key order is thereby
  wrong is a **separate question with its own evidence**; this lane
  **preserves the behaviour byte-for-byte** and does not silently correct it.
  Site 7 (`uri_bifs.rs:60-93`) does not sort at all, which is the same question
  from the other side.
- `shape_hunt.py` skips `src/native/context/`, so the accumulator module is
  **invisible to the hunt**. Correct here — it is the remedy, not a defect —
  but it means the hunt cannot police the remedy's own bytes.

## 8. PER-SITE PLAN (tranche 1 — ELEVEN, not fourteen)

| site | file | carrier | shape | terminal |
|---|---|---|---|---|
| 1 | `distribution/control.rs:518` | `mfa` | single Term across `spawn_options_to_list` | `with_rooted` |
| 2 | `distribution/pg.rs:486` | `terms` | `Vec<Term>` / S3a | `to_list` |
| 3 | `native/code_management_bifs.rs:145` | `list` | threaded tail / S3b | `to_list` |
| 5 | `native/file_meta_bifs.rs:166` | `terms` | `Vec<Term>` / S3a | `to_list` |
| 7 | `native/stdlib_stubs/uri_bifs.rs:60` | `keys`+`values` | two `Vec<Term>` | `to_map_pairs` |
| 8 | `native/stdlib_stubs/uri_bifs.rs:127` | `terms` | `Vec<Term>` / S3a | `to_list` |
| 9 | `native/stdlib_stubs/uri_bifs.rs:129` | `key` | single Term across the value arm | `with_rooted` |
| 10 | `native/udp_bifs.rs:245` | `ip` | single Term across `alloc_binary` | `with_rooted` |
| 11 | `term/json.rs:365` | `tail` | threaded tail / S3b | `to_list` |
| 14 | `native/stdlib_stubs/string_bifs.rs:89` | `terms` | `Vec<Term>` / S3a | `to_list` |
| 15 | `term/json.rs:379` | `pairs` | `Vec<(Term, Term)>` / S3e | `sort_pairs_by_key` + `to_map_pairs` |

**Site 4 is NOT in this table and is not to be touched** — defended by the
caller's prereserve; its probe
(`ar1_site4_defended_by_the_callers_prereserve_not_by_the_site`) is the one that
**stays green through a fix** and must never be inverted by pattern-matching the
set. **Site 6** stays open unless a serialised leg falls out naturally.

Four of the eleven (1, 9, 10, and the key half of 8/9) are **single-Term
carriers**, not collections. They take `with_rooted` directly and never touch
`TermAccumulator` — worth saying because "eleven sites, one remedy" is the
composition error row 4 already refused once.
