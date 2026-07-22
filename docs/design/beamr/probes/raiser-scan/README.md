# raiser-scan — no-match raiser census + probe fixtures (0.16.0 release-check instrument)

Rerunnable instrument for the Arc-2 enablement release (~0.16.0), which ships the
interpreter's no-match raiser corrections (`func_info` raises catchable
`error:function_clause` instead of looping; `if_end` raises the bare atom
`if_clause` instead of `{if_clause, []}`). Two halves:

## 1. `scan_raisers.erl` — the census

For every function in the given .beam files, flags (a) **refutable heads** —
instructions referencing the function's func_info prelude label, i.e. head
dispatch that can fail to the `function_clause` landing pad — and (b) **in-body
no-match raisers** (`case_end` / `badmatch` / `if_end` / `try_case_end`).
Functions with neither are omitted.

Run (any OTP with `beam_disasm`, OTP-29 used for the recorded runs):

```sh
erlc scan_raisers.erl
erl -noshell -run scan_raisers main path/to/*.beam
```

Recorded results at the 0.16.0 gate (2026-07-23, lane `jit/admission-leg1`):

- **frame's 4 production beams: ZERO refutable heads** — structural, not
  incidental: gleam codegen emits irrefutable variable heads, and the shim's
  dispatch happens inside a receive.
- **Arc-2 16-module closure (`crates/beamr/tests/fixtures/gleam_otp_spike/beams/`):
  13 refutable-headed functions, all in the 3 handwritten Erlang modules**
  (gleam_erlang_ffi — 8, including the `'receive'/1,2` + `select/2` hot paths —
  gleam_stdlib, gleam_otp_external).
- **Exactly ONE `if_end` in the whole closure**: gleam_stdlib
  `percent_decode/2`.

## 2. Probe fixtures — raise-shape checks through beamr-cli

Minimal OTP-29 erlc modules (committed .erl + .beam), each raising one no-match
class; `main_catch/0` (where present) proves the raise is catchable with the
standard pattern, and `id/1` defeats constant folding:

| fixture | raises | entry points |
|---|---|---|
| `fc_probe` | `error:function_clause` (bare atom) | `main/0` |
| `case_probe` | `error:{case_clause, b}` | `main/0`, `main_catch/0` |
| `badmatch_probe` | `error:{badmatch, b}` | `main/0`, `main_catch/0` |
| `if_probe` | `error:if_clause` (bare atom) | `main/0`, `main_catch/0` |

Run each against the release candidate and against OTP-29 `erl` as the
differential oracle:

```sh
beamr fc_probe.beam --entry "fc_probe:main/0"          # error:function_clause, no hang
beamr if_probe.beam --entry "if_probe:main_catch/0"    # returns caught
erl -noshell -eval 'R = (catch if_probe:main_catch()), io:format("~p~n", [R]), halt().'
```

Release gate: every probe's uncaught raise shape and every `main_catch/0` result
must match the OTP-29 differential. Pre-0.16.0 mains fail two of these:
`fc_probe:main/0` spins forever (func_info set-MFA-and-continue) and
`if_probe:main_catch/0` lets the exception escape (`{if_clause, []}` 2-tuple
does not match the standard `error:if_clause` pattern).

The e2e regression walls for both defects live in the test suite
(`function_clause_e2e.rs`, `if_clause_e2e.rs`); this instrument is the
human-rerunnable census + differential for release checks against consumer
beams that are not test fixtures.
