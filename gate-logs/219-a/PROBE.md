# #219 (a) — the probe the fix makes possible, and an erratum in my own commit message

## ⛔ ERRATUM FIRST: commit 9c3be75's message OVERSTATES the dependency result

`9c3be75` says:

> before: cranelift-jit, tokio, mio, num_cpus, zstd all present
> after:  all absent

**`tokio` and `zstd` are NOT absent after the fix.** The accurate statement:

| crate | gone after the fix? | why |
|---|---|---|
| cranelift-jit | ✅ yes | came via beamr's `jit` default feature |
| mio | ✅ yes | via `readiness` |
| num_cpus | ✅ yes | via `threads` |
| **tokio** | ❌ **no** | arrives via `opentelemetry_sdk`, beamr's own SEPARATELY-DECLARED dev-dep — not via any beamr feature |
| **zstd** | ❌ **no** | still compiled; `cargo tree -i zstd` prints "nothing to print" for this target, so its provenance is **UNRESOLVED at this instrument** |

⚠️ **THE COMMIT MESSAGE IS NOT AMENDED, DELIBERATELY.** The battery ran at pin
`9c3be75`; rewriting the message changes the sha and orphans the evidence that
was measured against it. "The bytes that ran are the bytes that ship" outranks a
tidy message. The erratum lives here instead.

⭐ HOW I CAUGHT IT: the probe's compile list contained crates my `cargo tree`
check had reported absent. Those two instruments disagreed, so at least one of
my measurements did not mean what I said it meant. **The grep in the tree check
was too narrow — a crate absent from a filtered view is not a crate absent from
the build.** This is the same law that opened this whole sweep, turned on me:
READ THE COMPILE LIST, NOT JUST THE FILTERED VIEW.

⛔ And I do NOT have a mechanism for zstd. Saying "unresolved" is the honest
report; inventing a plausible path (build-dependency? stale target dir?) would
be a guess wearing a finding's clothes.

## THE CLAIM THAT MATTERS IS UNAFFECTED, AND IT IS MEASURED AT THE RIGHT INSTRUMENT

The lane's claim is about **beamr's enabled features**, not about which crates
appear anywhere in the graph. Measured directly:

```
cargo tree -p beamr --no-default-features --features cooperative,json -e features
  => beamr feature "test-support"
```

**That is the entire list.** No `default`, no `threads`, `jit`, `net`, `fs`,
`embedded`, `readiness`. Before the fix the same query returned the full default
set. `--no-default-features` is now genuinely in force under `cargo test`.

A dev-dependency pulling `tokio` in for its own reasons does not turn on any
beamr feature, and conflating "crate present in the graph" with "feature
enabled" is exactly the error the erratum above records.

## THE PROBE — pre-registered UNKNOWN, leaning RED. The red landed.

`cargo test -p beamr --no-default-features --features cooperative,json` → **rc 101**

**7 integration-test targets fail to COMPILE**, 15 unresolved imports:

| count | unresolved |
|---|---|
| 4 | `beamr::scheduler::{Scheduler, SchedulerConfig}` |
| 3 | `beamr::scheduler::{NativeBifs, Scheduler}` |
| 3 | `beamr::native::gate3_bifs` |
| 1 | `beamr::scheduler::dirty` |
| 1 | `beamr::native::meridian_ffi` |
| 1 | `beamr::native::gleam_ffi` |

Targets: `mfa_provenance_e2e`, `gleam_gate_e2e`, `is_function_bif`,
`suspend_result_binary`, `composition_report`, `dirty_scheduler`,
`supervision_integration` — plus one error inside
`crates/beamr/src/scheduler/mod.rs` itself.

### ⛔ THIS IS A NEW FINDING, NOT A REGRESSION, AND NOT A GATE

- **Not a regression**: this configuration was *unbuildable* before the fix. The
  suite did not stop working; it became *possible to ask*, and the answer is no.
- **Not a canon leg**: the battery at `9c3be75` is 9/9 with all axes exact. This
  probe gates nothing and must not be read as a lane failure.
- **The finding**: beamr's ~2150 tests have only ever been compiled with
  `threads` on, and a meaningful slice of the integration suite names APIs that
  do not exist without it. The suite is not portable to the configuration beamr
  actually ships to browsers.

This is the sharpened form of the sweep's candidate finding 2. It was a
*suspicion* about test reach; it is now a measured compile failure with named
targets and named imports.

### Recommended next step — NOT taken here

Making the suite portable is a real piece of work (feature-gating 7 test targets
or providing cooperative equivalents), and it is not this lane's scope. Routed,
with the list above as its ready-made worklist.
