# RUN RECORD — #103, a constant-less atom table made unrepresentable

Operator **Artemis Peach**, machine `Toms-MacBook-Pro.local`, 2026-08-12/13.
Fix commit **`07e8a60`**, parent `99f7e48`.

## The defect

`AtomTable::new()` built a table that seated nothing and started `next_index`
at 0 — the indices every `Atom::*` constant already occupies. The first name
such a table interned took `Atom::OK`, the fifth took `Atom::NIL`, and so on,
so every constant resolved to a **real but unrelated** name.

That is worse than resolving to nothing. #98 found it as a telemetry span
reporting `code.module = "put_chars"`: a plausible name from the right domain,
which survives the sanity check a missing name would fail. Sibling of #58.

## Ground, measured BEFORE anything was changed (at `99f7e48`)

| fact | measurement |
|---|---|
| `Atom::*` constants declared | 77 |
| `COMMON_ATOMS` entries | 77 |
| index range seated | 0..76, **contiguous, no gaps, no duplicates** |
| `next_index` after seating | 77 — exactly past the last constant |
| constants absent from the table | none |
| entries seated at/above `next_index` | none |

So seating is arithmetically safe: no constant's index can ever be re-issued
by `intern`.

**Call-site census:** 185 `AtomTable::new()` sites across 39 files, 7 of them
integration tests under `crates/beamr/tests/` (public API only — and
`Atom::new` is `pub(crate)`, so those files cannot construct an atom by index
at all). 440 `with_common_atoms()` sites.

## Probe 1 — is `impl Default for AtomTable` reached?

Two-arm, instrument `probe-default-reach.sh`, aborting if the impl block is
not matched verbatim exactly once.

| arm | bytes | `cargo check --workspace --all-targets --all-features` |
|---|---|---|
| A | `impl Default` deleted | **rc 0** |
| control | `impl Default` deleted + one non-test `AtomTable::default()` caller | **rc 101**, `E0599: no associated function or constant named 'default' found for struct AtomTable` |

All four workspace members compile in this command (`beamr`, `beamr-cli`,
`beamr-wasm`, `gleam-types`); `beamr-wasm` builds on host and appears in the
log. Tree restored and verified by sha, not assumed.

⇒ **Nothing in the workspace reaches `Default` today — production or test.**
It is a *latent* door, not a used one. My earlier note calling it "the door
generic code and `#[derive(Default)]` reach silently" **overstated it**, and
that correction is the reason this record exists rather than the claim.

## Probe 2 — the discriminator: does any test DEPEND on emptiness?

Prediction registered in `PREDICTION.md` **before** the run: 0-3 failures,
with the three failure shapes I could not rule out statically named in advance.

| bytes | `cargo test --workspace --all-features` |
|---|---|
| committed (`99f7e48`) — baseline | 73 result lines · **2120** passed · 0 failed · 0 ignored |
| `new()` seats the constants | 73 result lines · **2120** passed · 0 failed · 0 ignored |

**Identical. Zero tests depend on an empty table.**

⚠️ **A number that alarmed, and was a category error — recorded because the
correction is the useful part.** I first read 2120 against a banked "73/2110"
for this tree and saw a phantom +10. The 2110 in `gate-logs/98/RUN-RECORD-98.md`
is the **`tests` leg (`--features beamr/encode`)**, a different population;
`--all-features` additionally compiles the telemetry module. The result-line
count is 73 for *both* legs — same target set, different feature set — which is
exactly what made the mismatch invisible. **Axes are not a fact without the
command that produced them.** The baseline row above exists because a green on
patched bytes proves nothing without the identical command on the same box.

## The fix

`new()` seats `COMMON_ATOMS`; there is **no constructor that omits them**, so
the footgun is removed rather than renamed. `with_common_atoms()` is retained
as a delegating alias — 440 call sites, and the name states the invariant where
it is used. `impl Default` becomes correct for free, which is the reason to fix
`new()` rather than delete it (clippy's `new_without_default` requires the impl
to exist anyway).

Disposition (b) — delete/privatise `new()` — was rejected: it leaves `Default`
to be fixed separately and churns 185 sites for nothing. (c) document-and-leave
was rejected because the defect is silent by construction.

