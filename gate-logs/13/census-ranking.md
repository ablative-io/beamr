# JIT admission rejection census

REAL-ERLC-ADMISSION arc, §4 Leg 2. Every exported function of every module below was pushed through `beamr::jit::aot::AotCompiler::compile_module` — the same slicer, pre-pass and Cranelift lowering the demand path uses.

- FIXTURES root: `/Users/tom/Developer/ablative/stack/beamr/crates/beamr/tests/fixtures`
- OTP_STDLIB root: `/opt/homebrew/Cellar/erlang/29.0.3/lib/erlang/lib/*/ebin/*.beam` (all applications)
- Normalization: digit runs collapse to `N` unless preceded by a letter (so `utf8`/`int64` survive); whitespace runs collapse to one space.

**Self-tests:** positive control PASS, negative control PASS.

## 1. Headline

Total `.beam` files found: **1399**; module rows written: **1399** (conservation asserted equal).

| corpus | modules | loaded | load-error | module-fatal | exports | compiled | skipped | compiled % |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| FIXTURES | 79 | 79 | 0 | 0 | 1196 | 485 | 711 | 40.55% |
| OTP_STDLIB | 1320 | 1306 | 14 | 0 | 44598 | 23137 | 21461 | 51.88% |
| **COMBINED** | 1399 | 1385 | 14 | 0 | 45794 | 23622 | 22172 | 51.58% |

## 2. Rejection ranking, MECHANISM level (the dispatch list)

The `Debug` form of an instruction embeds its whole operand list, so a single mechanism (e.g. `SelectTupleArity`) fragments into one row per operand-list length under plain normalization. This table elides the operand payload — but keeps a bare tag payload (`TypeTest(IsNil)`, `Bif(Bif1)`), which IS the mechanism. Distinct mechanisms: **39**.

| # | mechanism | fns (all) | mods (all) | fns FIXTURES | mods FIXTURES | fns OTP | mods OTP | % of all skips | cumulative % |
|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | `unsupported JIT opcode: Call {…} in body position (not immediately followed by Return): the JIT tier has no body-call model` | 3887 | 635 | 44 | 21 | 3843 | 614 | 17.53% | 17.53% |
| 2 | `unknown JIT label: N` | 3806 | 661 | 83 | 26 | 3723 | 635 | 17.17% | 34.70% |
| 3 | `unsupported JIT operand: Literal(N)` | 3592 | 946 | 103 | 30 | 3489 | 916 | 16.20% | 50.90% |
| 4 | `unsupported JIT opcode: CallExt {…} in body position (not immediately followed by Return): the JIT tier has no body-call model` | 3189 | 724 | 175 | 38 | 3014 | 686 | 14.38% | 65.28% |
| 5 | `unsupported JIT opcode: Apply {…} in body position (not immediately followed by Return): the JIT tier has no body-call model` | 2990 | 239 | 0 | 0 | 2990 | 239 | 13.49% | 78.77% |
| 6 | `unsupported JIT opcode: TypeTest(IsNonemptyList)` | 872 | 332 | 62 | 12 | 810 | 320 | 3.93% | 82.70% |
| 7 | `unsupported JIT opcode: SelectTupleArity {…}` | 476 | 268 | 10 | 3 | 466 | 265 | 2.15% | 84.85% |
| 8 | `unsupported JIT opcode: Bif(Bif1)` | 430 | 210 | 5 | 3 | 425 | 207 | 1.94% | 86.79% |
| 9 | `unsupported JIT opcode: TypeTest(IsMap)` | 340 | 111 | 6 | 4 | 334 | 107 | 1.53% | 88.32% |
| 10 | `unsupported JIT opcode: Bif(Bif0)` | 294 | 141 | 7 | 3 | 287 | 138 | 1.33% | 89.64% |
| 11 | `unsupported JIT operand: frame slot count Allocation(…)` | 294 | 73 | 61 | 13 | 233 | 60 | 1.33% | 90.97% |
| 12 | `unsupported JIT opcode: Bif(GcBif1)` | 257 | 71 | 37 | 15 | 220 | 56 | 1.16% | 92.13% |
| 13 | `unsupported JIT opcode: TypeTest(IsNil)` | 197 | 160 | 2 | 2 | 195 | 158 | 0.89% | 93.02% |
| 14 | `unsupported JIT opcode: TypeTest(IsBoolean)` | 182 | 59 | 0 | 0 | 182 | 59 | 0.82% | 93.84% |
| 15 | `unsupported JIT operand: arithmetic import Unsigned(N)` | 168 | 88 | 31 | 11 | 137 | 77 | 0.76% | 94.60% |
| 16 | `unsupported JIT opcode: Catch {…}` | 162 | 79 | 0 | 0 | 162 | 79 | 0.73% | 95.33% |
| 17 | `unsupported JIT opcode: Badrecord {…}` | 159 | 27 | 0 | 0 | 159 | 27 | 0.72% | 96.04% |
| 18 | `unsupported JIT opcode: TypeTest(IsFloat)` | 152 | 9 | 4 | 2 | 148 | 7 | 0.69% | 96.73% |
| 19 | `unsupported JIT opcode: UpdateRecord {…}` | 117 | 44 | 0 | 0 | 117 | 44 | 0.53% | 97.26% |
| 20 | `unsupported JIT opcode: TypeTest(IsFunction2)` | 94 | 29 | 1 | 1 | 93 | 28 | 0.42% | 97.68% |
| 21 | `unsupported JIT opcode: TypeTest(IsPort)` | 84 | 10 | 1 | 1 | 83 | 9 | 0.38% | 98.06% |
| 22 | `unsupported JIT opcode: NifStart` | 78 | 8 | 0 | 0 | 78 | 8 | 0.35% | 98.41% |
| 23 | `unsupported JIT opcode: TypeTest(IsNumber)` | 61 | 15 | 0 | 0 | 61 | 15 | 0.28% | 98.69% |
| 24 | `unsupported JIT opcode: TypeTest(IsFunction)` | 56 | 28 | 0 | 0 | 56 | 28 | 0.25% | 98.94% |
| 25 | `unsupported JIT opcode: TypeTest(IsReference)` | 53 | 15 | 1 | 1 | 52 | 14 | 0.24% | 99.18% |
| 26 | `unsupported JIT opcode: runtime-deopt-capable Deallocate {…} is reachable after an observable side effect (a deopt would replay the effect on restart)` | 47 | 28 | 3 | 2 | 44 | 26 | 0.21% | 99.39% |
| 27 | `unsupported JIT opcode: CallFun {…} in body position (not immediately followed by Return): the JIT tier has no body-call model` | 46 | 27 | 29 | 14 | 17 | 13 | 0.21% | 99.60% |
| 28 | `unsupported JIT opcode: CaseEnd {…}` | 39 | 17 | 28 | 7 | 11 | 10 | 0.18% | 99.77% |
| 29 | `unsupported JIT opcode: IfEnd` | 13 | 5 | 1 | 1 | 12 | 4 | 0.06% | 99.83% |
| 30 | `unsupported JIT opcode: Badmatch {…}` | 11 | 9 | 3 | 1 | 8 | 8 | 0.05% | 99.88% |
| 31 | `unsupported JIT opcode: runtime-deopt-capable Allocate {…} is reachable after an observable side effect (a deopt would replay the effect on restart)` | 5 | 4 | 2 | 2 | 3 | 2 | 0.02% | 99.91% |
| 32 | `unsupported JIT operand: select_val candidate Literal(N)` | 4 | 2 | 4 | 2 | 0 | 0 | 0.02% | 99.92% |
| 33 | `unsupported JIT opcode: BinaryOp {…}` | 3 | 3 | 2 | 2 | 1 | 1 | 0.01% | 99.94% |
| 34 | `unsupported JIT opcode: runtime-deopt-capable CallExtLast {…} is reachable after an observable side effect (a deopt would replay the effect on restart)` | 3 | 3 | 0 | 0 | 3 | 3 | 0.01% | 99.95% |
| 35 | `unsupported JIT opcode: runtime-deopt-capable TestHeap {…} is reachable after an observable side effect (a deopt would replay the effect on restart)` | 3 | 3 | 0 | 0 | 3 | 3 | 0.01% | 99.96% |
| 36 | `unsupported JIT opcode: OnLoad` | 2 | 2 | 2 | 2 | 0 | 0 | 0.01% | 99.97% |
| 37 | `unsupported JIT opcode: TypeTest(IsBitstr)` | 2 | 2 | 2 | 2 | 0 | 0 | 0.01% | 99.98% |
| 38 | `unsupported JIT operand: invalid bs_create_bin segment operands` | 2 | 2 | 2 | 2 | 0 | 0 | 0.01% | 99.99% |
| 39 | `unsupported JIT opcode: runtime-deopt-capable FuncInfo {…} is reachable after an observable side effect (a deopt would replay the effect on restart)` | 2 | 1 | 0 | 0 | 2 | 1 | 0.01% | 100.00% |

