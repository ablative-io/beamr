# CRASH-2 GHOST-FRAME PROBE — the soak's 7-ghost shape REPRODUCED; mechanism identified at the bytes

Artemis Peach. Ground: beamr main `1965650` (= v0.18.2 `74e532f` for every
file cited — probe-drift check empty for jit/ + interpreter/). Probe crate:
scratchpad `ghost-frame-probe/` (path-dep on the tree, `jit` feature, ZERO
tree delta; source + run logs retained). Built by a dispatched worker
against the pre-registered brief (`ghost-frame-probe-BRIEF.md`, predictions
committed before any run); ALL counts below re-run at my own hands,
byte-consistent with the worker's. Follow-on instrumentation ran in a
scratch worktree (`wt-ghost-instr`, eprintln-only deltas, discarded from
production consideration; logs `instr-armB.err` kept).

## Arms and measured counts (total / wrapper-matching / line-less)

| arm | shape | total | wrapper | line-less |
|---|---|---:|---:|---:|
| CONTROL A/B | threshold 10^6, nothing compiles | 2 / 3 | 0 | 0 |
| ARM A | wrapper+callee COMPILED, 1 fatal badarg, no catch | 3 | 1 | 1 |
| ARM B | 6 in-VM-caught badargs through the compiled wrapper, then 1 fatal | 10 | **7** | **7** |

ARM B's record is the soak shape EXACTLY: seven identical line-less
`wrapmod:int_wrap/1` frames below the genuine line-bearing frames, raiser
(`ffimod:bif_int/1 line 102`) at the head — the 0.18.2 specimen's seven
`gleam@json:int/1` ghosts, K(caught)+1(fatal) = 7. Predictions: CONTROL
CONFIRMED, ARM A CONFIRMED (with one worded sub-branch refuted: both
functions compiled yet count = 1 — the compiled wrapper's external-call
helper enters the callee's bytecode directly, so only trampolines actually
ENTERED count), ARM B ghost-accumulation branch CONFIRMED, clear-discipline
branch REFUTED. Falsifier held: every record non-empty with a line-bearing
raiser.

## The mechanism (instrumented run + bytes)

The eprintln trail (instr-armB.err) for ARM B:

- Each of the 6 caught rounds: `capture_raw_stacktrace new_len=3` →
  `raise_exception CAUGHT: truncating stack 2 → depth 1` → `catch_end
  raw_len_at_entry=3` (cleared). **ZERO `add_compiled_frame` executions.**
- The fatal raise: capture `new_len=3` → UNCAUGHT →
  **SEVEN `add_compiled_frame` calls in one unwind** (raw_len 3→10, same
  identity each time).

Reading at the bytes, this is fully determined:

1. `jit_call_interpreted` (jit/runtime.rs:79-175) runs the body-call target
   by calling **`run_with_native_services` on the SHARED process** — a full
   nested interpreter run — after saving current_module + code_position.
2. The exception-handler stack is PROCESS-level with **no nesting barrier**
   (`push_exception_handler`/`pop_exception_handler`, process/mod.rs:672/677
   — plain Vec ops).
3. So when code inside the nested run raises and the nearest handler was
   registered OUTSIDE the nesting, `raise_exception`
   (interpreter/opcodes/exceptions.rs:176) pops that outer handler,
   truncates the stack, and returns `Jump(catch_position)` — and the NESTED
   run keeps executing the OUTER code at the catch label. The compiled
   invocation and its Rust frames (invoke_jit → native code →
   jit_call_interpreted → run) are never returned through: **one leaked
   native nesting per caught exception.**
4. The fatal unwind then exits every accumulated nested run in LIFO order;
   each leaked compiled trampoline's exception arm fires
   `jit_add_compiled_frame` once — the ghosts, one per leaked nesting.

## The corruption path (bytes-visible, one step from the measured facts)

When the displaced execution inside a leaked nested run eventually EXITS
normally, `jit_call_interpreted` returns `process.x_reg(0)` **as the
original body-call's return value into the compiled continuation**
(runtime.rs: `Ok(Exited(Normal)) => JitReturn::normal(process.x_reg(0))`),
and restores a two-minutes-stale current_module/code_position. Whatever the
process happened to be computing at that moment lands in a slot the
compiled code believes is the body-call's result. A non-integer delivered
into an integer-expected slot → exactly the soak's badarg at
`gleam_json_ffi:int/1` (`integer_to_binary` of a non-integer). This leg is
INFERENCE-at-the-bytes (not yet demonstrated live); a follow-on probe arm
(catch, then continue with good calls, assert result integrity) would
demonstrate or refute it.

## Consequences

- **This defect class requires only: jit enabled + any in-VM catch whose
  protected region crosses a compiled body-call trampoline.** Present in
  v0.17.0 and v0.18.2 both (version-containment PASS). Per-caught-exception
  native (Rust) stack growth is unbounded; trace ghosts on any later fatal
  raise; displaced-execution value delivery into compiled continuations.
- Crash-2 attribution status per the pre-registered sign-off gate:
  **hypothesis-class demonstrated at beamr's bytes with a deterministic
  repro matching the specimen's exact trace shape and timeline; the aion-
  side identity of the six caught raises in the 2-minute window remains
  unverified** (needs the atom decode / aion-side reading). Attribution not
  closed; it is now one identified class ahead of every alternative.
- Fix shape (PROPOSAL ONLY, gated): the nested run must not consume
  handlers registered outside its nesting — a nesting watermark on the
  handler stack; a raise finding no in-nest handler exits the nested run
  with exception status so propagation goes THROUGH the compiled code
  (whose exception arm then pushes its one frame correctly) and re-enters
  the outer interpreter to find the handler. Red test = ARM B as a fixture
  (expect 1 compiled frame + no leak). Not built — needs the lane gate.

## ADDENDUM — ARM C: the corruption leg REFUTED for the tail shape (finding 4, d9990956)

ARM C (one leaked nesting then displaced normal exit, vs no-catch control;
worker-built, re-run at my hands byte-identical): all three pre-registered
corruption signatures CLEAN — no re-execution (empty-stack tripwire never
fired: the body-call return frame is consumed by the catch unwind's
truncation, and each unwinding leaked trampoline's return_ maps empty-stack
→ Exit(Normal), core.rs:250-253), no wrong delivered value (the phantom
x0 handoff lands in the register already holding that value; in-VM
validation passed), no secondary exit. Scope: tail-shaped trampolines —
the only shape the tier compiles.

Consequences recorded loudly: the §"corruption path" section above is
REFUTED for this shape. The leak's demonstrated effects are ghost frames +
unbounded native-stack growth per caught exception. The soak's first caught
badarg PREDATES any leak ⇒ the non-integer was already in the value ⇒
cause upstream. Aion-side fit identified read-only:
aion_flow_query_pump.erl catches query-handler raises by design (reply path
outside the catch, its :113 comment) — six swallowed badargs + one fatal
through compiled json:int matches the structure; which value/traffic =
open, aion-side.