## The pins, and the falsifier

The only prior coverage of constant seating, `common_atoms_have_stable_constants`,
used `with_common_atoms()`. **The suite pinned the safe door and left the unsafe
one unpinned** — which is how this survived to be found by a telemetry span.

Two pins added: `interning_into_a_fresh_table_never_collides_with_a_constant`
(the property — seating alone is not enough if `next_index` starts inside the
constant range) and `every_public_constructor_seats_the_constants`, which gives
`Default` its first coverage in the tree.

Two-arm falsifier, `falsifier.sh`, mutation = `new()` reverted to the pre-fix
body by exact string surgery, aborting if not matched verbatim exactly once:

| arm | result |
|---|---|
| FIXED (shipped bytes) | **rc 0**, 9/9 pass |
| UNFIXED (pre-fix behaviour) | **rc 101**, 3 FAILED |

⇒ **the pins are load-bearing.** `table.rs` restored to the identical sha.

⚠️ **The falsifier's first run was WRONG and its own guard caught it.** It
restores with `git checkout --`, which restores *committed* bytes; run against
an uncommitted fix, "restore" reverted the fix rather than the mutation. The
before/after sha check printed `ABORT: file not restored` and stopped. The fix
was therefore committed first and the falsifier re-run against the bytes that
ship. **Never write a mutation harness without a before/after sha equality
check** — it is the difference between a caught error and a confident wrong
verdict.

## Battery — canon, 8 legs read from `gates.json` at run time

Pin `07e8a60`, opened 2026-08-12T21:23:09Z, closed 21:28:17Z, tree 0/0 both
ends. **COMPLETE 8/8, pin stable.** Raw: `battery-07e8a60-BATTERY.log`,
`battery-07e8a60-legs.tsv`, per-leg logs.

| leg | rc | leg | rc |
|---|---|---|---|
| 1 fmt | 0 | 5 tests | 0 |
| 2 clippy | 0 | 6 blocking-call-in-native-bif | 0 |
| 3 wasm32-check | 0 | 7 clippy-all-features | 0 |
| 4 wasm-tests | 0 | 8 tests-all-features | 0 |

**The marker is authoritative, not the exit code** — `COMPLETE` is derived from
legs scored vs declared plus pin stability and a tree census at both ends.

### Axes, against a denominator pre-registered before the run

Both cargo-test legs, each carrying its command because the axes mean nothing
without it:

| leg | command | prior (`99f7e48`) | now | delta |
|---|---|---|---|---|
| 5 `tests` | `cargo test --workspace --features beamr/encode` | 73 / 2110 / 0 / 0 | **73 / 2112 / 0 / 0** | **+2, exact** |
| 8 `tests-all-features` | `cargo test --workspace --all-features` | 73 / 2120 / 0 / 0 | **73 / 2122 / 0 / 0** | **+2, exact** |

Two tests added, two tests appear, in both legs — confirmed by name in each
leg's own output rather than inferred from the totals.

## Disclosure — the runner is NOT byte-identical to #98's

`battery-RUNNER.sh` here differs from `gate-logs/98/battery-RUNNER.sh` by
**exactly one line**: leg output is captured to `$OUT.leg<N>.log` instead of
being sent to `/dev/null`, because the axes above cannot be read from a runner
that discards them. Leg selection, scoring, the pin/tree census and the
`COMPLETE` derivation are unchanged.

- this runner: `b43254aed3c77a7e6850d39430f57963aea42421bc51aea6bf4210169ff94c76`
- #98's runner: `98beaa532740be8af38f99a4af08822c171046664df1c15f812cae80858adecc`

## Carried out of this lane, NOT fixed here

1. **`with_common_atoms()` is now a synonym for `new()`** at 440 call sites.
   Collapsing to one name is a mechanical lane of its own and is **not** claimed
   as done. It is churn, not a defect.
2. **The `--all-features` prior for this tree did not exist before today.**
   `gate-logs/98` recorded only the `tests` leg's axes even though the 8-leg
   battery ran both. Future run records should carry both, each labelled with
   its command.
