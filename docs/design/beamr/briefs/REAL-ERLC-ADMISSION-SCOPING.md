# REAL-ERLC-ADMISSION arc — scoping brief

**Author:** Artemis Peach (beamr seat). **Dispatch:** Tom via Waffles, 2026-07-22
~14:12Z ("scope the arc from the five archived red fixtures + the typed-overflow
deopt-restart class, fold JIT-003's sub-op widening in as a candidate first leg —
ranked against what the fixtures actually say blocks admission").
**Base state:** beamr main `1a1ca1e` (JIT-002 landed at `251df6b`).
**Red suite:** `artemis-artifacts/2026-07-22-jit002build/` — five fixtures, all
real OTP-29 erlc output, archived by the JIT-002 build leg.

## 1. What the fixtures actually say

| Fixture | Status | What it proves blocks admission |
|---|---|---|
| `frameless.erl` | RED | Multi-clause dispatch: `select_val`'s fail target is the func_info prelude label, which the slicer strips (`aot.rs:342-370` starts at `entry+1`) → `UnknownLabel`. **Every function with >1 clause hits this.** |
| `jit_real_function.{erl,beam}` | RED | (a) Self-`call_last {f,entry}` targets the stripped entry label → `UnknownLabel`. (b) Frame + Y-live-across-**body**-call — the continuation call model the tier does not have. |
| `jit_real_tail_loop.erl` | RED | Body `call_ext` with continuation is mis-compiled absent the R3 wall (the 336-vs-777 probe); with the wall it is honestly rejected. Corollary proven at erlc -S: **frames (R1) and `call_ext_last` (R2) never occur on real erlc output without a body call** — so the whole JIT-002 frame substrate is unreachable end-to-end until body calls exist. |
| `jit_countdown_loop.{erl,beam}` | GREEN (landed) | The `{f,0}` no-fail route + purity guard admitted it — proof the arc's increments convert directly to real-erlc admission. Stays as the admission-telemetry vehicle. |
| `jit_badarith.{erl,beam}` | GREEN (landed) | Deopt-vs-interpreter exception-equality differential — the proof pattern every arc leg's deopt-adjacent work must reuse. |

Reading: the structural blockers are (i) **slicer label stripping** — ubiquitous,
every multi-clause function — and (ii) the **missing body-call continuation
model** — gates all frame/Y/`call_ext_last` code. No red fixture is blocked on a
missing sub-op. That is the evidence against dispatching JIT-003 first (§5).

## 2. The deopt-restart soundness class (byte-verified 2026-07-22 on `1a1ca1e`)

Deopt semantics: `JIT_STATUS_DEOPT` → `Ok(None)` (`interpreter/opcodes/core.rs:850`)
→ the callee **restarts interpreted from its start**. Any restart after an
observable side effect replays that effect. JIT-002 guarded exactly one door —
the `{f,0}` Bif purity guard (`ir_control.rs:455-456`) — and disclosed the rest
of the class to this arc. Verified today:

- `lower_typed_int_arithmetic` branches to deopt on overflow/non-small-int
  **unconditionally, even when the Bif has a real in-slice fail label**
  (`ir_typed.rs:32-55`; the fail label serves only the untyped helper path).
- The purity guard does not cover it: it applies only to `is_no_fail_label`
  Bifs (`ir_control.rs:455`), and a real-fail-label Bif skips it (`:639`).
- `Send` flushes typed state (`materialize_registers` removes the marker,
  `ir_typed.rs:320`) — but any later literal `Move` re-arms it
  (`mark_loaded_operand_type`), so `[Send, Move #lit→X, typed-arith overflow]`
  is a constructible **duplication** replay (double send).
- The receive family's deopt edges (`translate_loop_rec`,
  `ir_message.rs:53-74`) sit adjacent to `RemoveMessage` — a deopt landing
  after an accepted message restarts a receive whose message is already
  consumed: the **loss** shape.

Severity profile: trigger is rare (genuine small-int overflow / speculation
miss), consequence is silent message duplication or loss — the worst failure
class this VM has (cf. the monitor/2 silent hang, same family of "wrong quietly").
Reachability of a live end-to-end replay on main is NOT yet proven — the first
work item of Leg 1 is the probe that settles it (§4, Leg 1a). Scoping treats it
as presumed-reachable until the probe says otherwise.

