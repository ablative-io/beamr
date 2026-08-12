# #89 JIT-FRAME-IDENTITY — battery receipt

Pin: commit `e990b017dc4a26f212f4bf47b443eedd4ffa0c6a`, branch
`artemis/jit-frame-identity`, off `2551841` (origin/main at 0.18.1). Runner:
`runner.sh` beside this file. Six canon legs read from the committed
`gates.json` at run time.

## Verdict: 6/6 rc 0. COMPLETE marker DERIVED (legs_declared=6, legs_scored=6).

fmt 0 · clippy `-D warnings` 0 · wasm32-check 0 · wasm-tests 0 · tests 0 ·
blocking-call-in-native-bif 0.

## Named axes

Base `2551841`: result-lines 72 / passed 2080 / failed 0 / ignored 0.
This run: result-lines 72 / passed **2085** / failed 0 / ignored 0.

**+5 exactly, no new test binary** (these are lib walls, not an integration
gate — hence result-lines unchanged, which is itself the check that no binary
appeared unnoticed):

* `scheduler::execution::core::frame_resolution_tests::compiled_frame_reports_the_module_owning_the_compiled_function`
* `scheduler::execution::core::frame_resolution_tests::compiled_frame_reports_no_line_rather_than_the_modules_first_line`
* `scheduler::execution::core::frame_resolution_tests::interpreted_frames_keep_resolving_their_line_from_the_instruction_pointer`
* `interpreter::opcodes::exceptions::tests::compiled_frame_names_the_module_that_owns_the_compiled_function`
* `interpreter::opcodes::exceptions::tests::interpreted_frame_identity_agrees_whichever_source_it_comes_from`

Tree check (amended three-artifact form): **0 pre, 0 post**.

## Mutation M-FRAME-1 — pre-registered, then run

`MUTATION-M-FRAME-1-PREDICTION.txt` beside this file was written **before** the
mutation ran. Both pre-fix behaviours were restored (identity() discarding
mfa.0; the line guard removed) with every test byte-identical. Result matched
the prediction exactly: the **three compiled walls went RED by name**, and the
**two interpreted walls stayed GREEN**.

Those two are not weak arms and are not hidden. They assert the *no-op*
property — for an interpreted frame `mfa.0 == module.name` by construction,
because `current_mfa` derives from `Module::mfa_at_ip` which pairs the function
with that same module's own name — so a mutation confined to the compiled path
**cannot** move them. Had either gone red, the change would have reached
further than the commit claims and the fix would be wrong.

## What this battery does NOT say

It covers F1 (the module splice) and F2 (the fabricated line on the
crash-report path). It does **not** cover **F3**, a third and separate defect
found while answering an external report and left deliberately unfixed here:
`function_table` records the ip of the `FuncInfo` instruction while
`line_table` records the ip of the `Line` instruction, and a BEAM prologue is
`Label → Line → FuncInfo`. There is therefore exactly one ip per function
(every function after the first) at which `line_at_ip` already reports the next
function's head line while `function_at_ip` still reports the previous
function. Measured by decoding `test-workflows/sample/gleam@list.beam` through
beamr's own loader. The honest fix moves a function's recorded region to start
at its prologue rather than its `func_info`, which changes `mfa_at_ip` and
therefore `current_mfa`; that change is not made here because the path by which
a recorded position lands on that ip is not yet established, and a `Line`
instruction cannot raise on its own. Note that `Module::mfa_at_ip`'s own
doc-comment asserts the invariant F3 violates.

Neither F1, F2 nor F3 is attributed to any production incident. F2's proposed
attribution to an external crash report was **withdrawn** when its
pre-registered prediction was measured and failed.

---

## 🔴 CORRECTION APPENDED 2026-08-12 — one mutation arm did not discriminate

**Do not read the mutation section above without this.** It is appended rather
than edited: the original text is what was written at the time and stays
legible as such.

**F2's description in this receipt is wrong.** It says the crash-report
renderer resolved `line_at_ip(0)` and so "returned the module's first
line-table entry". It does not. `line_at_ip` binary-searches and subtracts one
from the insertion point, so `ip = 0` yields the first entry only if a line
marker sits at **exactly** ip 0. Measured across all 24 sample `.beam`
modules: ip 0 is the first function's leading `label` in every one and the
first `line` marker is at ip 1 in every one; and
`loader::load::tests::module_builds_function_and_line_tables` had already
asserted `line_at_ip(0) == None` on a built module all along.