### 2b. Mechanism → verbatim examples

| # | mechanism | verbatim examples (up to 3) |
|---:|---|---|
| 1 | `unsupported JIT opcode: Call {…} in body position (not immediately followed by Return): the JIT tier has no body-call model` | `unsupported JIT opcode: Call { arity: Unsigned(1), label: Label(46) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: Call { arity: Unsigned(1), label: Label(2) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: Call { arity: Unsigned(1), label: Label(8) } in body position (not immediately followed by Return): the JIT tier has no body-call model` |
| 2 | `unknown JIT label: N` | `unknown JIT label: 16`<br>`unknown JIT label: 62`<br>`unknown JIT label: 422` |
| 3 | `unsupported JIT operand: Literal(N)` | `unsupported JIT operand: Literal(3)`<br>`unsupported JIT operand: Literal(2)`<br>`unsupported JIT operand: Literal(1)` |
| 4 | `unsupported JIT opcode: CallExt {…} in body position (not immediately followed by Return): the JIT tier has no body-call model` | `unsupported JIT opcode: CallExt { arity: Unsigned(1), import: Unsigned(0) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: CallExt { arity: Unsigned(2), import: Unsigned(6) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: CallExt { arity: Unsigned(2), import: Unsigned(4) } in body position (not immediately followed by Return): the JIT tier has no body-call model` |
| 5 | `unsupported JIT opcode: Apply {…} in body position (not immediately followed by Return): the JIT tier has no body-call model` | `unsupported JIT opcode: Apply { arity: Unsigned(0) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: Apply { arity: Unsigned(1) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: Apply { arity: Unsigned(3) } in body position (not immediately followed by Return): the JIT tier has no body-call model` |
| 6 | `unsupported JIT opcode: TypeTest(IsNonemptyList)` | `unsupported JIT opcode: TypeTest(IsNonemptyList)` |
| 7 | `unsupported JIT opcode: SelectTupleArity {…}` | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 1 }, fail: Label(108), list: List([Unsigned(2), Label(107), Unsigned(3), Label(106)]) }`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(1), type_index: 1 }, fail: Label(67), list: List([Unsigned(2), Label(66), Unsigned(3), Label(65)]) }`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(1), type_index: 1 }, fail: Label(60), list: List([Unsigned(2), Label(59), Unsigned(3), Label(58)]) }` |
| 8 | `unsupported JIT opcode: Bif(Bif1)` | `unsupported JIT opcode: Bif(Bif1)` |
| 9 | `unsupported JIT opcode: TypeTest(IsMap)` | `unsupported JIT opcode: TypeTest(IsMap)` |
| 10 | `unsupported JIT opcode: Bif(Bif0)` | `unsupported JIT opcode: Bif(Bif0)` |
| 11 | `unsupported JIT operand: frame slot count Allocation(…)` | `unsupported JIT operand: frame slot count Allocation([Words(1), Floats(0), Funs(1)])`<br>`unsupported JIT operand: frame slot count Allocation([Words(4), Floats(0), Funs(1)])`<br>`unsupported JIT operand: frame slot count Allocation([Words(5), Floats(0), Funs(1)])` |
| 12 | `unsupported JIT opcode: Bif(GcBif1)` | `unsupported JIT opcode: Bif(GcBif1)` |
| 13 | `unsupported JIT opcode: TypeTest(IsNil)` | `unsupported JIT opcode: TypeTest(IsNil)` |
| 14 | `unsupported JIT opcode: TypeTest(IsBoolean)` | `unsupported JIT opcode: TypeTest(IsBoolean)` |
| 15 | `unsupported JIT operand: arithmetic import Unsigned(N)` | `unsupported JIT operand: arithmetic import Unsigned(5)`<br>`unsupported JIT operand: arithmetic import Unsigned(14)`<br>`unsupported JIT operand: arithmetic import Unsigned(13)` |
| 16 | `unsupported JIT opcode: Catch {…}` | `unsupported JIT opcode: Catch { destination: Y(2), label: Label(146) }`<br>`unsupported JIT opcode: Catch { destination: Y(1), label: Label(140) }`<br>`unsupported JIT opcode: Catch { destination: Y(0), label: Label(3) }` |
| 17 | `unsupported JIT opcode: Badrecord {…}` | `unsupported JIT opcode: Badrecord { value: X(1) }`<br>`unsupported JIT opcode: Badrecord { value: X(0) }`<br>`unsupported JIT opcode: Badrecord { value: X(4) }` |
| 18 | `unsupported JIT opcode: TypeTest(IsFloat)` | `unsupported JIT opcode: TypeTest(IsFloat)` |
| 19 | `unsupported JIT opcode: UpdateRecord {…}` | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(119))), Unsigned(12), TypedRegister { register: X(0), type_index: 3 }, Y(2), List([Unsigned(3), X(1)])] }`<br>`unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(83))), Unsigned(19), TypedRegister { register: X(1), type_index: 1 }, X(0), List([Unsigned(19), X(0)])] }`<br>`unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(101))), Unsigned(5), TypedRegister { register: X(0), type_index: 2 }, X(0), List([Unsigned(2), Literal(6)])] }` |
| 20 | `unsupported JIT opcode: TypeTest(IsFunction2)` | `unsupported JIT opcode: TypeTest(IsFunction2)` |
| 21 | `unsupported JIT opcode: TypeTest(IsPort)` | `unsupported JIT opcode: TypeTest(IsPort)` |
| 22 | `unsupported JIT opcode: NifStart` | `unsupported JIT opcode: NifStart` |
| 23 | `unsupported JIT opcode: TypeTest(IsNumber)` | `unsupported JIT opcode: TypeTest(IsNumber)` |
| 24 | `unsupported JIT opcode: TypeTest(IsFunction)` | `unsupported JIT opcode: TypeTest(IsFunction)` |
| 25 | `unsupported JIT opcode: TypeTest(IsReference)` | `unsupported JIT opcode: TypeTest(IsReference)` |
| 26 | `unsupported JIT opcode: runtime-deopt-capable Deallocate {…} is reachable after an observable side effect (a deopt would replay the effect on restart)` | `unsupported JIT opcode: runtime-deopt-capable Deallocate { words: Unsigned(0) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable Deallocate { words: Unsigned(3) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable Deallocate { words: Unsigned(2) } is reachable after an observable side effect (a deopt would replay the effect on restart)` |
| 27 | `unsupported JIT opcode: CallFun {…} in body position (not immediately followed by Return): the JIT tier has no body-call model` | `unsupported JIT opcode: CallFun { arity: Unsigned(0) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: CallFun { arity: Unsigned(1) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: CallFun { arity: Unsigned(2) } in body position (not immediately followed by Return): the JIT tier has no body-call model` |
| 28 | `unsupported JIT opcode: CaseEnd {…}` | `unsupported JIT opcode: CaseEnd { value: X(0) }`<br>`unsupported JIT opcode: CaseEnd { value: X(1) }`<br>`unsupported JIT opcode: CaseEnd { value: X(2) }` |
| 29 | `unsupported JIT opcode: IfEnd` | `unsupported JIT opcode: IfEnd` |
| 30 | `unsupported JIT opcode: Badmatch {…}` | `unsupported JIT opcode: Badmatch { value: X(0) }`<br>`unsupported JIT opcode: Badmatch { value: Atom(Some(Atom(3))) }`<br>`unsupported JIT opcode: Badmatch { value: X(2) }` |
| 31 | `unsupported JIT opcode: runtime-deopt-capable Allocate {…} is reachable after an observable side effect (a deopt would replay the effect on restart)` | `unsupported JIT opcode: runtime-deopt-capable Allocate { stack_need: Unsigned(0), live: Unsigned(0) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable Allocate { stack_need: Unsigned(1), live: Unsigned(1) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable Allocate { stack_need: Unsigned(2), live: Unsigned(2) } is reachable after an observable side effect (a deopt would replay the effect on restart)` |
| 32 | `unsupported JIT operand: select_val candidate Literal(N)` | `unsupported JIT operand: select_val candidate Literal(0)` |
| 33 | `unsupported JIT opcode: BinaryOp {…}` | `unsupported JIT opcode: BinaryOp { op: BsGetPosition, operands: [X(1), X(0), Unsigned(2)] }`<br>`unsupported JIT opcode: BinaryOp { op: BsGetPosition, operands: [X(2), X(1), Unsigned(3)] }` |
| 34 | `unsupported JIT opcode: runtime-deopt-capable CallExtLast {…} is reachable after an observable side effect (a deopt would replay the effect on restart)` | `unsupported JIT opcode: runtime-deopt-capable CallExtLast { arity: Unsigned(0), import: Unsigned(8), deallocate: Unsigned(1) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable CallExtLast { arity: Unsigned(2), import: Unsigned(67), deallocate: Unsigned(2) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable CallExtLast { arity: Unsigned(3), import: Unsigned(14), deallocate: Unsigned(2) } is reachable after an observable side effect (a deopt would replay the effect on restart)` |
| 35 | `unsupported JIT opcode: runtime-deopt-capable TestHeap {…} is reachable after an observable side effect (a deopt would replay the effect on restart)` | `unsupported JIT opcode: runtime-deopt-capable TestHeap { heap_need: Unsigned(4), live: Unsigned(0) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable TestHeap { heap_need: Unsigned(3), live: Unsigned(0) } is reachable after an observable side effect (a deopt would replay the effect on restart)` |
| 36 | `unsupported JIT opcode: OnLoad` | `unsupported JIT opcode: OnLoad` |
| 37 | `unsupported JIT opcode: TypeTest(IsBitstr)` | `unsupported JIT opcode: TypeTest(IsBitstr)` |
| 38 | `unsupported JIT operand: invalid bs_create_bin segment operands` | `unsupported JIT operand: invalid bs_create_bin segment operands` |
| 39 | `unsupported JIT opcode: runtime-deopt-capable FuncInfo {…} is reachable after an observable side effect (a deopt would replay the effect on restart)` | `unsupported JIT opcode: runtime-deopt-capable FuncInfo { module: Atom(Some(Atom(77))), function: Atom(Some(Atom(137))), arity: Unsigned(2) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable FuncInfo { module: Atom(Some(Atom(77))), function: Atom(Some(Atom(134))), arity: Unsigned(1) } is reachable after an observable side effect (a deopt would replay the effect on restart)` |

