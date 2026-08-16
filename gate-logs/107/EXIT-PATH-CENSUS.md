# CRASH-2 FIX — exit-path census for the nested-run handler floor

Artemis Peach. Ground: beamr `1965650` + the working-tree fix. Required by
Waffles' ruling 1 (dm 6580d965): field-on-`Process` with manual save/restore is
approved, and the price of the manual idiom is that the delivery report
enumerates every exit from the guarded region with its restore named at the
line. This is also his answer to my question 3 (nesting depth > 1): the
"restored on every exit path" claim becomes a checked enumeration rather than an
assurance.

## The guarded region

`crates/beamr/src/jit/runtime.rs`, `jit_call_interpreted` (function spans
:79-209):

```
:176    let saved_handler_floor = process.nested_handler_floor();
:177    process.set_nested_handler_floor(process.exception_handler_count());   <- INSTALL
:178    let result = run_with_native_services(process, current_module, registry, services);
:179    process.set_nested_handler_floor(saved_handler_floor);                 <- RESTORE
```

The region between install and restore is **one line**.

## Mechanical census (not eyeballed)

Instrument: a python pass that locates the install and restore lines **by their
own bytes** (not by line number, so it cannot drift), then counts control-flow
escapes strictly between them. Output at the shipped bytes:

```
install line: 177          restore line: 179
lines strictly between install and restore: 1
  178:     let result = run_with_native_services(process, current_module, registry, services);
`return` occurrences in region: 0
`?` occurrences in region: 0
total `return` statements in function: 15
returns BEFORE install: 15
returns AFTER restore:   0
```

## Enumerated exits

| # | Exit | Where | Restored? |
|---|---|---|---|
| 1-14 | 14 `JitReturn::deopt(...)` guards (null process/context, bad module or import index, arity mismatch, non-Code import target, missing target module, unresolvable export ip, …) | :86-153, all BEFORE the install | N/A — floor never installed on these paths |
| 15 | reductions-exhausted `JitReturn::yield_` | :162-165, BEFORE the install | N/A — installed AFTER this check, deliberately, so a yield cannot carry a floor out |
| 16 | `Ok(Exited(Normal))` -> `JitReturn::normal` | :185-187 | YES — :179 ran unconditionally first |
| 17 | `Ok(Exited(_))` with an exception -> `JitReturn::exception` | :188-193 | YES — same |
| 18 | `Ok(Exited(_))` / `Waiting` / `DirtyCall` -> deopt | :194-196 | YES — same |
| 19 | `Ok(Yielded)` -> yield | :197-200 | YES — same |
| 20 | `Err(_)` with an exception -> `JitReturn::exception` | :201-206 | YES — same |
| 21 | `Err(_)` -> deopt | :207 | YES — same |

The five interpreter outcomes and two error arms are all *match arms on a bound
`result`*, so control reaches :179 before any of them. There is no `?` in the
region: `run_with_native_services`'s `Result` is bound, never propagated.

**The one uncovered path, stated rather than omitted:** an unwinding panic
through `run_with_native_services` would skip :179. That is out of scope by the
same rule that already leaves `saved_module`/`saved_position` unrestored on the
same path — a panic through the interpreter is a VM-fatal condition, not a
control-flow outcome. The floor is no worse off than the two values whose idiom
it copies.

## Depth > 1 (ruling 3)

The floor is a scalar saved and restored around each nesting, so nestings
compose: an inner `jit_call_interpreted` saves the outer run's floor, raises it
to the handler count at inner entry, and puts the outer value back on the way
out. The census above is what makes "puts it back" checkable rather than
asserted, and
`each_trampoline_records_one_frame_at_nesting_depth_two`
(crates/beamr/tests/jit_nested_run_handler_leak.rs) exercises two real nestings
end to end: one frame per trampoline at K=2 and K=6, and zero frames for the
interpreted middle.

## Pre-existing asymmetry, disclosed and NOT touched

The reductions-exhausted return at :162-165 leaves `current_module` and
`code_position` pointing at the callee — it returns *after* :156-160 set them
and *before* the restores at :180-183. That is pre-existing behaviour, plausibly
deliberate (the yield is meant to resume into the callee), and outside this
lane. I did not change it; I placed the floor install after it so the new state
cannot inherit the same shape.