## 2a. CORRECTIONS (2026-07-22, post-Leg-1 build — visible correction, not a rewrite)

Two §2 mechanism claims are corrected by Leg 1's byte-verified findings; the
class verdict (presumed-reachable) was CONFIRMED LIVE, but by a different
instance than §2 predicted:

1. **The typed-overflow shape is NOT reachable after a side effect.** §2's
   "literal moves re-arm typed state" is wrong: `Move` lowering uses
   `typed_state.copy` (which clears on a literal source) and never calls
   `mark_loaded_operand_type` — that marker is set only by
   `GetHd/GetTl/GetList/GetTupleElement` from an already-typed container, and
   every side-effecting op empties the typed map. The only Int-typing root is
   the entry signature. So `lower_typed_int_arithmetic`'s overflow→deopt is
   unreachable post-side-effect.
2. **The LIVE instance is the unconditional-deopt lowerings** —
   `RecvMarkerReserve` (a `Coverage::Supported` op lowered as unconditional
   deopt) placed after a `Send` in one admitted slice. Probe red on main
   `1a1ca1e`: JIT-live drive `(Error, Some(Badarg))` vs interpreter-only
   `(Normal, None)` — a divergent crash (native clobbered x0 before the
   restart), the §2 class exactly, different observable than the predicted
   silent duplication.
3. **NEW FINDING (changes Leg 1c's ground):** beamr's interpreter has NO
   function_clause raise path at all. `core::func_info` sets the current MFA
   and returns Continue (`interpreter/opcodes/core.rs:68-80`);
   `ExecError::FunctionClause` (`error.rs:113`) is declared but never
   constructed anywhere in src. Consequence, empirically confirmed via a real
   erlc two-clause fixture driven through beamr-cli: a multi-clause no-match
   falls through the prelude back into the dispatch and **loops forever**
   (10s watchdog kill, ~full-core spin; fixture `fc_probe.erl`: `f(a) -> ok.`
   called as `f(b)`). This is a production interpreter defect in the
   wrong-quietly family, and it means Leg 1c's A2 ("FuncInfo as the
   function_clause landing pad") has no correct interpreter semantics to stay
   differential-equal to until the interpreter is fixed. See §4 Leg 1c
   (amended).

## 3. Blocker map → four work classes

- **A. Slicer label retention** — two sub-shapes: (A1) retain the entry label
  (slice starts at `entry`, `Label` already Supported) → admits self-`call_last`;
  (A2) retain the func_info prelude (slice starts at the prelude label; lower
  `FuncInfo` as the function_clause landing pad) → admits every multi-clause
  function. Both slicers change together under the R8 slice-equality pin
  (`aot.rs:464` binds `Module::function_instructions` to
  `exported_instructions` — the pin is preserved by co-update, and stays the
  wall). A2 reclassifies `FuncInfo` in the 75-variant table
  (RejectedInherent → Supported-as-terminal), which the exhaustive no-wildcard
  `coverage()` forces us to do honestly.
- **B. Deopt-restart soundness** — interim: extend the purity discipline from
  "{f,0} Bifs" to "any runtime-deopt-capable instruction following an observable
  side effect" in the pre-pass, reusing the existing
  `is_observable_side_effect` authority (new use, no new machinery — the R3
  tail-wall precedent: a conservative wall that stays correct forever, not a
  half-measure to redo). Permanent: precise-resume deopt, which is a Leg 3
  outcome (§4).
- **C. Body-call continuation model** — native CP, mid-function resume, frame
  interop with the interpreter. Retires the R3 tail wall, makes frames/
  `call_ext_last`/Y-across-call reachable on real erlc, and provides the
  precise-resume substrate that retires B's conservatism. The arc's centerpiece
  and its largest design surface.
- **D. Sub-op widening (= JIT-003)** — TypeTest sub-ops (6/17 today), Bif
  import-table reach, Bs* (13/21), exception terminals (the
  RejectedIncremental-12), UpdateRecord. Coverage breadth, not structural
  admission.

## 4. Sequenced plan (what dispatches, in order)

**Leg 1 — "sound admission widening" (one lane, dispatch-ready).**
1a. *Replay-reachability probe first*: a compiler test in the jit_badarith
proof pattern — `[Send, Move #lit→X, typed arith that overflows]` through the
wired demand path, asserting the send count. If red (double send): the guard in
1b is a defect fix and the commit carries the red. If unreachable: 1b lands as
hardening with the probe as its permanent wall.
1b. *Deopt-after-side-effect guard* (class B interim): pre-pass rejects slices
where a runtime-deopt-capable instruction follows an observable side effect.
Must land **no later than** 1c, because 1c widens admission of exactly the
multi-clause shapes that contain Send-then-arithmetic.
1c. *Slicer retention A1+A2* (AMENDED per §2a.3): both slicers co-updated under
the pin; the recommended A2 shape is now two-part — (i) **interpreter fix
first**: `func_info` raises a catchable `error:function_clause` with the
instruction's MFA (the BEAM semantic; fail-first = the fc_probe fixture,
red = deadline-bounded non-termination, green = the proper error); then
(ii) **FuncInfo lowered as DEOPT** (the RecvMarker precedent — existing seam,
no new exception machinery), sound because the restarted interpreter now
raises correctly and the prelude sits before any side effect in slice order,
plus the entry-flow change (native entry branches to the export-label block,
not instruction 0, so a normal call never touches the prelude). Table
reclassified; `FuncInfo` joins `is_runtime_deopt_capable`.
**Fail-first:** `frameless.erl` and `jit_real_function` (label half) go
red→green through the demand path. `jit_real_function`'s frame half stays red
(it needs Leg 3) — the test pins its rejection as honest fallback, not silence.
**Payoff:** every multi-clause frameless tail-recursive function admits — the
largest single admission gain available at bounded cost.

