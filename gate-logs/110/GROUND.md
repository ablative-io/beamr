# AR-1 ROW 4 — measured ground for the per-site red pass

Artemis Peach. Ground pin: `f993280` (= `origin/main` at the time of writing).
Released by Waffles' DM `433fd43c` (2026-08-16) as **common work by
construction** — required under any disposition, so it does not wait on Cally's
rulings 2 and 4. What waits on her is the DISPOSITION (which site takes which
treatment, where the exactness wall falls), not the proof that a site is broken.

## 1. The population, and who owns what

`dispositions.json` carries **22 rows: the 17 sites + 5 `CONTROL-FIXTURE`
rows.** Of the 17:

| state | ids | count |
|---|---|---:|
| red-at-parent DEMONSTRATED (Cally, Amendment 6) | 13, 17 | 2 |
| another seat's (row 8 — NOT absorbable) | 14 | 1 |
| **undemonstrated and mine** | 1–11, 15 (native) · 12, 16 (wasm) | **14** |

⛔ Site 14 is `native/stdlib_stubs/string_bifs.rs::bif_split`, Osiris'. Osiris is
withdrawn with no replacement, so **reassigning it is a ruling, not my call.**
It is named here and left alone — row 8 forbids quietly sweeping it in.

## 2. The pins this lane inherits are UNREACHABLE COMMITS — and the evidence survives anyway

`git cat-file -t` says all four exist; `git branch -a --contains` returns EMPTY
for every one:

