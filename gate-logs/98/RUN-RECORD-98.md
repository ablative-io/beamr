# RUN RECORD — #98: four telemetry scheduler tests

Operator **Artemis Peach**, 2026-08-13, beamr main at `1795daf`.

#98 posed two candidates and warned against assuming the comfortable one:
**(a)** stale test setup — shutdown-before-execute no longer yields `Requeue`;
**(b)** a live scheduler change — a looping process exiting instead of being
preempted, which would be a real fairness/correctness defect.

**Both were wrong, and the discriminator said so directly.** The answer is a
stale test setup, but by a mechanism neither candidate named: `shutdown` is
irrelevant, and the fault is the **entry instruction pointer**.

## The discriminator

A temporary probe ran the same shapes with the shutdown call present and
absent, and at three entry points. It was reverted before any fix was written;
these are its numbers.

| module body | shutdown first | entry ip | outcome |
|---|---|---|---|
| self-loop `CallOnly` | yes | 0 | `Exited(Error, nil)` |
| self-loop `CallOnly` | **no** | 0 | `Exited(Error, nil)` |
| `Return` | yes | 0 | `Exited(Error, nil)` |
| `Return` | **no** | 0 | `Exited(Error, nil)` |
| self-loop `CallOnly` | yes | **1** | **`Requeue`** |
| self-loop `CallOnly` | yes | **2** | **`Requeue`** |
| `Return` | yes | **1** | **`Exited(Normal, nil)`** |
| `Return` | yes | **2** | **`Exited(Normal, nil)`** |

- **Candidate (a) as written is REFUTED**: the no-shutdown arms are identical.
- **Candidate (b) is REFUTED**: entered correctly, a looping process *is*
  preempted and requeued, exactly as the tests assert. There is no scheduler
  fairness defect here.
- The `Return` arm is the positive control in the other direction: a module
  that should exit `Normal` also exited `Error` at ip 0, so the fault could not
  be about yielding.

## Mechanism

All four tests entered at `instruction_pointer: 0`, which in their own module
layout is the `FuncInfo` instruction. `func_info` is the multi-clause dispatch
**landing pad**: reaching it means no clause matched. Commit `5bcd529`
(2026-07-23, first shipped in **v0.16.3**) changed it from set-MFA-and-Continue
to raising a catchable `error:function_clause`, fixing an infinite-loop defect.
From that commit on, every one of these four tests killed its process on the
first instruction, before the slice ran.

Production is unaffected: `Scheduler::spawn_in` resolves
`entry.module.label_ip(entry.label)` and enters *after* the landing pad.

**They never re-ran, so nobody saw it.** These four are the only direct
`execute_slice` call sites and all four are `#[cfg(feature = "telemetry")]` —
the exact gap #97 found and #96 is chartered to census.

## A second defect, found only because the first was fixed

With a correct entry ip, three tests passed and the fourth failed differently:
its span carried `code.module = "put_chars"`. Not cross-talk — it reproduced
with the test running alone.

`Process::current_mfa` → `Module::mfa_at_ip` → `function_at_ip`, which reads
`Module::function_table`. The `test_module` scaffold built
`function_table: Vec::new()`, so **every scaffold module returns `None` for
every ip** and the span fell back to `Atom::NIL`.

`Atom::NIL` then resolved to `"put_chars"` because these tests build their
table with `AtomTable::new()`, which preloads **nothing** and starts
`next_index` at 0 — so interned names are assigned 0,1,2,… and collide with
every `Atom::*` constant. `Atom::NIL` is index 4; the fifth atom interned in
that run was `put_chars`. `AtomTable::with_common_atoms()` is the constructor
that seats the constants.

Fix applied: `test_module` now derives its `function_table` from the `FuncInfo`
instructions, mirroring the loader. Scaffolds with no `FuncInfo` still yield
`None`, so tests that assert `Atom::NIL` MFAs are untouched.

## Result

- `cargo test -p beamr --features telemetry --lib scheduler::` — **279 passed, 0 failed**
- `cargo test --workspace --all-features` — **1842 passed, 0 failed**
  (was 1838 passed / 4 failed)

This unblocks the `tests-all-features` canon leg that #97 declared and
deliberately did not add while red.

## Carried out of this lane — NOT fixed here

1. **`AtomTable::new()` is a public footgun.** It seats no constants, so every
   `Atom::*` silently resolves to an unrelated name — a wrong answer, not a
   missing one. 185 `new()` sites vs 439 `with_common_atoms()`. Production
   construction uses `with_common_atoms`; the exposure is embedders and tests.
   Same family as #58. Wants its own lane and a ruling, not a sweep here.
2. **`Scheduler::spawn_process` enters at ip 0**, and its doc says "at the
   beginning of a module". For any module whose code begins with `FuncInfo` —
   which is what the loader produces — that is the landing pad. Every call site
   today is a test. `pub`, so embedder-reachable.

## Battery at the fix commit

Ran at `3c1e08f`, the seven canon legs **read from `gates.json` at run time**
(#75's arrangement — never transcribed). Runner: `battery-RUNNER.sh`; it aborts
on an empty leg command rather than scoring a leg that ran nothing, which is
the failure #75 caught by the denominator.

**COMPLETE 7/7, pin stable, tree 0/0.** Raw: `battery-3c1e08f-BATTERY.log`,
`battery-3c1e08f-legs.tsv`. Opened 2026-08-12T17:10:33Z, closed 17:12:06Z.

| leg | rc |
|---|---|
| fmt | 0 |
| clippy | 0 |
| wasm32-check | 0 |
| wasm-tests | 0 |
| tests | 0 |
| blocking-call-in-native-bif | 0 |
| clippy-all-features | 0 |

Canon `tests` leg axes (`cargo test --workspace --features beamr/encode`):
**result-lines 73 · passed 2110 · failed 0 · ignored 0.**

⚠️ **No delta is claimed against any earlier axis reading.** The last recorded
prior at my seat is 72/2067/0/0 at `b96be7a` (#75), and several lanes have
landed between — the populations are not comparable and the difference is not
attributed here.

Separately, `cargo test --workspace --all-features` — the leg this lane
unblocks — is **1842 passed, 0 failed**, against the 1838/4 recorded when it
was declared known-red.
