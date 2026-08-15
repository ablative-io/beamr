# RUN RECORD — #91 F3 reachability: can any recorded ip land on a prologue Line instruction?

Lane: #91, successor to #89 (compiled-frame identity). F3 as disclosed
in gate-logs/89-jit-frame-identity/RECEIPT.md: `function_table` records
the `FuncInfo` ip while `line_table` records the `Line` ip, and a BEAM
prologue is `Label → Line → FuncInfo` — so exactly one ip per function
(every function after the first) exists where `line_at_ip` already
reports the next function's head line while `function_at_ip` still
reports the previous function. The honest fix (move a function's
recorded region to its prologue) changes `mfa_at_ip`/`current_mfa`
semantics and was deliberately held until reachability was established.
This lane establishes it. All measured at beamr main `70f61f7`, my
hands, 2026-08-15. Context: the #193 road's widened predicate
(boundary-crossing without a compiled send) made this class current.

## VERDICT

**Tier 1 — every execution-recording path: UNREACHABLE.** No durable
recorded ip can hold the window ip. Producer-by-producer:

1. **Jump/yield/wait targets.** All label resolution goes through
   `label_index`, built by the loader mapping each label to the `Label`
   instruction's OWN ip (loader/load.rs:624-629). Yield-at-reduction
   records the jump TARGET (core.rs:958-969 jump_position_with_reduction
   — the only reduction-charge site, so the only mid-function suspension
   mechanism), wait/receive resumes are label targets likewise. Canonical
   loader output has no label between `Line` and `FuncInfo`, so no label
   maps into the window.
2. **Frame return ips.** `push_frame(…, return_ip, …)` records
   call_ip+1. For that to be the window ip the call would need to occupy
   the prologue `Label`'s ip — occupied by the Label instruction itself.
   The pathological fall-through layout (call as a function's last
   instruction) yields the next function's LABEL ip, one short of the
   window.
3. **Raise-site capture.** `capture_raw_stacktrace`
   (interpreter/opcodes/exceptions.rs:268-294) records the CURRENT
   code_position as the top entry; run_loop mutates position only on Ok
   outcomes (interpreter/mod.rs:289-294), so at raise time it is the
   RAISING instruction's own ip. `Instruction::Line`'s dispatch arm is a
   pure `Continue` (opcodes/mod.rs:385-390) — it cannot raise ("a Line
   instruction cannot raise on its own," the #89 receipt's own words,
   now grounded in the arm). Remaining capture entries are frame return
   ips (class 2).
4. **JIT surface.** Position before `invoke_jit` is the canonical entry
   ip, guarded by `is_function_entry` (core.rs:656-700); JIT yield and
   deopt leave that entry position (core.rs:875-912 — deopt restarts the
   function); compiled-frame stack entries carry an EXPLICIT mfa with
   placeholder ip 0 and NIL location (jit/ir_exceptions.rs:329-335, the
   F1/F2-fixed shape) — never resolved through the window tables on the
   fixed path.
5. **Transient pass-through.** After a prologue `Label` executes
   (Continue), position = Line ip momentarily — but only mid-slice under
   the executing worker's exclusive `&mut Process` borrow
   (run_loop signature). No suspension point can fire there (reduction
   charge exists only at jumps; Label/Line don't jump) and no concurrent
   reader exists during a slice.

**Tier 2 — the public ReplayDebugger single-step surface: the window ip
IS exposable, display-grade.** `ReplayDebugger::step_forward` executes
one instruction and exposes the raw `code_position()`
(replay/debugger.rs:198-230): stepping over a prologue Label parks the
exposed position at the Line ip. The debugger itself resolves only
FRAME return-ips through `function_at_ip` (debugger.rs:326), never the
current position — the mis-attribution fires only if an API consumer
takes the exposed ip and resolves it through `mfa_at_ip`/`line_at_ip`
themselves, and yields a wrong function/line DISPLAY, not a wrong
execution. In-estate the debugger has zero production callers
(re-exported at replay/mod.rs:16 only; confirmed unchanged since the
#48 walk).

## Consequence

The F3 fix stays UNMADE, now on measured ground instead of suspicion:
no crash report, yield, wait, frame, or current_mfa can mis-attribute
through the window today. The duty travels at the pin site (doc
amendment on `Module::mfa_at_ip`, this lane's only tree change): whoever
(a) wires the ReplayDebugger into a surface that resolves the current
position through the window tables, (b) adds a suspension point that is
not a jump target (e.g., per-instruction reduction charging), (c) adds a
concurrent reader of a running process's position, or (d) admits
non-loader modules with populated function_tables and non-canonical
prologues — must re-derive F3 reachability and, if reached, make the
receipt's honest fix (region starts at the prologue) rather than patch
the observer.

## Battery (canon 8-leg, 2026-08-15 09:34–09:41Z)

Runner: gate-logs/103/battery-RUNNER.sh byte-identical; stdout to its own
file (the #78 capture-clip cannot recur — full header survived: pin-pre,
tree-pre 1 modified, OPEN line all present). Prediction pre-registered in
PREDICTION-91.md BEFORE launch and BEFORE the pre-battery check output
was read. Pre-battery check per the #78 law: BOTH clippy legs' FULL
commands re-run AFTER the last edit (the doc amendment) — rc 0 both.

RESULT: GREEN — all 8 legs rc 0 AT THE PER-LEG TSV (the verdict; the
marker agrees but is not it). Measured axes vs the registration, exact:

| leg                | predicted         | measured          |
|--------------------|-------------------|-------------------|
| tests              | 75 / 2116 / 0 / 0 | 75 / 2116 / 0 / 0 |
| tests-all-features | 75 / 2126 / 0 / 0 | 75 / 2126 / 0 / 0 |

Pin 70f61f776335b264e756184a6094ed7fc4cdf79f stable pre/post. Tree
census 1/1 pre/post = exactly the declared single path
(crates/beamr/src/module.rs, the doc amendment), verified by porcelain
at close. Battery logs: BATTERY.log (runner stdout), legs.tsv, leg5.log
+ leg8.log (axes witnesses).
