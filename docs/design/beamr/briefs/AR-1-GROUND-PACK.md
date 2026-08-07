# AR-1 GROUND PACK — accumulator rooting

**Author:** Artemis Peach (beamr owner seat). **Status:** DRAFT, ground only.
**Lane:** `AR-1`. **Measured at** `2419556` on `artemis/rf-006` (unpushed).
**Evidence dir:** `docs/design/beamr/briefs/evidence/accumulator-rooting/` — a
**sibling** of `review-23-07/`, never a child, so RF-006's tree cannot reach
AR-1's rows without a deliberate step. **No fix is designed
here**; the lane lead writes the acceptance conditions **before** seeing a fix,
by her ruling and for the pre-registration reason — a check authored after the
artefact is shaped by it, invisibly to whoever wrote it.

---

## 1. The class, stated so a detector can be written against it

**Build a boxed term in a loop, hold it in a bare Rust local (a `Vec<Term>`, or
a threaded `tail`), allocate again, then hand the collection to a sink.**

Every allocation after the first can collect. A collection traces
**x-registers below `live_x`** plus **`process.native_roots`** — a Term in a
bare Rust local is **neither**, so it is neither traced nor forwarded.

### ⭐ Why every one of these sites looks defended

The terminal `alloc_list` / `alloc_tuple` **does** root its elements
(`with_rooted` + re-read, `native/context/alloc.rs`). **It roots them after
they have already gone stale, and rooting a stale pointer does not recover
it.** The rooting call is right there, usually on the function's last line.

⇒ **DETECTOR REQUIREMENT (the lane lead's, and the most important line in the
pack): the rule must be ORDERING-SENSITIVE — *rooted BEFORE the first
collecting call*, never *rooted somewhere in this function*.** Any instrument
keyed on **presence** — a grep, a lint, a reviewer's eye, the obvious
fix-checker somebody writes next week — **passes all fourteen while they are
still broken.** This belongs in the gate as a break-it row.

## 2. The discriminator (governing fact, established at the bytes)

| callee | takes Term args? | roots them? | consequence |
|---|---|---|---|
| `alloc_tuple`, `alloc_cons`, `alloc_list`, `alloc_list_with_tail`, `alloc_map` | yes | **yes** — `with_rooted` + re-read | an argument is SAFE |
| `alloc_binary`, `alloc_fd_resource`, `alloc_reference`, `alloc_float`, `alloc_bigint`, `alloc_external_*` | **no** | nothing to root | **can collect, roots nothing** |

⇒ **Is the carrier an ARGUMENT to the collecting call (safe), or merely LIVE
ACROSS it (real)?** Also: `try_pid` is an **immediate**; `alloc_external_pid`
is **boxed** (4 words). Detached `ProcessContext::new()` never collects.

## 3. The facility already exists, in TWO correct shapes

**11 `with_rooted` scopes / 11 functions / 6 files** (non-test, outside
`native/context/`).

| shape | how | where |
|---|---|---|
| **S1 — root as you go** | `with_rooted` + `rooted_push` after each allocation + `rooted()` re-read | 8 fns / 4 files: `ets_bifs` ×4, `etf_bifs::segments_to_iolist`, `json_bifs::{parse_array, parse_object}`, `string_bifs::bif_next_grapheme` (**10** `rooted_push` sites — two functions hold two each) |
| **S2 — root up front, reserve once** | `with_rooted` over the whole input, one `ensure_heap_space`, then **only `_prereserved` allocators** | 3 fns / 2 files: `lists_bifs::list_from_vec`, `maps_bifs::{bif_maps_find, make_map_from_entries}` |

**S1 is what the seventeen need.** S2 is correct only when the inputs already
exist — and note **S2's safety is exactly the arithmetic that four
`UNRULED-PRERESERVE` rows are deferred on.**

⚠️ **`list_from_vec` is a SINK, not a cure.** It builds a list by threading a
`tail` — the identical shape to defect #11 — and is safe *purely* because it
receives an already-complete `&[Term]` and roots before reserving. Handing it
a `Vec` accumulated across allocations inherits the defect whole.

⇒ **NOT A MISSING FACILITY — AN UNEVENLY APPLIED ONE.** No name-based or
facility-based search finds this, which is why the remedy is a sweep with a
rule rather than fourteen patches.

## 4. The population — SEVENTEEN sites (see the A4 STOP at the end: 14 → 17)

**The fourteen known at the time of the RF-006 verdict pass:**