**Leg 2 — JIT-003 sub-op widening, telemetry-ranked (one lane, after Leg 1).**
Dispatch blind-ranked sub-ops nothing; dispatch what rejection telemetry says.
After Leg 1, run real module corpora (the committed countdown/badarith fixtures,
the gleam_otp spike's module set once that lane lands, stdlib beams) through the
demand path and rank `UnsupportedOpcode`/`UnsupportedOperand` rejections by
frequency; JIT-003's brief takes the top of that list (expected front-runners
from the JIT-002 ground pack: TypeTest sub-ops and the exception terminals, but
the corpus decides, not the guess). Surfaces are disjoint from Leg 3's design →
can run parallel to Leg 3's design phase.

**Leg 3 — continuation call model (brief with a design phase; the centerpiece).**
Body calls with a native continuation: CP discipline, mid-function entry/resume
points, interpreter↔native frame interop, safepoint/GC rooting across the call.
Precise-resume deopt rides the same machinery — landing it retires Leg 1b's
conservatism where profitable and closes class B permanently. Red fixtures
ready: `jit_real_tail_loop.erl`, `jit_real_function` (frame half). The R3 tail
wall and the R8 pin remain the walls until this leg's own walls replace them.
Design constraints already ruled and binding: NO-POLLING; the typed-overflow
edge belongs to this arc (JIT-002 F2 ruling); deopt-equality differentials in
the jit_badarith pattern are the proof floor.

## 5. JIT-003 disposition (the dispatched question, answered)

JIT-003 is folded in as **Leg 2, not the first leg**. The evidence: zero of the
five red fixtures is blocked on a missing sub-op — the countdown fixture's
arithmetic already worked the moment `{f,0}` routing landed, and the remaining
reds are all slicer labels (Leg 1) or the call model (Leg 3). Widening sub-ops
before Leg 1 would decorate functions the slicer still rejects wholesale
(any multi-clause function). After Leg 1, the same work is aimed by real
rejection telemetry instead of a guess, at identical cost.

## 6. Boundaries (standing, restated for the arc)

Zero bytes in `crates/beamr/src/loader/encode/**` (BC-4/BC-5 assurance — this
arc is decode/JIT-side only). No custom target dirs; default gate bar per
conventions; zero new `#[allow]`; fail-first red evidence in commit messages.
The R8 slice-equality pin is co-updated only inside Leg 1c, never bypassed.
Numbering of the legs' briefs follows dispatch order per the JIT-002 numbering
ruling — leg names here, numbers at dispatch.