⚠️ **Stated as observed behaviour, not as a contract.** `validate_module`
enforces nothing about prologue ordering, so this is what the current compiler
emits over the measured set — not a rule beamr refuses to break. A test
asserting `None` is the loader's behaviour, not its promise, and the difference
is exactly the kind of upgrade that produced the error being corrected here.

⇒ **`line_at_ip(0)` is `None`, so F2 produced no line and had no observable
effect.** The `!compiled` guard is still correct — it makes the absent line
structural rather than a coincidence of table layout — but it is a **hardening,
not a behaviour fix**, and the 0.18.2 entry now says so.

### What this does to the pre-registered mutation

`MUTATION-M-FRAME-1-PREDICTION.txt` predicted three walls RED. **Only two of
them were discriminating:**

* `compiled_frame_reports_the_module_owning_the_compiled_function` — **VALID**
  (F1, module splice).
* `compiled_frame_names_the_module_that_owns_the_compiled_function` — **VALID**
  (F1, the twin renderer).
* `compiled_frame_reports_no_line_rather_than_the_modules_first_line` —
  **NON-DISCRIMINATING.** It went red only because its fixture set
  `line_table = vec![(0, 0)]`, putting a line marker at an ip the loader never
  emits. On a realistic table the assertion holds with the guard and without
  it. Renamed to `compiled_frame_reports_no_line_from_its_placeholder_ip`, its
  fixture rebuilt to the real shape, and the test now states in its own body
  that it does not discriminate.

The pre-registration was honest and the prediction did match. **The fixture
underneath one arm was not sound**, which a prediction cannot catch — a
correct forecast of a wrong instrument's output is still wrong.

### The class, and what now gates it

⭐ **A FIXTURE THE REAL PRODUCER CANNOT BUILD PROVES THE DEFECT IN A WORLD THAT
DOES NOT EXIST.** A second instance was found in the same sweep —
`interpreter/opcodes/exceptions.rs` also built a marker at ip 0 — and is fixed
here too. Both are now gated by a new constructibility test,
`loader::load::tests::no_real_module_carries_a_line_marker_at_ip_zero`, which
sweeps every sample module and fails loudly if the assumption ever breaks.

⚠️ The refuting fact **was already pinned by a test in this repository**, in
the loader, twenty lines from where the tables are built. It was not absent
knowledge — it was known in one file and contradicted in another, which is the
same shape as F3, the defect this very lane was documenting. **The pattern
reproduced one layer down, inside the release that named it.**

### Unaffected

**F1 is unaffected and is now stronger.** An external reviewer has since shown
F1 rendering a real production frame off a shipped `0.17.0` exactly — pinned
module, mfa's function, mfa's module discarded, and no line — so F1 moved from
*measured in the code* to *confirmed in the wild*, with F2's absent line as
part of that confirmation. **#88 is untouched**: different helper, different
lane, own battery and census.


### Sweep output — the class enumerated and classified, not just patched

The remedy is a classification of every line-table literal whose first entry is
at ip 0, because a per-row fix never reaches the rows that closed before the
gate existed. **Two committed members, both rewritten, zero marked-controls:**

| site | was | disposition |
| --- | --- | --- |
| `scheduler/execution/core.rs` `module_with_first_line` | `vec![(0, 0)]` | **REWRITTEN** to `vec![(1, 0)]`, helper renamed `module_with_realistic_line_table`, dependent test's ip moved 0 → 2 |
| `interpreter/opcodes/exceptions.rs` `build_stacktrace_resolves_mfa_and_line_info` | `vec![(0, 0), (10, 1)]` | **REWRITTEN** to `vec![(1, 0), (10, 1)]`, position moved 0 → 2 |

**No third category and no survivors:** no literal in the class is retained as
a deliberate control, so none needs the marking that would otherwise be
required to stop the next reader mistaking a load-bearing control for a stale
fixture. `module.rs:814` (`vec![(2, 0), (6, 1), (10, 99)]`) is **not** a member
— its first entry is at ip 2, a shape the loader does produce.