`code_management_bifs::all_loaded/list` · `uri_bifs::bif_uri_string_dissect_query/{key,terms}` ·
`uri_bifs::bif_uri_string_parse/values` · `udp_bifs::finish_udp_recv/ip` ·
`control.rs::alloc_spawn_request/mfa` · `pg.rs::members/terms` ·
`dictionary_bifs::entries_to_list/tuples` · `file_meta_bifs::finish_list_dir/terms` ·
`erlang_stubs::bif_os_getenv_0/variables` · `term/json.rs::array_to_list_term/tail` ·
`beamr-wasm/convert.rs::{json_value_to_term, array_to_term}/tail` ·
**`string_bifs::bif_split/terms` — OSIRIS' LANE, handed over, not touched.**

Eleven of these fourteen are literally the same three lines; the three found
since are a third variant (§A4 STOP). **Present in every released
version. No JIT required. `code:all_loaded/0` is not exotic.**

## 5. ⭐ THE MEASUREMENT — and it refutes the proposed remedy

The lane lead proposed: *make a bare `Vec<Term>` unable to reach
`alloc_list`/`alloc_tuple` at all — take a rooted handle in the signature, and
the compiler becomes the check.* She explicitly did **not** rule it, and set
the bracket: **"If it's 20, the type change is the answer. If it's 200, it
isn't."** Measured, production files, callee-resolved:

| sink | sites | literal `&[a, b]` | variable collection | other |
|---|---|---|---|---|
| `alloc_tuple` | 107 | 99 | 7 | 1 |
| `alloc_list` | 77 | 39 | 36 | 2 |
| `list_from_vec` | 11 | 0 | 11 | 0 |
| `alloc_map` | 8 | 2 | 6 | 0 |
| `alloc_list_with_tail` | 2 | 0 | 2 | 0 |
| **TOTAL** | **205** | **140** | **62** | **3** |

Plus `alloc_cons` at **10** production sites (tail-threading; takes no
collection).

> ### ⛔ **205. THE TYPE CHANGE IS NOT THE ANSWER — by her own bracket.**

**But the useful number is the second one: only 65 of 205 are
accumulator-capable** (62 variable + 3 other). **140 pass a fixed literal
array and can never be an accumulator** — they are pure cost under a signature
change and carry no risk at all. A remedy aimed at the 65 costs less than a
third of one aimed at the type.

**This does not select a remedy** — that is a design ruling and it is not
mine to make. It removes one candidate and sizes the rest.

## 6. Disclosures — read these before trusting any number above

- **The sink census took three generations, and each disagreed with the last.**
  **r1** required a `.`/`::` receiver, so it was structurally blind to
  **free-function** calls — it reported `list_from_vec` = 0 when the truth is
  11. **r2** over-corrected and counted every free call of a matching *name*,
  conflating **five different functions** that share these names
  (`ProcessContext` methods; `distribution/etf.rs`'s heap-based `alloc_list`;
  `ets/match_{arena,spec}.rs` arena allocators; `gc` test helpers taking
  `&mut process`). **r3 resolves the callee.** ⇒ **r1 is C-vi's defect one
  level out: a finder keyed on a marker its target does not carry.**
- **C-vii, found by the lane lead's question and not by me.** My summary said
  the facility was used in "nine places … two of those files also hold an
  unfixed instance." The count was keyed on `rooted_push`, so it **could not
  see S2 at all**; and it is **one** overlapping file, not two — the phantom
  second was `native/stdlib_stubs/json_bifs.rs` conflated with
  `crates/beamr/src/term/json.rs`. **Different modules, both basenames say
  "json."** Three name-collisions in one night, all in my own prose.
- **`ets_bifs.rs` compounds, on an axis neither of us expected.** It is *not*
  in the facility∩defect overlap. But it holds **4 of the 11 correct uses**
  *and* `bif_info_1`, one of the four `UNRULED-PRERESERVE` rows. **The file
  that best demonstrates "the tool was right there" is also a file with a
  deferred row.**
- 🔴 **THE POPULATION IS A FIELD OF VIEW, AND THE THIRD SHAPE WAS REAL.** I
  wrote "nothing proves there is not a third" as a caveat. **It existed, and
  one targeted pass with a DIFFERENTLY-SHAPED instrument found it (§A4 STOP).**
  A second generation of the same binder could never have.
- **The closed-list coverage bound still applies**: only *direct* calls to the
  37 named collecting primitives are recognised. A crossing whose only
  collecting call is one level of indirection (`spawn_options_to_list`,
  `ipv4_tuple`, `ok_tuple`) is invisible.

## 7. Open, and explicitly NOT decided here

1. **The remedy shape.** Measured out: the type change. Everything else open.
2. **The four `UNRULED-PRERESERVE` rows** — deferred with named, cheap
   discharge conditions (three word-count functions). Whether they belong to
   this lane or stay with RF-006 is the lead's call.
3. **Ordering-sensitive detector** — required (§1); form is the gate author's.
4. `string_bifs::bif_split` is **Osiris'**; handed over with the instrument
   and its disclosed blindness attached.