## 3. Rejection ranking, EXACT normalized reason

Distinct normalized rejection reasons: **59**.

| # | normalized reason | fns (all) | mods (all) | fns FIXTURES | mods FIXTURES | fns OTP | mods OTP | % of all skips |
|---:|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | `unsupported JIT opcode: Call { arity: Unsigned(N), label: Label(N) } in body position (not immediately followed by Return): the JIT tier has no body-call model` | 3887 | 635 | 44 | 21 | 3843 | 614 | 17.53% |
| 2 | `unknown JIT label: N` | 3806 | 661 | 83 | 26 | 3723 | 635 | 17.17% |
| 3 | `unsupported JIT operand: Literal(N)` | 3592 | 946 | 103 | 30 | 3489 | 916 | 16.20% |
| 4 | `unsupported JIT opcode: CallExt { arity: Unsigned(N), import: Unsigned(N) } in body position (not immediately followed by Return): the JIT tier has no body-call model` | 3189 | 724 | 175 | 38 | 3014 | 686 | 14.38% |
| 5 | `unsupported JIT opcode: Apply { arity: Unsigned(N) } in body position (not immediately followed by Return): the JIT tier has no body-call model` | 2990 | 239 | 0 | 0 | 2990 | 239 | 13.49% |
| 6 | `unsupported JIT opcode: TypeTest(IsNonemptyList)` | 872 | 332 | 62 | 12 | 810 | 320 | 3.93% |
| 7 | `unsupported JIT opcode: Bif(Bif1)` | 430 | 210 | 5 | 3 | 425 | 207 | 1.94% |
| 8 | `unsupported JIT opcode: TypeTest(IsMap)` | 340 | 111 | 6 | 4 | 334 | 107 | 1.53% |
| 9 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N)]) }` | 328 | 208 | 10 | 3 | 318 | 205 | 1.48% |
| 10 | `unsupported JIT opcode: Bif(Bif0)` | 294 | 141 | 7 | 3 | 287 | 138 | 1.33% |
| 11 | `unsupported JIT operand: frame slot count Allocation([Words(N), Floats(N), Funs(N)])` | 294 | 73 | 61 | 13 | 233 | 60 | 1.33% |
| 12 | `unsupported JIT opcode: Bif(GcBif1)` | 257 | 71 | 37 | 15 | 220 | 56 | 1.16% |
| 13 | `unsupported JIT opcode: TypeTest(IsNil)` | 197 | 160 | 2 | 2 | 195 | 158 | 0.89% |
| 14 | `unsupported JIT opcode: TypeTest(IsBoolean)` | 182 | 59 | 0 | 0 | 182 | 59 | 0.82% |
| 15 | `unsupported JIT operand: arithmetic import Unsigned(N)` | 168 | 88 | 31 | 11 | 137 | 77 | 0.76% |
| 16 | `unsupported JIT opcode: Catch { destination: Y(N), label: Label(N) }` | 162 | 79 | 0 | 0 | 162 | 79 | 0.73% |
| 17 | `unsupported JIT opcode: Badrecord { value: X(N) }` | 159 | 27 | 0 | 0 | 159 | 27 | 0.72% |
| 18 | `unsupported JIT opcode: TypeTest(IsFloat)` | 152 | 9 | 4 | 2 | 148 | 7 | 0.69% |
| 19 | `unsupported JIT opcode: TypeTest(IsFunction2)` | 94 | 29 | 1 | 1 | 93 | 28 | 0.42% |
| 20 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), X(N)])] }` | 84 | 23 | 0 | 0 | 84 | 23 | 0.38% |
| 21 | `unsupported JIT opcode: TypeTest(IsPort)` | 84 | 10 | 1 | 1 | 83 | 9 | 0.38% |
| 22 | `unsupported JIT opcode: NifStart` | 78 | 8 | 0 | 0 | 78 | 8 | 0.35% |
| 23 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N)]) }` | 75 | 62 | 0 | 0 | 75 | 62 | 0.34% |
| 24 | `unsupported JIT opcode: TypeTest(IsNumber)` | 61 | 15 | 0 | 0 | 61 | 15 | 0.28% |
| 25 | `unsupported JIT opcode: TypeTest(IsFunction)` | 56 | 28 | 0 | 0 | 56 | 28 | 0.25% |
| 26 | `unsupported JIT opcode: TypeTest(IsReference)` | 53 | 15 | 1 | 1 | 52 | 14 | 0.24% |
| 27 | `unsupported JIT opcode: runtime-deopt-capable Deallocate { words: Unsigned(N) } is reachable after an observable side effect (a deopt would replay the effect on restart)` | 47 | 28 | 3 | 2 | 44 | 26 | 0.21% |
| 28 | `unsupported JIT opcode: CallFun { arity: Unsigned(N) } in body position (not immediately followed by Return): the JIT tier has no body-call model` | 46 | 27 | 29 | 14 | 17 | 13 | 0.21% |
| 29 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N)]) …` | 45 | 40 | 0 | 0 | 45 | 40 | 0.20% |
| 30 | `unsupported JIT opcode: CaseEnd { value: X(N) }` | 39 | 17 | 28 | 7 | 11 | 10 | 0.18% |
| 31 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), U…` | 17 | 15 | 0 | 0 | 17 | 15 | 0.08% |
| 32 | `unsupported JIT opcode: IfEnd` | 13 | 5 | 1 | 1 | 12 | 4 | 0.06% |
| 33 | `unsupported JIT opcode: Badmatch { value: X(N) }` | 10 | 8 | 3 | 1 | 7 | 7 | 0.05% |
| 34 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), TypedRegister { register: X(N), type_index: N }])] }` | 9 | 9 | 0 | 0 | 9 | 9 | 0.04% |
| 35 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), Atom(Some(Atom(N)))])] }` | 8 | 7 | 0 | 0 | 8 | 7 | 0.04% |
| 36 | `unsupported JIT opcode: runtime-deopt-capable Allocate { stack_need: Unsigned(N), live: Unsigned(N) } is reachable after an observable side effect (a deopt would replay the effect on restart)` | 5 | 4 | 2 | 2 | 3 | 2 | 0.02% |
| 37 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), Literal(N)])] }` | 4 | 4 | 0 | 0 | 4 | 4 | 0.02% |
| 38 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), X(N), Unsigned(N), X(N)])] }` | 4 | 3 | 0 | 0 | 4 | 3 | 0.02% |
| 39 | `unsupported JIT operand: select_val candidate Literal(N)` | 4 | 2 | 4 | 2 | 0 | 0 | 0.02% |
| 40 | `unsupported JIT opcode: BinaryOp { op: BsGetPosition, operands: [X(N), X(N), Unsigned(N)] }` | 3 | 3 | 2 | 2 | 1 | 1 | 0.01% |
| 41 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), U…` | 3 | 3 | 0 | 0 | 3 | 3 | 0.01% |
| 42 | `unsupported JIT opcode: runtime-deopt-capable CallExtLast { arity: Unsigned(N), import: Unsigned(N), deallocate: Unsigned(N) } is reachable after an observable side effect (a deopt would replay the effect on restart)` | 3 | 3 | 0 | 0 | 3 | 3 | 0.01% |
| 43 | `unsupported JIT opcode: runtime-deopt-capable TestHeap { heap_need: Unsigned(N), live: Unsigned(N) } is reachable after an observable side effect (a deopt would replay the effect on restart)` | 3 | 3 | 0 | 0 | 3 | 3 | 0.01% |
| 44 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), U…` | 3 | 1 | 0 | 0 | 3 | 1 | 0.01% |
| 45 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), U…` | 3 | 1 | 0 | 0 | 3 | 1 | 0.01% |
| 46 | `unsupported JIT opcode: OnLoad` | 2 | 2 | 2 | 2 | 0 | 0 | 0.01% |
| 47 | `unsupported JIT opcode: TypeTest(IsBitstr)` | 2 | 2 | 2 | 2 | 0 | 0 | 0.01% |
| 48 | `unsupported JIT operand: invalid bs_create_bin segment operands` | 2 | 2 | 2 | 2 | 0 | 0 | 0.01% |
| 49 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), Atom(Some(Atom(N))), Unsigned(N), X(N)])] }` | 2 | 1 | 0 | 0 | 2 | 1 | 0.01% |
| 50 | `unsupported JIT opcode: runtime-deopt-capable FuncInfo { module: Atom(Some(Atom(N))), function: Atom(Some(Atom(N))), arity: Unsigned(N) } is reachable after an observable side effect (a deopt would replay the effect on r…` | 2 | 1 | 0 | 0 | 2 | 1 | 0.01% |
| 51 | `unsupported JIT opcode: Badmatch { value: Atom(Some(Atom(N))) }` | 1 | 1 | 0 | 0 | 1 | 1 | 0.00% |
| 52 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), U…` | 1 | 1 | 0 | 0 | 1 | 1 | 0.00% |
| 53 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: Y(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N)]) }` | 1 | 1 | 0 | 0 | 1 | 1 | 0.00% |
| 54 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), Atom(None)])] }` | 1 | 1 | 0 | 0 | 1 | 1 | 0.00% |
| 55 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), TypedRegister { register: X(N), type_index: N }, Unsigned(N), …` | 1 | 1 | 0 | 0 | 1 | 1 | 0.00% |
| 56 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), X(N), Unsigned(N), X(N), Unsigned(N), X(N)])] }` | 1 | 1 | 0 | 0 | 1 | 1 | 0.00% |
| 57 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, Y(N), List([Unsigned(N), X(N)])] }` | 1 | 1 | 0 | 0 | 1 | 1 | 0.00% |
| 58 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, Y(N), List([Unsigned(N), Y(N)])] }` | 1 | 1 | 0 | 0 | 1 | 1 | 0.00% |
| 59 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: Y(N), type_index: N }, X(N), List([Unsigned(N), Y(N)])] }` | 1 | 1 | 0 | 0 | 1 | 1 | 0.00% |

## 4. Exact normalized reason → verbatim examples

| # | normalized reason | verbatim examples (up to 3) |
|---:|---|---|
| 1 | `unsupported JIT opcode: Call { arity: Unsigned(N), label: Label(N) } in body position (not immediately followed by Return): the JIT tier has no body-call model` | `unsupported JIT opcode: Call { arity: Unsigned(1), label: Label(46) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: Call { arity: Unsigned(1), label: Label(2) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: Call { arity: Unsigned(1), label: Label(8) } in body position (not immediately followed by Return): the JIT tier has no body-call model` |
| 2 | `unknown JIT label: N` | `unknown JIT label: 16`<br>`unknown JIT label: 62`<br>`unknown JIT label: 422` |
| 3 | `unsupported JIT operand: Literal(N)` | `unsupported JIT operand: Literal(3)`<br>`unsupported JIT operand: Literal(2)`<br>`unsupported JIT operand: Literal(1)` |
| 4 | `unsupported JIT opcode: CallExt { arity: Unsigned(N), import: Unsigned(N) } in body position (not immediately followed by Return): the JIT tier has no body-call model` | `unsupported JIT opcode: CallExt { arity: Unsigned(1), import: Unsigned(0) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: CallExt { arity: Unsigned(2), import: Unsigned(6) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: CallExt { arity: Unsigned(2), import: Unsigned(4) } in body position (not immediately followed by Return): the JIT tier has no body-call model` |
| 5 | `unsupported JIT opcode: Apply { arity: Unsigned(N) } in body position (not immediately followed by Return): the JIT tier has no body-call model` | `unsupported JIT opcode: Apply { arity: Unsigned(0) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: Apply { arity: Unsigned(1) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: Apply { arity: Unsigned(3) } in body position (not immediately followed by Return): the JIT tier has no body-call model` |
| 6 | `unsupported JIT opcode: TypeTest(IsNonemptyList)` | `unsupported JIT opcode: TypeTest(IsNonemptyList)` |
| 7 | `unsupported JIT opcode: Bif(Bif1)` | `unsupported JIT opcode: Bif(Bif1)` |
| 8 | `unsupported JIT opcode: TypeTest(IsMap)` | `unsupported JIT opcode: TypeTest(IsMap)` |
| 9 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N)]) }` | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 1 }, fail: Label(108), list: List([Unsigned(2), Label(107), Unsigned(3), Label(106)]) }`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(1), type_index: 1 }, fail: Label(67), list: List([Unsigned(2), Label(66), Unsigned(3), Label(65)]) }`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(1), type_index: 1 }, fail: Label(60), list: List([Unsigned(2), Label(59), Unsigned(3), Label(58)]) }` |
| 10 | `unsupported JIT opcode: Bif(Bif0)` | `unsupported JIT opcode: Bif(Bif0)` |
| 11 | `unsupported JIT operand: frame slot count Allocation([Words(N), Floats(N), Funs(N)])` | `unsupported JIT operand: frame slot count Allocation([Words(1), Floats(0), Funs(1)])`<br>`unsupported JIT operand: frame slot count Allocation([Words(4), Floats(0), Funs(1)])`<br>`unsupported JIT operand: frame slot count Allocation([Words(5), Floats(0), Funs(1)])` |
| 12 | `unsupported JIT opcode: Bif(GcBif1)` | `unsupported JIT opcode: Bif(GcBif1)` |
| 13 | `unsupported JIT opcode: TypeTest(IsNil)` | `unsupported JIT opcode: TypeTest(IsNil)` |
| 14 | `unsupported JIT opcode: TypeTest(IsBoolean)` | `unsupported JIT opcode: TypeTest(IsBoolean)` |
| 15 | `unsupported JIT operand: arithmetic import Unsigned(N)` | `unsupported JIT operand: arithmetic import Unsigned(5)`<br>`unsupported JIT operand: arithmetic import Unsigned(14)`<br>`unsupported JIT operand: arithmetic import Unsigned(13)` |
| 16 | `unsupported JIT opcode: Catch { destination: Y(N), label: Label(N) }` | `unsupported JIT opcode: Catch { destination: Y(2), label: Label(146) }`<br>`unsupported JIT opcode: Catch { destination: Y(1), label: Label(140) }`<br>`unsupported JIT opcode: Catch { destination: Y(0), label: Label(3) }` |
| 17 | `unsupported JIT opcode: Badrecord { value: X(N) }` | `unsupported JIT opcode: Badrecord { value: X(1) }`<br>`unsupported JIT opcode: Badrecord { value: X(0) }`<br>`unsupported JIT opcode: Badrecord { value: X(4) }` |
| 18 | `unsupported JIT opcode: TypeTest(IsFloat)` | `unsupported JIT opcode: TypeTest(IsFloat)` |
| 19 | `unsupported JIT opcode: TypeTest(IsFunction2)` | `unsupported JIT opcode: TypeTest(IsFunction2)` |
| 20 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), X(N)])] }` | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(83))), Unsigned(19), TypedRegister { register: X(1), type_index: 1 }, X(0), List([Unsigned(19), X(0)])] }`<br>`unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(118))), Unsigned(4), TypedRegister { register: X(1), type_index: 1 }, X(0), List([Unsigned(2), X(3)])] }`<br>`unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(118))), Unsigned(3), TypedRegister { register: X(1), type_index: 1 }, X(0), List([Unsigned(2), X(3)])] }` |
| 21 | `unsupported JIT opcode: TypeTest(IsPort)` | `unsupported JIT opcode: TypeTest(IsPort)` |
| 22 | `unsupported JIT opcode: NifStart` | `unsupported JIT opcode: NifStart` |
| 23 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N)]) }` | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 2 }, fail: Label(179), list: List([Unsigned(5), Label(177), Unsigned(6), Label(176), Unsigned(7), Label(175)]) }`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 2 }, fail: Label(163), list: List([Unsigned(5), Label(162), Unsigned(6), Label(160), Unsigned(7), Label(156)]) }`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 3 }, fail: Label(2182), list: List([Unsigned(2), Label(2133), Unsigned(3), Label(2127), Unsigned(4), Label(2126)]) }` |
| 24 | `unsupported JIT opcode: TypeTest(IsNumber)` | `unsupported JIT opcode: TypeTest(IsNumber)` |
| 25 | `unsupported JIT opcode: TypeTest(IsFunction)` | `unsupported JIT opcode: TypeTest(IsFunction)` |
| 26 | `unsupported JIT opcode: TypeTest(IsReference)` | `unsupported JIT opcode: TypeTest(IsReference)` |
| 27 | `unsupported JIT opcode: runtime-deopt-capable Deallocate { words: Unsigned(N) } is reachable after an observable side effect (a deopt would replay the effect on restart)` | `unsupported JIT opcode: runtime-deopt-capable Deallocate { words: Unsigned(0) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable Deallocate { words: Unsigned(3) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable Deallocate { words: Unsigned(2) } is reachable after an observable side effect (a deopt would replay the effect on restart)` |
| 28 | `unsupported JIT opcode: CallFun { arity: Unsigned(N) } in body position (not immediately followed by Return): the JIT tier has no body-call model` | `unsupported JIT opcode: CallFun { arity: Unsigned(0) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: CallFun { arity: Unsigned(1) } in body position (not immediately followed by Return): the JIT tier has no body-call model`<br>`unsupported JIT opcode: CallFun { arity: Unsigned(2) } in body position (not immediately followed by Return): the JIT tier has no body-call model` |
| 29 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N)]) …` | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 2 }, fail: Label(172), list: List([Unsigned(4), Label(170), Unsigned(5), Label(168), Unsigned(6), Label(167), Unsigned(7), Label(166)]) }`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 2 }, fail: Label(509), list: List([Unsigned(2), Label(508), Unsigned(3), Label(506), Unsigned(4), Label(505), Unsigned(5), Label(504)]) }`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 1 }, fail: Label(67), list: List([Unsigned(2), Label(66), Unsigned(3), Label(63), Unsigned(4), Label(49), Unsigned(5), Label(46)]) }` |
| 30 | `unsupported JIT opcode: CaseEnd { value: X(N) }` | `unsupported JIT opcode: CaseEnd { value: X(0) }`<br>`unsupported JIT opcode: CaseEnd { value: X(1) }`<br>`unsupported JIT opcode: CaseEnd { value: X(2) }` |
| 31 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), U…` | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 1 }, fail: Label(185), list: List([Unsigned(2), Label(226), Unsigned(3), Label(213), Unsigned(4), Label(195), Unsigned(5), Label(189), Unsigned(6), Label(187)]) }`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 1 }, fail: Label(1), list: List([Unsigned(3), Label(25), Unsigned(4), Label(15), Unsigned(5), Label(7), Unsigned(6), Label(6), Unsigned(7), Label(3)]) }`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(1), type_index: 1 }, fail: Label(66), list: List([Unsigned(2), Label(65), Unsigned(3), Label(64), Unsigned(4), Label(61), Unsigned(21), Label(60), Unsigned(22), Label(59)]) }` |
| 32 | `unsupported JIT opcode: IfEnd` | `unsupported JIT opcode: IfEnd` |
| 33 | `unsupported JIT opcode: Badmatch { value: X(N) }` | `unsupported JIT opcode: Badmatch { value: X(0) }`<br>`unsupported JIT opcode: Badmatch { value: X(2) }`<br>`unsupported JIT opcode: Badmatch { value: X(1) }` |
| 34 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), TypedRegister { register: X(N), type_index: N }])] }` | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(97))), Unsigned(7), TypedRegister { register: X(1), type_index: 5 }, X(1), List([Unsigned(6), TypedRegister { register: X(7), type_index: 6 }])] }`<br>`unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(150))), Unsigned(9), TypedRegister { register: X(1), type_index: 1 }, X(0), List([Unsigned(9), TypedRegister { register: X(0), type_index: 11 }])] }`<br>`unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(179))), Unsigned(11), TypedRegister { register: X(1), type_index: 1 }, X(0), List([Unsigned(11), TypedRegister { register: X(0), type_index: 6 }])] }` |
| 35 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), Atom(Some(Atom(N)))])] }` | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(93))), Unsigned(6), TypedRegister { register: X(2), type_index: 2 }, X(0), List([Unsigned(2), Atom(Some(Atom(2)))])] }`<br>`unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(105))), Unsigned(11), TypedRegister { register: X(1), type_index: 1 }, X(1), List([Unsigned(7), Atom(Some(Atom(106)))])] }`<br>`unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(109))), Unsigned(3), TypedRegister { register: X(1), type_index: 2 }, X(1), List([Unsigned(2), Atom(Some(Atom(116)))])] }` |
| 36 | `unsupported JIT opcode: runtime-deopt-capable Allocate { stack_need: Unsigned(N), live: Unsigned(N) } is reachable after an observable side effect (a deopt would replay the effect on restart)` | `unsupported JIT opcode: runtime-deopt-capable Allocate { stack_need: Unsigned(0), live: Unsigned(0) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable Allocate { stack_need: Unsigned(1), live: Unsigned(1) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable Allocate { stack_need: Unsigned(2), live: Unsigned(2) } is reachable after an observable side effect (a deopt would replay the effect on restart)` |
| 37 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), Literal(N)])] }` | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(101))), Unsigned(5), TypedRegister { register: X(0), type_index: 2 }, X(0), List([Unsigned(2), Literal(6)])] }`<br>`unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(80))), Unsigned(62), TypedRegister { register: X(0), type_index: 1 }, X(0), List([Unsigned(50), Literal(8)])] }`<br>`unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(131))), Unsigned(13), TypedRegister { register: X(3), type_index: 2 }, X(0), List([Unsigned(10), Literal(18)])] }` |
| 38 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), X(N), Unsigned(N), X(N)])] }` | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(239))), Unsigned(4), TypedRegister { register: X(0), type_index: 1 }, X(0), List([Unsigned(3), X(1), Unsigned(4), X(2)])] }`<br>`unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(239))), Unsigned(5), TypedRegister { register: X(0), type_index: 1 }, X(0), List([Unsigned(3), X(1), Unsigned(4), X(2)])] }`<br>`unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(127))), Unsigned(19), TypedRegister { register: X(0), type_index: 1 }, X(0), List([Unsigned(16), X(1), Unsigned(17), X(2)])] }` |
| 39 | `unsupported JIT operand: select_val candidate Literal(N)` | `unsupported JIT operand: select_val candidate Literal(0)` |
| 40 | `unsupported JIT opcode: BinaryOp { op: BsGetPosition, operands: [X(N), X(N), Unsigned(N)] }` | `unsupported JIT opcode: BinaryOp { op: BsGetPosition, operands: [X(1), X(0), Unsigned(2)] }`<br>`unsupported JIT opcode: BinaryOp { op: BsGetPosition, operands: [X(2), X(1), Unsigned(3)] }` |
| 41 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), U…` | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 1 }, fail: Label(154), list: List([Unsigned(2), Label(153), Unsigned(3), Label(152), Unsigned(4), Label(149), Unsigned(6), Label(146), Unsigned(7), Label(145), Unsigned(8), Label(133)]) }`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(1), type_index: 2 }, fail: Label(147), list: List([Unsigned(2), Label(141), Unsigned(3), Label(140), Unsigned(4), Label(137), Unsigned(6), Label(134), Unsigned(7), Label(133), Unsigned(11), Label(132)]) }`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 2 }, fail: Label(118), list: List([Unsigned(1), Label(133), Unsigned(2), Label(127), Unsigned(3), Label(126), Unsigned(4), Label(122), Unsigned(5), Label(121), Unsigned(6), Label(120)]) }` |
| 42 | `unsupported JIT opcode: runtime-deopt-capable CallExtLast { arity: Unsigned(N), import: Unsigned(N), deallocate: Unsigned(N) } is reachable after an observable side effect (a deopt would replay the effect on restart)` | `unsupported JIT opcode: runtime-deopt-capable CallExtLast { arity: Unsigned(0), import: Unsigned(8), deallocate: Unsigned(1) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable CallExtLast { arity: Unsigned(2), import: Unsigned(67), deallocate: Unsigned(2) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable CallExtLast { arity: Unsigned(3), import: Unsigned(14), deallocate: Unsigned(2) } is reachable after an observable side effect (a deopt would replay the effect on restart)` |
| 43 | `unsupported JIT opcode: runtime-deopt-capable TestHeap { heap_need: Unsigned(N), live: Unsigned(N) } is reachable after an observable side effect (a deopt would replay the effect on restart)` | `unsupported JIT opcode: runtime-deopt-capable TestHeap { heap_need: Unsigned(4), live: Unsigned(0) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable TestHeap { heap_need: Unsigned(3), live: Unsigned(0) } is reachable after an observable side effect (a deopt would replay the effect on restart)` |
| 44 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), U…` | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(1), type_index: 1 }, fail: Label(169), list: List([Unsigned(2), Label(168), Unsigned(3), Label(167), Unsigned(4), Label(164), Unsigned(5), Label(161), Unsigned(7), Label(160), Unsigned(8), Label(159), Unsigned(12), Label(1…`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(1), type_index: 1 }, fail: Label(142), list: List([Unsigned(2), Label(141), Unsigned(3), Label(140), Unsigned(4), Label(137), Unsigned(5), Label(134), Unsigned(7), Label(133), Unsigned(8), Label(132), Unsigned(12), Label(1…`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 1 }, fail: Label(115), list: List([Unsigned(2), Label(114), Unsigned(3), Label(113), Unsigned(4), Label(110), Unsigned(5), Label(107), Unsigned(7), Label(106), Unsigned(8), Label(105), Unsigned(12), Label(1…` |
| 45 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), U…` | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(1), type_index: 1 }, fail: Label(176), list: List([Unsigned(2), Label(173), Unsigned(3), Label(172), Unsigned(4), Label(167), Unsigned(6), Label(166), Unsigned(7), Label(165), Unsigned(9), Label(164), Unsigned(12), Label(1…`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(1), type_index: 1 }, fail: Label(148), list: List([Unsigned(2), Label(145), Unsigned(3), Label(144), Unsigned(4), Label(139), Unsigned(6), Label(138), Unsigned(7), Label(137), Unsigned(9), Label(136), Unsigned(12), Label(1…`<br>`unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 1 }, fail: Label(120), list: List([Unsigned(2), Label(117), Unsigned(3), Label(116), Unsigned(4), Label(111), Unsigned(6), Label(110), Unsigned(7), Label(109), Unsigned(9), Label(108), Unsigned(12), Label(1…` |
| 46 | `unsupported JIT opcode: OnLoad` | `unsupported JIT opcode: OnLoad` |
| 47 | `unsupported JIT opcode: TypeTest(IsBitstr)` | `unsupported JIT opcode: TypeTest(IsBitstr)` |
| 48 | `unsupported JIT operand: invalid bs_create_bin segment operands` | `unsupported JIT operand: invalid bs_create_bin segment operands` |
| 49 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), Atom(Some(Atom(N))), Unsigned(N), X(N)])] }` | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(95))), Unsigned(32), TypedRegister { register: X(5), type_index: 1 }, X(0), List([Unsigned(15), Atom(Some(Atom(3))), Unsigned(18), X(4)])] }` |
| 50 | `unsupported JIT opcode: runtime-deopt-capable FuncInfo { module: Atom(Some(Atom(N))), function: Atom(Some(Atom(N))), arity: Unsigned(N) } is reachable after an observable side effect (a deopt would replay the effect on r…` | `unsupported JIT opcode: runtime-deopt-capable FuncInfo { module: Atom(Some(Atom(77))), function: Atom(Some(Atom(137))), arity: Unsigned(2) } is reachable after an observable side effect (a deopt would replay the effect on restart)`<br>`unsupported JIT opcode: runtime-deopt-capable FuncInfo { module: Atom(Some(Atom(77))), function: Atom(Some(Atom(134))), arity: Unsigned(1) } is reachable after an observable side effect (a deopt would replay the effect on restart)` |
| 51 | `unsupported JIT opcode: Badmatch { value: Atom(Some(Atom(N))) }` | `unsupported JIT opcode: Badmatch { value: Atom(Some(Atom(3))) }` |
| 52 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), Unsigned(N), Label(N), U…` | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: X(0), type_index: 1 }, fail: Label(6), list: List([Unsigned(1), Label(186), Unsigned(2), Label(159), Unsigned(3), Label(122), Unsigned(4), Label(84), Unsigned(5), Label(38), Unsigned(6), Label(12), Unsigned(14), Label(8)]) }` |
| 53 | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: Y(N), type_index: N }, fail: Label(N), list: List([Unsigned(N), Label(N), Unsigned(N), Label(N)]) }` | `unsupported JIT opcode: SelectTupleArity { value: TypedRegister { register: Y(0), type_index: 2 }, fail: Label(33), list: List([Unsigned(2), Label(32), Unsigned(3), Label(31)]) }` |
| 54 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), Atom(None)])] }` | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(206))), Unsigned(9), TypedRegister { register: X(2), type_index: 3 }, X(2), List([Unsigned(3), Atom(None)])] }` |
| 55 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), TypedRegister { register: X(N), type_index: N }, Unsigned(N), …` | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(97))), Unsigned(9), TypedRegister { register: X(1), type_index: 4 }, X(0), List([Unsigned(2), TypedRegister { register: X(0), type_index: 1 }, Unsigned(3), TypedRegister { register: X(3), type_index: 2 }])] }` |
| 56 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, X(N), List([Unsigned(N), X(N), Unsigned(N), X(N), Unsigned(N), X(N)])] }` | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(239))), Unsigned(5), TypedRegister { register: X(0), type_index: 1 }, X(0), List([Unsigned(3), X(1), Unsigned(4), X(2), Unsigned(5), X(3)])] }` |
| 57 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, Y(N), List([Unsigned(N), X(N)])] }` | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(119))), Unsigned(12), TypedRegister { register: X(0), type_index: 3 }, Y(2), List([Unsigned(3), X(1)])] }` |
| 58 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: X(N), type_index: N }, Y(N), List([Unsigned(N), Y(N)])] }` | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(119))), Unsigned(9), TypedRegister { register: X(1), type_index: 1 }, Y(0), List([Unsigned(3), Y(1)])] }` |
| 59 | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(N))), Unsigned(N), TypedRegister { register: Y(N), type_index: N }, X(N), List([Unsigned(N), Y(N)])] }` | `unsupported JIT opcode: UpdateRecord { operands: [Atom(Some(Atom(111))), Unsigned(19), TypedRegister { register: Y(1), type_index: 1 }, X(0), List([Unsigned(7), Y(0)])] }` |

