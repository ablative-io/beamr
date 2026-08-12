# beamr #94 IMPORT-INVARIANT — battery receipt

Base `2ed9931` (= `origin/main`). Runner beside this file; six canon legs read
from the committed `gates.json` at run time, never transcribed.

## Verdict: 6/6 rc 0. COMPLETE marker DERIVED (legs_declared=6, legs_scored=6).

Tree check (three-artifact form, `wc` not `grep -c`): **4 pre, 4 post**, the
four files this commit touches, unchanged across the run. Interpreter logged:
`/usr/bin/python3`, Python 3.9.6.

## Axes — unchanged, and that was the prediction

**73 / 2107 / 0 / 0**, identical to the base. This change adds no test and
removes none: the five `.and_then(|entry| entry.as_ref())` hops and one helper
signature were edited *inside* existing tests, not deleted as tests. An axis
that moved here would itself have been the finding.

## The defect — and its size, stated honestly

`resolve_imports` declared `Vec<Option<ResolvedImport>>` and **never produced a
`None`**. Measured: **4** `resolved.push(Some(` sites, **0** `push(None)`. All
four arms — Native/Denied, Deferred, Code, Unresolved — push `Some`; a failed
resolution is a `ResolvedImportTarget::Unresolved` *variant*, not an absence.

⚠️ **The `Option` was vestigial, so no released version could hit this. This
fixes no observed misbehaviour.** What it removes is the ability to express one.

**What it would have cost had a `None` ever appeared.** The vector is
positional — entry `i` resolves `ImpT` entry `i`, and instructions name their
target by that same index:

* `prepare_module_with_origin_and_policy` ran
  `resolved_by_index.into_iter().flatten().collect()` before handing the vector
  to `module_from_parsed`;
* `jit/runtime.rs:132` does `current_module.resolved_imports.get(import_index)`
  with the **original** instruction index.

One `None` shifts every later import down by one and **silently dispatches to
the wrong function** — no error, no crash, a valid-looking target.

**Validation would not have caught it.** `validate.rs` bounds-checks
`import_index >= resolved_imports.len()` against the **unflattened** slice (it
runs before the flatten) and never inspects `Some`/`None`. The length it checks
is not the length used at runtime.

## The remedy is the type, not an assertion

`Vec<Option<ResolvedImport>>` → `Vec<ResolvedImport>`. The `flatten` is deleted
along with the `Option`. An assert was the wrong shape here — an assert is a
thing someone must remember to keep true.

## The falsifier — a COMPILE error, which is the strongest form available

⛔ **There is no runtime test to write for something unrepresentable**, so the
falsifier proves the compiler now refuses it. The exact expression that used to
compile and cause the shift was spliced back in:

    let shifted: Vec<ResolvedImport> = resolved_by_index.into_iter().flatten().collect();

Result — **compilation FAILS**, two errors:

* `E0277: ResolvedImport is not an iterator`
* `E0599: the method collect exists for Flatten<std::vec::IntoIter<ResolvedImport>>, but its trait bounds were not satisfied`

The probe was removed and the tree restored before the battery ran; the
restored tree re-checked clean. **The shift is now impossible to write, not
merely absent.**

## ⚠️ BREAKING API — this rides a minor bump

`resolve_imports` is `pub` and re-exported from `loader::mod`, so the return
type change is breaking for external callers. Each `0.x` minor is a semver
major, so this belongs to **0.19.0**, the same shape as #86's
unrepresentable-empty-bundle change which shipped in 0.18.0. Callers that
matched on `Option` drop that layer; callers already treating every entry as
present need no change.
