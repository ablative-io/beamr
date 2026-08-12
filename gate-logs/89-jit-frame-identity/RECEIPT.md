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
