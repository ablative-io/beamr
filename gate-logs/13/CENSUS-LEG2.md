# CENSUS — REAL-ERLC-ADMISSION Leg 2 precondition: rejection telemetry, corpus-ranked

Lane: #13 (A2 coverage arc). The scoping brief
(docs/design/beamr/briefs/REAL-ERLC-ADMISSION-SCOPING.md §4) makes Leg 2
(JIT-003 sub-op widening) dispatchable only on a corpus ranking: "run real
module corpora through the demand path and rank
`UnsupportedOpcode`/`UnsupportedOperand` rejections by frequency; JIT-003's
brief takes the top of that list … the corpus decides, not the guess."
This is that ranking. Measured 2026-08-15 at beamr main `c9a63c6`, my
verification at three independent legs (below).

## Instrument

`AotCompiler::compile_module` (jit/aot.rs) over every exported function —
the same slicer + pre-pass + Cranelift lowering the demand path uses; the
demand path differs only in WHICH functions it compiles (hot ones), so
per-function enumeration is the right denominator for a coverage ranking.
Instrument source committed as census-instrument.rs (built and run OUTSIDE
the tree — zero tree delta; no stderr suppression; every-file-accounted
assert; positive + negative controls both fired: 485 fixture functions
compiled, 711 skipped).

Corpora: (a) FIXTURES — crates/beamr/tests/fixtures/*.beam, 79 modules of
committed real-erlc output; (b) OTP_STDLIB — all 35 OTP 29.0.3 application
ebins, 1,320 modules. No cap applied.

## Headline

| corpus | modules | loaded | load-err | exports | compiled | skipped |
|---|---:|---:|---:|---:|---:|---:|
| FIXTURES | 79 | 79 | 0 | 1,196 | 485 | 711 |
| OTP_STDLIB | 1,320 | 1,306 | 14 | 44,598 | 23,137 | 21,461 |
| combined | 1,399 | 1,385 | 14 | 45,794 | 23,622 | 22,172 |

51.6% of real exported functions compile today. Only 1 loaded module
compiles nothing; 25 are fully green. Module-fatal JIT errors: 0.

## The ranking (mechanism-level; full table in census-ranking.md)

| # | mechanism | fns | mods | share |
|---:|---|---:|---:|---:|
| 1 | body-call family (`Call`/`CallExt`/`Apply`/`CallFun` in body position — "no body-call model", ir_control.rs:700 R3 wall) | 10,112 | 1,116 | 45.6% |
| 2 | `unknown JIT label: N` (slicer label reach, compiler.rs:58) | 3,806 | 661 | 17.2% |
| 3 | `unsupported JIT operand: Literal(N)` (compiler.rs:56) | 3,592 | 946 | 16.2% |
| 4 | `TypeTest(IsNonemptyList)` | 872 | 332 | 3.9% |
| 5 | `SelectTupleArity` | 476 | 268 | 2.1% |
| 6 | `Bif(Bif1)` | 430 | 210 | 1.9% |
| 7 | `TypeTest(IsMap)` | 340 | 111 | 1.5% |
| 8 | frame slot count `Allocation(…)` operand | 294 | 73 | 1.3% |
| 9 | `Bif(Bif0)` | 294 | 141 | 1.3% |
| 10 | `Bif(GcBif1)` | 257 | 71 | 1.2% |

Tail: TypeTest(IsNil) 197 · TypeTest(IsBoolean) 182 · arithmetic-import
operand 168 · Catch 162 · Badrecord 159 · TypeTest(IsFloat) 152 (9 mods) ·
UpdateRecord 117 · exception terminals marginal (CaseEnd 39, IfEnd 13,
Badmatch 11). 39 distinct mechanisms total.

## What the corpus decided (vs the guess)

The JIT-002 ground-pack front-runner guess — TypeTest sub-ops and
exception terminals — is REFUTED as the top: the whole TypeTest family is
9.2% and the exception terminals are under 2% combined. The actual top of
the list is **Leg 3's declared surface, not Leg 2's**: the body-call
continuation model alone is 45.6% of every rejection, touching 1,116 of
1,385 loaded modules. Ranks 2 (label reach — slicer territory, the
Leg-1-class successor; attribution of the residue is OPEN, plausibly the
out-of-slice continuation labels of the same body calls) and 3 (Literal
operand lowering — the broadest single mechanism by module count, 946
modules) sit between the legs as scoped. The JIT-003 band as originally
scoped (sub-ops: TypeTest, Bif slots, SelectTupleArity, frame-slot and
arithmetic operand forms) is ranks 4–15 — under 12% of rejections.

Sequencing consequence: §4's "JIT-003's brief takes the top of that list"
cannot be satisfied by a sub-op brief — the top of the list IS the Leg 3
centerpiece. That is a sequencing ruling for the seats that ruled the arc,
not a call this census makes. Routed accordingly.

## Separate finding: loader coverage (NOT JIT rejections)

14 OTP modules fail to LOAD at all, 3 verbatim reasons:
`unsupported opcode 186` ×6 (dbg_ieval, dialyzer_codeserver, erl_types,
erl_eval, io_lib, io_lib_pretty); `unsupported opcode 185` ×4 (inet_db,
calendar, io_lib_fread, rand); `unsupported ETF literal tag 77` ×4
(3× megaco_per_media_gateway_control_v*, PKIXAttributeCertificate-2009).
`calendar`, `rand`, `erl_eval`, `io_lib` are not exotic modules. Filed as
defect candidates with the routing, not opened as lanes.

## Verification (worker report treated as claim)

Worker ran 3× byte-identical; I re-ran the binary MYSELF — raw + modules
TSVs byte-identical to the worker's (sha256 match); I recounted the
mechanism ranking INDEPENDENTLY from the raw TSV with my own bucketing
rules — every headline number reproduced exactly (22,172 skipped; 10,112 =
45.6% body-call; 3,806 label; 3,592/946 Literal); each top-3 reason string
traced to its raising site in jit/ at the bytes. The worker's mtime flag
on loader/encode/** resolved: those are my own #95 edits (02:00–02:09,
committed as 7e318c0) — git status clean, zero drift, the arc's
zero-encode-bytes boundary untouched.

Scratchpad artifacts (sha256, raw TSV too large to commit):
- jit-census-raw.tsv (45,794 rows, 8.0 MB): `4f864eda…49c5db`
- jit-census-modules.tsv: `2bbcff6e…16a78` (committed here)
- jit-census-ranking.md: `a0a718d4…8729a` (committed here)
- instrument main.rs: `4ece7076…04a8c0` (committed here)
