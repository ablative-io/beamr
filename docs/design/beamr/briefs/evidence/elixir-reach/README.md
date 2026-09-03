# elixir-reach — how much of Elixir does beamr not run: the count, by tier

**Pins (every table below carries them):** `Elixir 1.20.4 (otp-29 build) · OTP 29.0.5 · beamr 43d87819ed37515695a697b7416d47c6740502e2`
(base = `origin/main` at 2026-09-03T03:02Z; probe on lane `elixir-reach-20260903` at `dc26cec76e21ebc13b3769a5d19097918d89650e`).
Venue: `aion-workflows` (192.168.50.205), run at Artemis Peach's hands over ssh on 2026-09-03 between 03:03Z and the time in the
last section; no aion workflow, no venue cap consumed. Brief: `docs` `briefs/beamr-elixir-reach.md` @ `bbeecda` (APPROVED).
Derived counts re-derived at the base: **enum variants 75 · decoder arms 127 (mapping 129 distinct opcode numbers, max 184) ·
executed variants 73** (complement `Badrecord`, `NifStart`). Every count names its command in `COMMANDS.md` by number (C1–C15).
**No sentence in this report estimates effort, ranks difficulty, or recommends.**

## What a row means

`load_status` — `loaded`: beamr's loader decoded every instruction and produced a module; `decode-failed`: the loader refused the
module at the first opcode number it does not map (`crates/beamr/src/loader/decode/code.rs:427-431`), `first_refused_opcode` is that
number; `load-failed`: refused for another reason (`load_error` quotes it). For a module that is not `loaded`, axes C and D read
`unmeasurable: load`. Axis A's generic names decode and then, at execute, `set_tuple_element` (67) and `debug_line` (184) raise
`UnknownOpcode` while `executable_line` (183) is a no-op (`opcodes/mod.rs:385-389`, `:450`). Axis C's `unresolved` are external
calls with neither a native nor a loaded module under the 41 `--dir`s; `deferred` are calls into a module that IS loaded and
resolve at call time; `denied` are policy refusals. Two instruments per axis where the brief names them; every disagreement is a
row in *Disagreements*, none averaged.

## Controls (C9, C10, C13, C14) — run and agreeing before any tier was trusted

