# CRASH-2 SPECIMEN READ 1 — the 0.18.2 re-soak death localized at the JIT trampoline exception seam

Artemis Peach. Ground: `~/.aion-196-soak/` (Waffles' pointer, read at my hands
2026-08-16) against beamr bytes at v0.18.2 `74e532f` / main `1965650` (the
files cited below are byte-identical across that span — probe-drift check
came back empty for jit/ and interpreter/). Task #107 Strand A. Everything
marked MEASURED below is read directly from the soak record or the beamr
bytes; the funcid naming is marked INFERENCE and its discharge instrument is
named.

## The timeline (MEASURED, three independent files agree)

- T0 = 13:13:04 (00-MANIFEST). Workload cadence: one activity pair per ~2
  minutes, 196 activities completed (06-TERMINAL-RECORD, 593 events).
- FOUR JIT compilations in the entire soak (01-server.log, cranelift
  "defining function"): funcid44 @ 14:12:01, funcid45 @ 14:44:28,
  **funcid46+47 @ 16:29:35.964/.965** — 30 ms after activity 194's
  completion (16:29:35.934), i.e. submitted from the call-miss edge during
  that envelope's processing, compiled on the pool while the workflow sat in
  a 120 s `pace`.
- Death: WorkflowFailed **16:31:36.147** (seq 593) — 63 ms after scheduling
  activity 196 (16:31:36.084), on the FIRST dispatch cycle whose encode work
  could hit the fresh cache entries. T0+3h18m31s exactly.
- The early 13:15:16 ERRORs are the deliberately-failed ARM-CONTROL workflow
  (.failpoll.json, display_name 196-ALARM-FAILED-ARM-CONTROL) — controls,
  not anomalies.

## The face (MEASURED, 07-FAILURE-REASON)

`badarg` raised at `gleam_json_ffi:int/1 line 54` (interpreted frame, line
present) under `activity_dispatch:await/1`'s thunk → `pump:run/1` →
`step_poll/12` → `runtime:run/4` — the workflow was JSON-encoding an integer
during activity await/dispatch. `gleam_json_ffi`'s `int` is
`integer_to_binary/1`-shaped: badarg means THE VALUE WAS NOT AN INTEGER at
read time. Small ints are immediates and cannot go stale; the corrupted read
is therefore of a HEAP slot (closure env / container) that was believed to
hold an int.

## ★ THE ANOMALY (MEASURED, counted twice — visual + grep -c = 7)

The trace's last seven entries are **seven identical, line-less
`gleam@json…:int/1` frames BELOW the workflow entry point
(`runtime:run/4`)**. `int/1` does not recurse; seven stacked copies below
the entry frame are not a possible genuine call chain.

At the beamr bytes, exactly one producer writes name-resolved
(`mfa: Some(..)`, `compiled: true`) entries: `jit_add_compiled_frame`
(jit/ir_exceptions.rs:307) — appends to the process-persistent
`raw_stacktrace`, called ONLY from a compiled function's
exception-propagation arm. The four soak CLIFs each contain exactly one such
call. So the seven entries are seven executions of a compiled exception arm
whose constants are (module-atom, fn-atom 534, arity 1) — recorded into a
trace whose interpreted portion was captured once (capture_raw_stacktrace,
exceptions.rs:268, REPLACES the vec, sole caller = raise_exception :180).

**Open mechanism question (the discriminating experiment):** one raise
propagating through one trampoline pushes ONE frame; seven pushes with no
intervening capture-replace require either (a) an exception path that raises
WITHOUT passing capture_raw_stacktrace (candidate: BIF/ExecError channel —
`take_jit_exec_error` in opcodes/core.rs:888 bypasses raise_exception), or
(b) catch shapes that consume exceptions without reaching
try_end/catch_end's clear (raise_exception's handler arm does NOT clear;
only the later try_end/catch_end opcodes do), or (c) a repeated
invoke→raise→catch loop upstream. A minimal erlc fixture repro (cross-module
int-encode chain, tier the wrapper up, badarg after N successful calls,
inspect the rendered trace) discriminates these at the bytes.

## Funcid identification (INFERENCE — discharge = atom-decode instrument)

Decoded against the JIT import table (declaration order,
compiler/ir_helpers.rs): fn5 = charge_reduction, fn6 =
`jit_call_interpreted(process, module_atom, import_index, arity, args)`
(runtime.rs:79), fn11 = `jit_add_compiled_frame(process, module_atom,
fn_atom, arity)` (ir_exceptions.rs:307). The four compiled functions are
body-call trampolines:

| funcid | module atom | calls import | exception-arm mfa | shape |
|---|---|---|---|---|
| 44 | 1369 | 5 | (1369, 268, 1) | trampoline |
| 45 | — | — | — | pure identity (charge → return arg) |
| 46 | 1359 | 7 | (1359, 534, 1) | trampoline |
| 47 | 1369 | 3 | (1369, 534, 1) | trampoline |

Modules 1369 and 1359 each own an arity-1 function with the SAME name-atom
534; the death trace names exactly two modules with a shared `int/1`:
`gleam@json` and `gleam_json_ffi`. Reading: funcid47 = `gleam@json:int/1`
(body-calls its import = ffi int), funcid46 = `gleam_json_ffi:int/1`
(body-calls `erlang:integer_to_binary/1`), funcid45 = the ffi's identity
(`json_to_iodata`-shaped), and the rendered ghosts' (1369, 534) = funcid47's
constants exactly. Atom-table decode of the bundle (aion-data/, archaeology
instrument, board word granted) turns this inference into a measurement.

## What this does to the strand-A ranking

- The death is LOCALIZED: first native dispatch of freshly-compiled
  `json.int` trampolines → seven compiled-exception-arm executions → badarg
  on a heap slot read as a non-integer — all within one encode window. The
  compile→first-invoke→death coupling is now MEASURED on this specimen (2 m
  01 s compile-to-death, one pace cycle).
- **AR-1 no longer looks like the leading mechanism for crash-2.** The hot
  seam is the jit body-call trampoline exception path
  (jit_call_interpreted / add_compiled_frame / ExecError channel), which is
  present and open in BOTH v0.17.0 and v0.18.2 — it survives the
  version-containment filter alongside AR-1, and unlike AR-1 it now has
  specimen evidence sitting directly on it. Finding 1's "AR-1 = only named
  class open in both" was scoped to NAMED classes; this seam was unnamed.
  The AR-1 remedy proceeds on its own rulings regardless.
- The seven ghost frames are themselves a reportable beamr defect candidate
  (trace integrity at the compiled boundary) independent of whether they
  share a mechanism with the badarg.

## Next instruments (in order)

1. Repro fixture: tiered-up cross-module trampoline + badarg — reproduce or
   refute the 7-ghost shape and the ExecError-bypass hypothesis. RED test if
   reproduced.
2. Atom-decode archaeology on aion-data/ — discharge the naming inference,
   also names funcid44's fn-atom 268 and the import indices 3/5/7.
3. 0.17.0-death record side-by-side once its trace is at hand (funcids 44-47
   banked as the same set there — same workload, same tier-ups).