## 5. Loader coverage findings (NOT JIT rejections)

These modules never reached the JIT: `load_beam_chunks` (or the file read) refused them. They are a LOADER coverage census and must not be mixed into the ranking above.

Reasons here are VERBATIM, not normalized: the numeric specific is the finding (opcode 185 and opcode 186 are distinct loader coverage gaps).

| # | verbatim load-error | modules | example modules |
|---:|---|---:|---|
| 1 | `load: failed to decode BEAM data: unsupported opcode 186` | 6 | `debugger-7.0/dbg_ieval.beam`<br>`dialyzer-6.0.2/dialyzer_codeserver.beam`<br>`dialyzer-6.0.2/erl_types.beam`<br>`stdlib-8.0.2/erl_eval.beam`<br>`stdlib-8.0.2/io_lib.beam`<br>`stdlib-8.0.2/io_lib_pretty.beam` |
| 2 | `load: failed to decode BEAM data: unsupported ETF literal tag 77` | 4 | `megaco-4.9/megaco_per_media_gateway_control_v1.beam`<br>`megaco-4.9/megaco_per_media_gateway_control_v2.beam`<br>`megaco-4.9/megaco_per_media_gateway_control_v3.beam`<br>`public_key-1.21.3/PKIXAttributeCertificate-2009.beam` |
| 3 | `load: failed to decode BEAM data: unsupported opcode 185` | 4 | `kernel-11.0.3/inet_db.beam`<br>`stdlib-8.0.2/calendar.beam`<br>`stdlib-8.0.2/io_lib_fread.beam`<br>`stdlib-8.0.2/rand.beam` |