| # | control | expectation (brief R5) | measured | verdict |
|---|---|---|---|---|
| 1 | `stdlib-8.0.3/ebin/supervisor.beam`, 41 dirs | load recorded; C `unresolved + deferred` non-zero | `loaded`; unresolved 0, **deferred 2**, denied 0; CLI stream equal | holds |
| 2 | `wport9_conformance.beam`, direct CLI, runnable seven | rc 0 each; probe loaded, A all zero, B zero | `wake_send`, `wake_receive_timeout`, `wake_timer_deadline`, `bif_supported`, `bif_unsupported`, `output_entry` **rc 0** with expected values; **`wake_cast` rc 124** (120 s timeout: it parks in `receive` awaiting the driver's cast, source `:50-51`) — excluded BY MEASUREMENT; probe `loaded`, 284 instructions, generic 0/0/0, B 0 | 6 of 7 hold; one refuted |
| 2b | `process_error/0` | recorded, not asserted; C names `wport9_missing_module:missing_fn/0` as unresolved, calls 1 | rc 1 `beamr: undefined function wport9_missing_module:missing_fn/0`; the MFA lands in **`deferred`** (6 deferred, 0 unresolved), calls 1 | **refuted as written**: a call into an absent module is classified deferred, not unresolved; recorded, unchanged |
| 3 | planted opcode **191** (`get_record_field/5`, real OTP 29) by byte-patch of `main/0`'s first `allocate` | probe `decode-failed 191`; CLI quotes `unsupported opcode 191`; disassembler names it | probe `decode-failed`, `first_refused_opcode 191`; `beamr imports` rc 2 `load: failed to decode BEAM data: unsupported opcode 191`; walker names `get_record_field/5`; `beam_disasm` parses (as `{test,get_record_field,…}`) | holds |
| 3b | planted opcode **200** (unknown to OTP) | both instruments refuse | probe `decode-failed 200`; CLI `unsupported opcode 200`; walker `refused 200 at offset 10`; `beam_disasm` `{'EXIT',{beam_disasm,363,badarg}}` | holds |
| 4 | `erlc +line_coverage` (opcode 183) | loaded; `executable_line` count = disassembler's; entry rc 0 | probe `loaded`, `executable_line` **1** = walker `op 183 … 1`; `beamr … --entry reach_lc:main/0 --dir stdlib ebin` **rc 0** `{ok, 12}` (without `--dir`: rc 1 `undefined function lists:sum/1` — the call, not the opcode) | holds |
| 5 | `erlc +beam_debug_info` (opcode 184) | loaded; `debug_line` count = disassembler's; entry raises `UnknownOpcode 184` | probe `loaded`, `debug_line` **2** = walker `op 184 … 2`; entry rc 1 **`beamr: unknown opcode 184`** | holds |
| 6 | fabricated `erlang:no_such_bif_zzz/0` | C unresolved names it, calls 1; CLI stream same | unresolved `erlang:no_such_bif_zzz/0` ×1; CLI prints the same line, rc 0 | holds |

The `.S` route for control 3 was tried first and is recorded: `erlc reach_op191.S` refuses an instruction the assembler's validator
does not know (`unknown_instruction`), so the planted module is a byte patch (C10), the patched offset chosen from the walker's own
offset listing (`REACH_OFFSETS=1`).

**Instrument self-test (C7).** The OTP-side walker (`tools/reach_otp.escript`) walks the raw `Code` chunk with
`beam_opcodes:opname/1` and skips operands per `beam_asm.erl:726-857`. Its first sweep false-refused **10 of 202** stdlib+kernel
modules: extended-tag subtag 2 was read as an 8-byte float where OTP 29 encodes `{fr,N}` (a float register followed by one unsigned
term, `beam_asm.erl:786-787`). Fixed; the full sweep over all 1320 OTP modules and the 271 T2 modules is in
`transcripts/venue-sweep.log` and `sweep-maxop.txt`. Deviation from the brief's wording: the brief said `beam_disasm` names →
`beam_opcodes:opcode/2`; `beam_disasm`'s compound instruction forms (`test`, `bif`, `gc_bif`, …) do not map one-to-one to opcode
numbers, so the walker reads the numbers directly from the chunk with the same OTP table. `beam_disasm` is still used for `call_ext*`
call counts and `beam_lib` for `ImpT`.

**Full sweep (C7, after the fix):** all **1320 of 1320** OTP modules and **271 of 271** T2 modules walk `ok`. Across OTP,
**10 modules** carry an opcode above beamr's mapped ceiling of 184 — 4 with `185` and 6 with `186` — the same ten the probe
skips in the 41-dir preload (cross-instrument agreement, below). Across T2: **zero** modules carry an opcode above 184 and
**zero** carry 183 or 184 (`transcripts/venue-sweep.log`, `sweep-maxop.txt`). The `Code` chunk header's opcode field (`beam_dict:highest_opcode`,
`beam_asm.erl:150`) is never below the walker's highest opcode seen (0 of 1320 modules); it is not a per-module maximum
(it reads 181 on 1310 modules and equals the walker's max on 544), so the consistency check is the one-sided bound only.

**Native census (C4 vs C5).** The registry's own `registered_mfas()` (probe `--natives`) lists **330 MFAs across 31 modules**,
identical on the Mac and on the venue: `erlang` 178, `ets` 23, `lists` 21, `maps` 17, `string` 14, `gleam_erlang_ffi` 10, `pg` 9,
`io` 6, `meridian_ffi` 6, `json` 5, `math` 5, `os` 5, `gleam_stdlib` 4, `global` 4, `binary` 3, `base64` 2, `code` 2, `unicode` 2,
`uri_string` 2, and one each of `application`, `gleam_otp_external`, `init`, `io_lib`, `io_lib_format`, `logger`, `net_kernel`,
`proc_lib`, `rand`, `supervisor`, `sys`, `timer`. A static grep of the source tables (C5) found 26 modules — it misses five whose
tables have another shape — so axis D's `native-stub` status is taken from the registry, not the grep.

## The 41 `--dir`s: what does not load (same for every probe invocation)

Every tier's probe pre-loads the 6 Elixir ebin dirs (`elixir`, `iex`, `mix`, `logger`, `eex`, `ex_unit`) and all 35 OTP ebin dirs
(≈1900 modules) with the CLI's warn-and-skip policy. **14 modules are skipped, all OTP; every Elixir module in the six dirs loads.**

| refusal | modules |
|---|---|
| `unsupported opcode 185` (`bif3/6`) | `stdlib`: `calendar`, `io_lib_fread`, `rand`; `kernel`: `inet_db` |
| `unsupported opcode 186` (`is_any_native_record/2`) | `stdlib`: `erl_eval`, `io_lib`, `io_lib_pretty`; `dialyzer`: `dialyzer_codeserver`, `erl_types`; `debugger`: `dbg_ieval` |
| `unsupported ETF literal tag 77` (a load-failed class the brief did not anticipate) | `megaco`: `megaco_per_media_gateway_control_v{1,2,3}`; `public_key`: `PKIXAttributeCertificate-2009` |

(From `out/T1/raw/Elixir.T1Hello.probe.err`; identical in every `*.probe.err`.)

## T1 execute (C14) — the six modules, run under `beamr` with all 41 dirs

| module | source | rc | output / error |
|---|---|---|---|
| `Elixir.T1Hello` | `t1/t1_hello.ex` | 0 | `hello from beamr` → `ok` |
| `Elixir.T1EnumMap` | `t1/t1_enum_map.ex` | 0 | `4` |
| `Elixir.T1Struct` | `t1/t1_struct.ex` | 0 | `{big, 3}` |
| `Elixir.T1String` | `t1/t1_string.ex` | **134** | `thread 'beamr-sched-0' has overflowed its stack` / `fatal runtime error: stack overflow, aborting` |
| `Elixir.T1Process` | `t1/t1_process.ex` | **1** | `beamr: error: badarg` at `Elixir.T1Process:main/0 line 4` (`spawn(fn -> send(parent, …) end)`) |
| `Elixir.T1Task` | `t1/t1_task.ex` | **1** | `beamr: bad argument` (`Task.async`/`Task.await`) |

All six `loaded`; none carries a generic opcode; with the 41 dirs none has an unresolved import. The three failures are at execute.

## T1 — 6 modules · load_status: loaded 6

**first_refused_opcode histogram (T1)** — modules whose load beamr refuses, by the first opcode it refuses:

| opcode | OTP 29 name | modules |
|---|---|---|
| — | — | 0 (every module in this tier decodes) |

| modules | decode-failed | load-failed | loaded | instructions (loaded) | A generic: 67 / 183 / 184 occurrences (modules) | B (complement) | C unresolved: distinct MFAs / total calls / modules with ≥1 | C unmeasurable | deferred (sum) | denied (sum) | CLI stream ≠ probe |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 6 | 0 | 0 | 6 | 410 | 0 (0) / 0 (0) / 0 (0) | 0 by construction | 0 / 0 / 0 | 0 | 0 | 0 | 0 |

**C — unresolved natives by target module (T1, top 20 by calls)**: none

**D — OTP behaviour/facility modules reached (T1, module → importing modules, beamr status)**: `erlang` 6 [native-stub]

**Disagreements (T1)**: (none)

## T2 — 271 modules · load_status: loaded 271

**first_refused_opcode histogram (T2)** — modules whose load beamr refuses, by the first opcode it refuses:

| opcode | OTP 29 name | modules |
|---|---|---|
| — | — | 0 (every module in this tier decodes) |

| modules | decode-failed | load-failed | loaded | instructions (loaded) | A generic: 67 / 183 / 184 occurrences (modules) | B (complement) | C unresolved: distinct MFAs / total calls / modules with ≥1 | C unmeasurable | deferred (sum) | denied (sum) | CLI stream ≠ probe |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 271 | 0 | 0 | 271 | 339479 | 0 (0) / 0 (0) / 0 (0) | 0 by construction | 0 / 0 / 0 | 0 | 23 | 0 | 0 |

**C — unresolved natives by target module (T2, top 20 by calls)**: none

**D — OTP behaviour/facility modules reached (T2, module → importing modules, beamr status)**: `erlang` 271 [native-stub]; `lists` 98 [native-stub]; `maps` 49 [native-stub]; `io_lib` 27 [native-stub]; `code` 21 [native-stub]; `binary` 20 [native-stub]; `file` 18 [loaded-bytecode]; `ets` 16 [native-stub]; `unicode` 11 [native-stub]; `string` 10 [native-stub]; `erl_anno` 10 [loaded-bytecode]; `filename` 10 [loaded-bytecode]; `application` 8 [native-stub]; `ordsets` 7 [loaded-bytecode]; `proplists` 7 [loaded-bytecode]; `os` 6 [native-stub]; `json` 6 [native-stub]; `beam_lib` 5 [loaded-bytecode]; `proc_lib` 5 [native-stub]; `gen_server` 5 [loaded-bytecode]; `io` 5 [native-stub]; `init` 4 [native-stub]; `math` 4 [native-stub]; `erl_eval` 4 [absent]; `re` 4 [loaded-bytecode]; `erl_internal` 4 [loaded-bytecode]; `logger` 3 [native-stub]; `unicode_util` 3 [loaded-bytecode]; `sets` 3 [loaded-bytecode]; `erpc` 3 [loaded-bytecode]; `gen` 3 [loaded-bytecode]; `orddict` 3 [loaded-bytecode]; `persistent_term` 3 [loaded-bytecode]; `gen_event` 2 [loaded-bytecode]; `global` 2 [native-stub]; `filelib` 2 [loaded-bytecode]; `net_kernel` 2 [native-stub]; `timer` 2 [native-stub]; `otp_internal` 2 [loaded-bytecode]; `supervisor` 2 [native-stub]

**Disagreements (T2)**: (none)

## T3 — 142 modules · load_status: decode-failed 6, loaded 136

**first_refused_opcode histogram (T3)** — modules whose load beamr refuses, by the first opcode it refuses:

| opcode | OTP 29 name | modules |
|---|---|---|
| 185 | `bif3/6` | 3 |
| 186 | `is_any_native_record/2` | 3 |

| modules | decode-failed | load-failed | loaded | instructions (loaded) | A generic: 67 / 183 / 184 occurrences (modules) | B (complement) | C unresolved: distinct MFAs / total calls / modules with ≥1 | C unmeasurable | deferred (sum) | denied (sum) | CLI stream ≠ probe |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 142 | 6 | 0 | 136 | 508984 | 0 (0) / 0 (0) / 0 (0) | 0 by construction | 0 / 0 / 0 | 6 | 145 | 0 | 0 |

**C — unresolved natives by target module (T3, top 20 by calls)**: none

**D — OTP behaviour/facility modules reached (T3, module → importing modules, beamr status)**: `erlang` 136 [native-stub]; `lists` 110 [native-stub]; `maps` 57 [native-stub]; `io_lib` 52 [native-stub]; `io` 35 [native-stub]; `unicode` 33 [native-stub]; `file` 30 [loaded-bytecode]; `string` 30 [native-stub]; `proplists` 28 [loaded-bytecode]; `error_logger` 25 [loaded-bytecode]; `logger` 25 [native-stub]; `erts_internal` 25 [loaded-bytecode]; `filename` 22 [loaded-bytecode]; `application` 21 [native-stub]; `os` 20 [native-stub]; `ets` 19 [native-stub]; `gen_server` 18 [loaded-bytecode]; `erl_anno` 17 [loaded-bytecode]; `init` 17 [native-stub]; `erl_parse` 16 [loaded-bytecode]; `ordsets` 15 [loaded-bytecode]; `code` 14 [native-stub]; `erl_scan` 13 [loaded-bytecode]; `proc_lib` 13 [native-stub]; `binary` 13 [native-stub]; `persistent_term` 13 [loaded-bytecode]; `sets` 11 [loaded-bytecode]; `erl_internal` 11 [loaded-bytecode]; `erl_eval` 11 [absent]; `net_kernel` 10 [native-stub]; `epp` 9 [loaded-bytecode]; `cerl` 9 [loaded-bytecode]; `re` 9 [loaded-bytecode]; `filelib` 8 [loaded-bytecode]; `sys` 8 [native-stub]; `erl_prim_loader` 7 [loaded-bytecode]; `prim_file` 7 [loaded-bytecode]; `global` 7 [native-stub]; `supervisor` 7 [native-stub]; `logger_server` 7 [loaded-bytecode]

**Disagreements (T3)**: (none)

## T4 — 148 modules · load_status: loaded 148

**first_refused_opcode histogram (T4)** — modules whose load beamr refuses, by the first opcode it refuses:

| opcode | OTP 29 name | modules |
|---|---|---|
| — | — | 0 (every module in this tier decodes) |

| modules | decode-failed | load-failed | loaded | instructions (loaded) | A generic: 67 / 183 / 184 occurrences (modules) | B (complement) | C unresolved: distinct MFAs / total calls / modules with ≥1 | C unmeasurable | deferred (sum) | denied (sum) | CLI stream ≠ probe |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 148 | 0 | 0 | 148 | 156958 | 0 (0) / 0 (0) / 0 (0) | 0 by construction | 0 / 0 / 0 | 0 | 31 | 0 | 0 |

**C — unresolved natives by target module (T4, top 20 by calls)**: none

**D — OTP behaviour/facility modules reached (T4, module → importing modules, beamr status)**: `erlang` 148 [native-stub]; `lists` 58 [native-stub]; `code` 28 [native-stub]; `maps` 28 [native-stub]; `file` 18 [loaded-bytecode]; `io` 17 [native-stub]; `io_lib` 17 [native-stub]; `os` 14 [native-stub]; `ets` 12 [native-stub]; `binary` 10 [native-stub]; `proplists` 8 [loaded-bytecode]; `filename` 8 [loaded-bytecode]; `string` 8 [native-stub]; `beam_lib` 7 [loaded-bytecode]; `unicode` 7 [native-stub]; `application` 6 [native-stub]; `logger` 6 [native-stub]; `crypto` 6 [loaded-bytecode]; `erpc` 5 [loaded-bytecode]; `re` 5 [loaded-bytecode]; `compile` 4 [loaded-bytecode]; `erl_anno` 4 [loaded-bytecode]; `erl_parse` 4 [loaded-bytecode]; `erl_scan` 4 [loaded-bytecode]; `ordsets` 4 [loaded-bytecode]; `inet` 4 [loaded-bytecode]; `epp` 4 [loaded-bytecode]; `zlib` 3 [loaded-bytecode]; `gen_tcp` 3 [loaded-bytecode]; `filelib` 3 [loaded-bytecode]; `error_logger` 3 [loaded-bytecode]; `shell` 2 [loaded-bytecode]; `proc_lib` 2 [native-stub]; `queue` 2 [loaded-bytecode]; `timer` 2 [native-stub]; `persistent_term` 2 [loaded-bytecode]; `sofs` 2 [loaded-bytecode]; `gen_server` 2 [loaded-bytecode]; `uri_string` 2 [native-stub]; `user_drv` 1 [loaded-bytecode]

**Disagreements (T4)**: (none)

## Axis C read correctly: what `unresolved 0` means, and the supplementary pass

The loader's rule (`crates/beamr/src/loader/load.rs:530-618`): an import with a registered native → resolved native (or
`denied` by capability); else, target module **not in the registry → `deferred`**; target module in the registry and the
export present → resolved code; target module in the registry but the **export absent → `unresolved`**. With the brief's
"every ebin dir in scope" as `--dir`, OTP's `erts-17.0.5/ebin` is loaded, and it carries **`erlang.beam`, `erts_internal.beam`,
`init.beam`, `prim_file.beam`, `prim_inet.beam`, …** — bytecode stand-ins whose exports cover the BIFs and NIFs the runtime
implements in C. Under that configuration every `erlang:*` call resolves to code, so **`unresolved 0` in every tier is a property
of the directory set, not evidence that the natives exist.** The `deferred` sums (T2 23 · T3 145 · T4 31) are calls into
modules that are NOT loaded — at this base those are the six T3 decode-failed modules and any module outside the 41 dirs.

