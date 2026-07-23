# WPORT-8 build STOP — completion-delivery contract fork at the pinned bytes

**Status:** BUILD HELD at `515463a` (pin-evidence commit; zero adapter code
written). STOP condition 4 ("any divergence between bytecode and native
callers through the shared registration path is a STOP") + RUNBOOK
stop-and-ask at contract forks. Finding, evidence, and options below;
resolution is the reviewer of record's.

## The finding

The brief's R2/R3 contract — the BEAM caller receives
`{ok, Map}` / `{error, {SlugAtom, DetailBinary}}` as VALUES, uniform for
bytecode and native callers — cannot be delivered through any single
`WasmAsyncCompletion` arm choice, because the existing seam's delivery
semantics DIVERGE by caller type at `50d6a16`:

- **Bytecode** (`apply_async_completion`,
  `crates/beamr/src/scheduler/wasm.rs:830-849`):
  `Ok(term)` → the RAW term lands in x0 (`:834-840`, no `{ok, _}` wrapper);
  `Error(term)` → x0 set, then **`Some(ExitReason::Error)` — the caller
  process DIES** (`:841-847`). An adapter error delivered through the
  Error arm kills a bytecode caller instead of returning
  `{error, {Slug, Detail}}`.
- **Native** (`deliver_native_async_completion`,
  `crates/beamr/src/scheduler/wasm_native.rs:266-294`):
  `Ok(term)` → mailbox message `{ok, term}`; `Error(term)` → mailbox
  `{error, term}` (`:271-274`) — the shape the brief cites as the target
  contract.

Consequences per arm strategy:
- Bridge sends `Ok(tagged_tuple)` always → bytecode sees the exact
  contract; native mailbox sees `{ok, {ok, Map}}` / `{ok, {error, …}}` —
  double-wrapped, not the cited contract.
- Bridge sends `Ok(map)` / `Error(slug_tuple)` → native sees the exact
  contract; bytecode gets a raw map on success (no `{ok, _}`) and DEATH
  on every adapter error — R2's "silently maps errors to success" wall's
  evil twin.

The registration path itself is shared and does not diverge (the
static-registration reading, confirmed by the reviewer of record
2026-07-23). The fork is in completion DELIVERY.

## Honest options

**Option A — uniform inner value through the Ok arm only.** Bridge always
delivers `Ok(tagged)` where tagged = `{ok, Map}` | `{error, {Slug, Detail}}`.
Bytecode: exact contract, adapter errors never kill the caller. Native:
one extra outer transport wrapper (`{ok, tagged}`) — handlers match the
inner value; the brief's cited native shape is NOT what arrives verbatim.
Zero `crates/beamr` changes. The divergence becomes a documented wrapper,
not a semantic fork.

**Option B — caller-aware arm selection at request time (recommended).**
The capability MFA executes in the CALLER's context; the bridge records
the caller type there (`ProcessContext::pid()`
`crates/beamr/src/native/context/mod.rs:649`; `process_mut()` `:769`;
`Process::is_native()` `crates/beamr/src/process/mod.rs:443`) into its
in-flight entry, and at completion picks the arm per caller: bytecode →
`Ok(tagged)` (x0 = exact contract value); native → `Ok(map)` /
`Error(slug_tuple)` (mailbox = exact contract per the cited
`:271-274`). BOTH caller types observe the brief's contract exactly;
the fork is absorbed inside the bridge and pinned by walls on both
paths. Zero `crates/beamr` changes. Residual risk: whether the
NATIVE-caller `ProcessContext` reaching `start_async_nif`
(`native_process.rs:405-419`) exposes the process/type is an unruled
invariant — it must be pinned by a wall at build (if the native context
carries `process: None`, that absence itself distinguishes the caller
type, but the wall must pin whichever invariant holds).

**Option C — extend the seam in `crates/beamr`.** A verbatim-delivery
completion arm (uniform: x0 = term for bytecode; mailbox = term for
native, no wrapper, no death). Cleanest semantics and closest to the
brief's uniform-contract intent, but violates STOP 3 (zero
`crates/beamr` production changes) and needs the reviewer's explicit
word to open the crate. Byte-identity note if opened: the cooperative
scheduler modules are cfg'd out of threaded builds, so the threaded
byte-identity law holds trivially; the change would still widen the
tear surface to `crates/beamr`.

## Build-worker recommendation

Option B: it is the only shape where BOTH caller types observe the
brief's contract verbatim with zero `crates/beamr` motion, and its one
soft spot (the native-context invariant) is pinnable by a wall. Option A
if the reviewer judges that invariant too soft to build on. Option C
only on an explicit crate-opening ruling.

Build stays HELD until the reviewer of record rules.