| pin | cited as | contained by |
|---|---|---|
| `9587d2f` | the sink census (row 5's DAG proof) | nothing |
| `35dc5ae` | the gate + Amendment 1 | nothing |
| `ab1f1f9` | Amendment 2 | nothing |
| `308b448` | Amendment 6 / sites 13+17 red-at-parent | nothing |

This is the #82 unreachable-commit class again. **It is NOT an evidence
failure, and I checked before saying so.** The AR-1 work reached `main` as
`d628feb`…`814dd3d` — rebased twins with different shas. The load-bearing
question is not ancestry but whether the SITE BYTES moved, and they did not:

* `crates/beamr-wasm/src/convert.rs` — blob `3f19a2c3…` at `308b448` **and** at
  `HEAD`. Identical.
* `crates/beamr/src/term/json.rs` — blob `7ac3c68d…` at both. Identical.
* Corroborated a second way: neither file appears in `git diff --name-status
  308b448 814dd3d`.

⭐ **BANKED INSTRUMENT RULE — the durable pin for a site-level red is the FILE
BLOB SHA, not the commit sha.** A blob sha survives a rebase; a commit sha does
not, and "red at its own parent commit" is a claim about bytes, not about
history. This is the scaled-up form of Waffles' constraint: across 17 sites, a
commit-shaped pin is 17 chances to cite something unreachable.

⇒ **Citable pin for Cally's row-4 work is `814dd3d`** (on main), not `308b448`.

## 3. Every site still exists at HEAD — measured, with the instrument observed red

`gate-logs/110/site_census.py` re-locates all 17 by their own bytes (function
name in the ref's committed blob, not by line number) and reports drift instead
of assuming its absence.

**Result at `f993280`: EXACT 17 · DRIFTED 0 · MISSING 0.**
Sites 15, 16, 17 carry no `function_line` in the ledger (the S3e rows are less
fully recorded) and are located by name — reported as `NO-REC-LINE`, not
silently counted as exact.

Controls, all three observed red before the census was believed:

| break | verdict | required |
|---|---|---|
| recorded line +1 | `DRIFT-1` | `DRIFT-1` |
| function name mangled | `MISSING-FN` | `MISSING-FN` |
| file path mangled | `MISSING-FILE` | `MISSING-FILE` |

## 4. The rig for a native red — and why the existing tests cannot produce one

Amendment 6 established both halves wasm-side; both hold at HEAD, native-side,
at my own reading:

1. **No process ⇒ no collection at any input size.** `ProcessContext::new()`
   sets `process: None` (`native/context/mod.rs:514`) and `ensure_heap_space`
   (`:788`) early-returns `Ok(())` without one.
2. **Immediates ⇒ no allocation between bind and use.**

The rig is already in-crate idiom — `Process::new(pid, capacity)` +
`context.attach_process(process, 0)` — used at 20+ call sites including
`udp_bifs.rs:457`, which is site 10's own file.

⛔ **AND THE EXISTING GREEN TEST AT SITE 4 IS THE EXACT TRAP CALLY NAMED.**
`dictionary_bifs::tests::get_0_returns_complete_dictionary_as_tuple_list` drives
the site through `bif_get_0` with `Term::atom` and `Term::small_int` — all
immediates. It names the right function, hits the right line, and is
**structurally incapable of failing on this class.** It has been green the whole
time.

## 5. ⭐ THE FINDING THAT RESHAPES THE PASS: a REAL verdict is a SHAPE verdict, not a DEFECT verdict

Site 4 (`dictionary_bifs::entries_to_list`) carries the flagged shape — a bare
`Vec<Term>` accumulating across `alloc_tuple` calls, terminal `alloc_list`. But
its caller **already prereserves**, with a comment saying exactly why:

```rust
let entry_count = context.dict_len()?;
// Reserve while the entries are still rooted by the process dictionary. ...
context.ensure_heap_space(entries_heap_words(entry_count))?;   // entry_count * 5
let entries = context.dict_get_all()?;
entries_to_list(&entries, context)
```

The arithmetic reads exact: per entry, one 2-tuple (`1 + elements.len()` = **3**
words) plus one cons in the terminal `alloc_list` (**2** words) = **5**.

Read at the bytes, the loop cannot collect:
* `alloc_tuple` calls `ensure_heap_space(3)` internally, but `ensure_space`
  returns early while `available() >= words` — and the caller's 5N reserve keeps
  it there for the whole loop.
* `alloc_list` reserves `elements.len() * 2` **and roots every element** via
  `with_rooted`, so a collection at the terminal call forwards them correctly.
  The vulnerable window is only *during accumulation*.

**One defeat path was hypothesised and MEASURED CLOSED:** `ensure_space` also
collects when `virtual_binary_pressure_exceeds_heap`, which a word-exact
prereserve does not account for. But `increase_virtual_binary_heap` is called
from **exactly one place in the tree — `gc/tests.rs:86`**. Nothing in production
drives it, so that path cannot be reached from a BIF. (Recorded as an adjacent
observation: the virtual-binary-pressure accounting appears inert in production.
NOT this lane's to fix — disclosed, not touched.)

⇒ **Expected verdict for site 4: REFUTED — prereserve-defended.** Held as an
expectation, not a result, until the two-arm probe runs.

### What this means for the pass

Row 1's detector is **ordering-sensitive by design** — it flags a rooting call
that lands after the first collecting call on the carrier's live range. That is
a true reading of the SHAPE. It cannot see a caller's prereserve. So a `REAL`
verdict from it means *the shape is present*, not *the defect fires*.

**Row 4 exists precisely to separate those two.** The honest output of this pass
is therefore a per-site verdict of **RED-DEMONSTRATED or REFUTED**, not fifteen
reds. Row 8 already contemplates this — "his verdict may be 'false positive' …
a corrected number is worth more than a padded one" — and the instrument's own
FP rate is recorded at 20/69 ≈ 29%.

## 6. The instrument a prereserve-defended site requires — TWO ARMS

A green from a probe on a defended site proves nothing on its own: it cannot
distinguish *"the prereserve saved it"* from *"the probe never exercised the
mechanism"*. That is Amendment 6's lesson restated one level down.

**The decisive instrument:**

| arm | production bytes | required |
|---|---|---|
| A | unmodified | **GREEN** — values read back exactly |
| B | the caller's `ensure_heap_space` prereserve DELETED | **RED** |

Arm B is the positive control: it proves the probe can see this defect at this
site, and simultaneously proves the prereserve is what prevents it. If arm B
does not go red, the probe is blind and **arm A's green is worthless** — the
probe is rebuilt, not reported.

This is row 3's exactness-wall pattern carried into row 4.

## 7. Sequencing

1. Build and measure site 4 at my own hands — establishes the two-arm pattern.
2. Write the pattern up as the dispatch brief.
3. Remaining sites: dispatch, verify at the sources (a worker report is a claim).
4. Per-site verdicts land as evidence artefacts, following Cally's row-4 form —
   **probe source + transcript committed, live test lands with the fix.** That
   is how "the red committed" (Waffles) and "banked, not landed, because it is
   red at HEAD" (Amendment 6) are both satisfied; her own artefacts
   (`row4_probe.rs.txt`, `row4_red_at_parent.txt`) are the precedent.