The supplementary pass (`tools/cprime.sh`, `outC/`) re-runs the probe over every tier with `--dir` = the 41 minus the erts
ebin, so a call into `erlang`, `erts_internal`, `init`, `prim_*` etc. that has **no registered native lands in `deferred`
under the erts target module** — that list, by name with C6's call counts, is the missing-BIF/NIF census Tom asked for.
It is reported in the section *Supplementary C* below (absent if the pass had not finished when this commit was cut).

## Not measured (brief R3, each its own line; no zero is implied by absence)

- the Elixir bootstrap sequence (`elixir_config`, `elixir_code_server`, `Code.ensure_loaded`) as a RUN;
- correctness of the unicode tables (`unicode_util` data), as opposed to their reachability;
- the Elixir compiler executing under beamr (T4's real cost);
- ETS-backed module and protocol consolidation tables (`:elixir_config`, protocol impl lookup);
- process-dictionary semantics;
- `try`/`catch`/`raise` and stacktrace shape;
- map key ordering;
- bigint and float arithmetic semantics;
- T3's `ImpT` scan is blind to `apply/3`, `erlang:apply`, behaviour-callback and protocol dispatch: T3 is a LOWER BOUND.

## Files

`COMMANDS.md` (C1–C15) · `MANIFEST.tsv` (575 rows: tier, path, sha256 of file bytes, bytes; `.beam` files never committed) ·
`DIRS.txt` · `lists/` · `sets/` (`DECODER-SET.txt`, `EXECUTED-SET.txt`, `NATIVE-MFAS.txt`, `NATIVE-MODULES.tsv`) · `T3-REACH.tsv`
(142 rows with `depth`) · `t4-otp-beyond-t3.tsv` · `out/<tier>/<module>.json` (R4 schema) + `out/<tier>/raw/` (probe JSON+stderr+rc,
walker TSV, CLI stdout+stderr+rc per module) · `controls/`, `controls191/`, `t1/` (sources, `.S`) · `tools/` · `transcripts/`.