## 6. Module-fatal JIT errors

A non-skippable `JitError` (or a panic) aborts the whole module in `compile_module`, so its remaining exports are never classified. Count: **0**.

None.

## 7. Per-application split (OTP_STDLIB)

| app | modules | loaded | load-error | module-fatal | compiled | skipped |
|---|---:|---:|---:|---:|---:|---:|
| asn1-5.5 | 22 | 22 | 0 | 0 | 66 | 210 |
| common_test-1.31.1 | 47 | 47 | 0 | 0 | 193 | 785 |
| compiler-10.0.2 | 59 | 59 | 0 | 0 | 261 | 378 |
| crypto-5.9.1 | 2 | 2 | 0 | 0 | 8 | 102 |
| debugger-7.0 | 24 | 23 | 1 | 0 | 109 | 180 |
| dialyzer-6.0.2 | 29 | 27 | 2 | 0 | 60 | 174 |
| diameter-2.7.1 | 47 | 47 | 0 | 0 | 246 | 752 |
| edoc-1.5 | 21 | 21 | 0 | 0 | 55 | 155 |
| eldap-1.3 | 2 | 2 | 0 | 0 | 21 | 141 |
| erts-17.0.3 | 22 | 22 | 0 | 0 | 416 | 402 |
| et-1.8 | 6 | 6 | 0 | 0 | 23 | 51 |
| eunit-2.11 | 13 | 13 | 0 | 0 | 47 | 94 |
| ftp-1.2.6 | 6 | 6 | 0 | 0 | 54 | 57 |
| inets-9.7.1 | 63 | 63 | 0 | 0 | 180 | 494 |
| kernel-11.0.3 | 104 | 103 | 1 | 0 | 594 | 1523 |
| megaco-4.9 | 65 | 62 | 3 | 0 | 265 | 1166 |
| mnesia-4.26.1 | 31 | 31 | 0 | 0 | 165 | 741 |
| observer-2.19 | 44 | 44 | 0 | 0 | 176 | 317 |
| odbc-2.17 | 3 | 3 | 0 | 0 | 12 | 29 |
| os_mon-2.12 | 8 | 8 | 0 | 0 | 43 | 67 |
| parsetools-2.8 | 4 | 4 | 0 | 0 | 14 | 16 |
| public_key-1.21.3 | 42 | 41 | 1 | 0 | 433 | 2348 |
| reltool-1.1 | 9 | 9 | 0 | 0 | 63 | 101 |
| runtime_tools-2.4 | 12 | 12 | 0 | 0 | 45 | 181 |
| sasl-4.4 | 17 | 17 | 0 | 0 | 57 | 145 |
| snmp-5.20.4 | 90 | 90 | 0 | 0 | 434 | 1285 |
| ssh-6.0.2 | 43 | 43 | 0 | 0 | 181 | 500 |
| ssl-11.7.3 | 78 | 78 | 0 | 0 | 352 | 880 |
| stdlib-8.0.2 | 98 | 92 | 6 | 0 | 516 | 1994 |
| syntax_tools-4.1 | 9 | 9 | 0 | 0 | 28 | 414 |
| tftp-1.3 | 8 | 8 | 0 | 0 | 27 | 40 |
| tools-4.2.1 | 16 | 16 | 0 | 0 | 107 | 304 |
| wx-2.6 | 241 | 241 | 0 | 0 | 17773 | 5201 |
| xmerl-2.2 | 35 | 35 | 0 | 0 | 113 | 234 |
