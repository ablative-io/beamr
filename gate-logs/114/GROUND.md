# Sites 4 and 6 — measured ground at `049ffbf`

The last two `PENDING` crossings, and the only thing between AR-1 row 2 and
discharge. Every fact below was read at the bytes.

⭐ **Headline: these are NOT the same problem, and only one of them is the
accumulator's to solve.**

## Address check first

Rows 4 and 6 carry `function_line` / `bind_line`, which are the ledger's
**unchecked** fields — the ones that drifted +74 in silence during #111 because
nothing verifies them. Re-derived here:

| row | claims | measured at `049ffbf` | |
|---|---|---|---|
| 4 | `entries_to_list` @ 127 | `dictionary_bifs.rs:127` | ✅ |
| 6 | `bif_os_getenv_0` @ 62 | `erlang_stubs.rs:62` | ✅ |

No drift this time. Recorded because a checked address is worth nothing if
nobody re-checks it, and these two are checked by hand or not at all.

---

## SITE 6 — `bif_os_getenv_0`, and it is straightforwardly fixable

```rust
let mut variables = Vec::new();
for (key, value) in std::env::vars() {
    variables.push(context.alloc_binary(format!("{key}={value}").as_bytes())?);
}
context.alloc_list(&variables)
```

- **Exactly one carrier**: `variables`, a `Vec<Term>` of freshly allocated
  binaries held across `alloc_binary`, which can collect.
- `key` / `value` are Rust `String`s from `std::env::vars()` — **not** `Term`s.
  Nothing else in the loop is a heap term.
- **No prereserve anywhere.** Unlike the dictionary BIFs, this site has no
  `ensure_heap_space` guarding it. The ledger's "the site's shape is unprotected"
  is confirmed at the bytes.

⇒ **Textbook AR-1, single carrier, fully discharged by `with_accumulator`.**

### The deferral reason, and why it dissolves

Row 6 was deferred on a *named mechanism*: `std::env::vars()` is process-global,
so controlling the population size means mutating an environment other parallel
tests read. That reason is real — and it is a reason about **how the loop gets
its input**, not about the loop.

Extract the accumulation into a helper that takes an iterator of pairs, and the
population becomes a **parameter** instead of ambient global state. The probe
then needs no environment access at all, no serialised leg, and no
`#[serial]`-style coordination. The deferral's own mechanism is what the
refactor removes.

⇒ **Plan: extract → accumulate → red-first probe on a synthetic population.**

---

## SITE 4 — `entries_to_list`, and ⛔ THE ACCUMULATOR CANNOT DISCHARGE IT

```rust
fn entries_to_list(entries: &[(Term, Term)], context: &mut ProcessContext) -> Result<Term, Term> {
    let mut tuples = Vec::with_capacity(entries.len());
    for &(key, value) in entries {
        tuples.push(context.alloc_tuple(&[key, value])?);
    }
    context.alloc_list(&tuples)
}
```

**There are TWO unrooted carriers here, not one.**

1. `tuples` — the accumulator's natural target.
2. ⭐ **`entries` itself** — a `&[(Term, Term)]` of *heap terms*, read on every
   iteration, across an allocating call. An accumulator around `tuples` leaves
   this one exactly as it was.

And `entries` is **not minted here**. Both callers create it upstream:

| caller | mints the carrier at | still rooted after? |
|---|---|---|
| `bif_get_0` (`:74`) | `context.dict_get_all()` → `Vec` | dictionary still holds the originals |
| `bif_erase_0` (`:99`) | `context.dict_erase_all()` → `Vec` | ⛔ **NO — the entries have been REMOVED from the dictionary** |

`ProcessContext::dict_get_all` (`context/mod.rs:1426`) is
`process.dict_get_all().to_vec()` — the `Process` method returns a **borrowed
slice** the dictionary roots; the context wrapper **copies it out into an owned
`Vec`**. That copy is where the unrooted carrier is born, one call above the row's
own address.

For `bif_erase_0` the situation is sharper still: after `dict_erase_all()`
**nothing roots those terms at all**. They exist only in the Rust `Vec`. The
prereserve is not merely helpful there — it is the only reason a collection
cannot run between the drain and the last `alloc_tuple`.

### What actually defends site 4 today

The caller's `ensure_heap_space(entry_count * 5)`, taken **before** the copy/drain
while the dictionary still roots the entries. #110 proved this load-bearing with
positive controls: arm B (reserve deleted) **RED**, arm D (`entry_count*2`)
**RED**, arm C (`5N-1`) **GREEN**.

⚠️ Arm C is worth restating: one word short is absorbed at the terminal
`alloc_list`, so what is load-bearing is **coverage of the accumulation**, not
exactness of the total. Sizing precision is not the invariant; *sufficiency* is.

### So what would discharge it

Not an accumulator. The real defect class is that **`dict_get_all` /
`dict_erase_all` hand out an unrooted `Vec<Term>` and trust every caller to have
reserved first.** Discharging site 4 means making "drain without reserving"
unrepresentable rather than merely unwritten — the same move as #86, #94, #103,
#112.

That is a **core `ProcessContext` API change with two call sites**, not a
test-local fix. It is a different size and a different blast radius from site 6.

⇒ **Plan: site 6 tonight; site 4 sized and named, with its remedy shape stated
and a re-check trigger, per tonight's park-reasons-expire law.**

## What this ground does NOT claim

- It does **not** say site 4 is safe to leave. It says the *remedy named in the
  ledger* (`replacement_construct: with_accumulator`) is the wrong instrument for
  it, which is a finding about the remedy, not a clearance for the site.
- It does **not** re-grade #110's evidence. The positive controls stand; this
  builds on them.
