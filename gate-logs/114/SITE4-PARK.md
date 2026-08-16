# Site 4 — parked, with the reason AND its re-check trigger

⛔ **This is NOT a park for difficulty, and it is NOT "looked at and found
clean."** Site 4's shape is real, present, and currently defended by something
other than itself. What is parked is a **specific remedy**, for a specific
reason, with a specific date-independent trigger.

## Finding 1 — the ledger's named remedy is the WRONG INSTRUMENT for this row

Every other AR-1 crossing was discharged with `with_accumulator`. Site 4 cannot
be, and the reason is structural rather than incidental.

```rust
fn entries_to_list(entries: &[(Term, Term)], context: &mut ProcessContext) -> Result<Term, Term> {
    let mut tuples = Vec::with_capacity(entries.len());
    for &(key, value) in entries {
        tuples.push(context.alloc_tuple(&[key, value])?);
    }
    context.alloc_list(&tuples)
}
```

**There are TWO unrooted carriers, not one:**

1. `tuples` — the accumulator's natural target.
2. ⭐ **`entries` itself** — a slice of *heap terms*, read on every iteration
   across an allocating call. An accumulator around `tuples` leaves it untouched.

And `entries` is **not minted here**. It is created one call above:

| caller | mints the carrier | still rooted after? |
|---|---|---|
| `bif_get_0` (`dictionary_bifs.rs:74`) | `context.dict_get_all()` → owned `Vec` | dictionary still holds the originals |
| `bif_erase_0` (`dictionary_bifs.rs:99`) | `context.dict_erase_all()` → owned `Vec` | ⛔ **NO — they have been REMOVED from the dictionary** |

`ProcessContext::dict_get_all` (`context/mod.rs:1426`) is
`process.dict_get_all().to_vec()`. The `Process` method returns a **borrowed
slice the dictionary roots**; the context wrapper **copies it into an owned
`Vec`**. That copy is where the unrooted carrier is born.

So rooting `tuples` would produce a *partially* fixed site whose remaining
hazard is invisible — arguably worse than the honest current state, where the
prereserve is documented as load-bearing at both call sites.

## Finding 2 — what actually defends it, and it is not the site

`ensure_heap_space(entry_count * 5)`, taken **before** the copy/drain while the
dictionary still roots the entries. #110 proved this load-bearing with positive
controls: arm B (reserve deleted) **RED**, arm D (`entry_count*2`) **RED**, arm C
(`5N-1`) **GREEN**.

⚠️ Arm C is the instructive one: one word short is absorbed at the terminal
`alloc_list`, so what is load-bearing is **coverage of the accumulation**, not
exactness of the total. Sufficiency is the invariant; precision is not.

## The remedy shape, worked out

Not an accumulator. The real defect class is that **`dict_get_all` /
`dict_erase_all` hand out an unrooted `Vec<Term>` and trust every caller to have
reserved first.** Discharging site 4 means fusing reserve + drain + build so
"drain without reserving" is unrepresentable — the #86/#94/#103/#112 move:

```rust
// on ProcessContext, reserve and build fused, no Vec<Term> ever escapes
pub fn dict_entries_to_list(&mut self) -> Result<Term, Term>
pub fn dict_erase_all_to_list(&mut self) -> Result<Term, Term>
```

Both `entries_to_list` call sites collapse into these. `bif_get_keys_1`
(`:113`) has the same reserve-then-copy shape and should ride along.

## ⛔ THE PARK REASON — and it is a fact, not a preference

The full remedy requires **removing `dict_get_all` / `dict_erase_all` from
`ProcessContext`**, because leaving them is leaving the footgun loaded.
`ProcessContext` is the **embedder-facing surface for native BIF authors**, its
methods are `pub`, and **beamr is a published crate**. Removing them is a
**semver-breaking change**.

That is not a reason to avoid the work. It is a reason the work belongs to a
**breaking release**, not to a fix lane landing on main tonight.

## THE RE-CHECK TRIGGER — park reasons expire unchecked

**Trigger: the 0.19.0 cut.** Site 4's remedy goes on the 0.19.0 cutter's list,
which already exists and already carries the `spawn_process` scaffold deletion
(#104). At that cut, the park reason is re-measured, not assumed:

1. Re-derive that `ProcessContext::dict_get_all` / `dict_erase_all` still have
   exactly one caller each (`git grep`). **If a third caller has appeared, the
   park is void and the row escalates** — the hazard would have spread.
2. Re-run #110's arm B to confirm the prereserve is still what defends the site.
3. Then fuse, delete the raw pair, and disposition row 4.

## ⭐ AND A NON-BREAKING INTERIM IS AVAILABLE NOW — needs a word

The fused methods can be **added** now, both call sites migrated, and the raw
pair left in place (unused, documented as hazardous) for deletion at 0.19.0.
That discharges the hazard **at the actual call sites tonight** with zero
breakage, and reduces the 0.19.0 job to a deletion.

I have not done this unilaterally because it adds surface to a published
embedder-facing type, which is a design call rather than a fix. **Say go and it
is a short lane.**

## What this park does NOT claim

- It does **not** claim site 4 is safe to leave indefinitely. It claims the
  defence is real, measured, and located in the caller rather than the site.
- It does **not** re-grade #110's evidence; it builds on it.
- Row 4 stays **PENDING**, so `--sign-off` keeps refusing. **Silence about a site
  is the failure — the ledger should go on saying so.**