---

# 🔴 A4 STOP — A THIRD CARRIER SHAPE EXISTS. 14 → 17.

**Found by running the lane lead's row 7 (*hunt a third shape, or record the
search*). It took one pass.** Reported at discovery, **before any fix design**,
per A4. Nothing fixed, nothing designed.

## S3e — THE Vec-OF-TUPLES ACCUMULATOR

```rust
let mut pairs = Vec::with_capacity(object.len());
for (key, value) in object {
    let key_term   = string_to_binary_term(key, context)?;  // allocates -> BOXED
    let value_term = value_to_term(value, context)?;        // allocates, recursive
    pairs.push((key_term, value_term));                     // <-- carrier is a TUPLE ELEMENT
}
context.alloc_map(&keys, &values)                           // roots them AFTER they are stale
```

⭐ **Why BOTH binders are blind to it:** w2 recognises an accumulator by
`Vec<Term>` + `.push(<term-ish>)`. **This is a `Vec<(Term, Term)>` and the push
argument is a TUPLE LITERAL, not a Term.** The carrier is one element *inside*
a tuple *inside* a Vec. The annotation binder never sees a `: Term` either.
**Same trap as ever: the terminal `alloc_map` roots its arguments, after they
have gone stale.**

| # | site | carrier | verified |
|---|---|---|---|
| 15 | `term/json.rs::object_to_map_term` | `pairs` | `string_to_binary_term` (`:352`) and `value_to_term` (`:91`) both allocate |
| 16 | `beamr-wasm/convert.rs::json_value_to_term` (Object arm, `:147`) | `pairs` | `alloc_binary` + recursive `json_value_to_term` |
| 17 | `beamr-wasm/convert.rs::value_to_term` (JS-object arm, `:199`) | `pairs` | `alloc_binary` + recursive `value_to_term` |

⛔ **ALL THREE SIT IN FILES THAT ALREADY HOLD A KNOWN CROSSING** (`term/json.rs`
= #11, `convert.rs` = #12/#13). ⇒ **A "14 → 14, each nameable" gate would have
gone GREEN with three defects still live in the very files being repaired.**
That is precisely what the lead's row 2 exists to catch, and it caught it
before a fix existed. **The gate earned its keep before the lane started.**

### Ruled NOT crossings (checked, not assumed)

- `interpreter/opcodes/closures.rs:423,482` — `extracted`/`updates` hold values
  read from registers/maps; **no allocating call in the loop.**
- `constant_pool/mod.rs:391` — `materialise_literal_term` into a **pool with
  its own `push_root`** facility; different subsystem, own rooting. Not ruled
  safe, ruled OUT OF THIS CLASS pending its own look.
- `loader/decode/etf.rs:120` — same shape, **VESPER'S CLAIM. Not touched, not
  ruled. Flagged to the lead for routing.**
- `native/stdlib_stubs/encoding_bifs.rs:34`, `jit/ir_map.rs:101` — not Terms.

### Row 6's shape appears — in TEST code only

`udp_bifs.rs:466-476` has `context.alloc_tuple(terms).unwrap_or(Term::NIL)` and
`alloc_cons(..).unwrap_or(Term::NIL)` — **exactly the "looks like defensive
programming" silent arm.** It is inside `#[cfg(test)] mod tests` (opens
`:382`). **Test-helper code, NOT a production silent arm.** Recorded because
the shape is real and the next one might not be in a test.

### ⚠️ A sibling hazard, NOT my class, stated so it is not lost

`term/json.rs:386` `pairs.sort_by_key(|(key, _)| *key)` and
`constant_pool/mod.rs:398` `pairs.sort_by(..)` **order Terms by raw value.** If
a collection moves a boxed term mid-sequence the sort key changes underneath
the ordering. Different defect class, different lane, **flagged not chased.**

## ⇒ WHAT THIS DOES TO THE LANE'S OWN CLAIMS

- **The count is 17, not 14** (16 if Osiris refutes his).
- **"Eleven of the fourteen are one shape" is now "fourteen of seventeen are
  one FAMILY, in three variants"** — `Vec<Term>` push, threaded `tail`, and
  `Vec<(Term, Term)>` push.
- ⭐ **AND THE STANDING WARNING IS NOW A MEASURED FACT, NOT A CAVEAT.** I wrote
  "nothing proves there is not a third." There was. **It took one targeted pass
  to find, which means the marginal cost of looking was far below the cost of
  the disclosure I was writing instead.** ⇒ **A DISCLOSED BLIND SPOT IS NOT A
  DISCHARGED ONE — if naming it is cheaper than searching it, the naming is
  buying silence, not safety.**
