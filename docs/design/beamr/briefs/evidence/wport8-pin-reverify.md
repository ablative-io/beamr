# WPORT-8 build — pin re-verification at base `50d6a16`

Per the brief's verification block (binding: re-verify before writing a
line) and Waffles' build-GO rider (wake-path inventory re-verified at the
first commit, not assumed from the pack). All checks run 2026-07-23 at the
build worktree, base `50d6a16`.

## Pin table — all nine rows VERIFIED

1. **Async-NIF seam** — `register_async_nif` `lib.rs:246`;
   `HostAsyncNifs::start_callback` `:1342`; `start_promise_completion`
   `:1373` with the rejection twin (`.map(WasmAsyncCompletion::Error)`)
   at `:1387` inside `:1386-1393`; Promise-leg edge
   `request_external_turn(FailureLeg::Promise)` `:1402`;
   `wasm_async_nif_stub` `:1594`. Production lines byte-identical to the
   pack's `fb3efcf` cites — the WPORT-3 landing moved test-module lines
   only.
2. **Dead-pid tolerance** — `complete_async` `wasm.rs:266` (absent-pid
   `false` return at `:267-269`); discarded bool `let _completed =`
   `lib.rs:1397`.
3. **Native delivery** — `deliver_native_async_completion`
   `wasm_native.rs:266`.
4. **Failure legs** — `LEG_SLUGS` five, `failure.rs:63`.
5. **CI carriers** — 66 expected names counted in the array; exact-count
   line `"test result: ok. 66 passed"` at `cooperative-wasm.yml:117`.
6. **Profile** — tallies 197 at `:68`/`:78`; zero `wasm_fetch`/`wasm_kv`
   matches anywhere in the doc. NOTE: `:373` records "two open-ended
   registration seams exist beyond the 197 static rows" — the
   host-registration seams; the WPORT-8 MFAs are STATIC rows (see reading
   below), so the seal moves per R7, not through the open-seam note.
7. **Codec** — `js_value_to_owned_term` `convert.rs:25`;
   `terms_to_js_array` `:68`; `term_to_js_value` `:77`.
8. **Injected-fetch law** — "No global fetch" doc at
   `artifact_loader.rs:78`; `artifact_load_error` mold `:532`.
9. **NativeServices** — `native_services` `wasm.rs:792` (five services).

## Wake-path site inventory — MEMBERSHIP UNCHANGED

`grep -n "request_external_turn\|schedule_external_edge"` over
`crates/beamr-wasm/src/*.rs` at `50d6a16`: production callers
`lib.rs:218, :280, :332 (infallible caller), :354, :369, :455`;
definitions `:459` / `:467`; infallible body's call `:471` (SpawnEdge);
arbiter definition `:621`; deadline fire `:1267`; promise completion
`:1402`; five test-only sites (`:1919, :2165, :3288, :3660, :3669`).
Same membership shape as the WPORT-7 re-pinned inventory, line-shifted
only. The build adds ZERO sites to this list; the closing battery re-runs
this exact grep.

## Contract reading recorded at dispatch (not a fork)

R1's spec sentence "registers that module's MFAs against the existing
async-NIF trampoline path" is satisfied at VM CONSTRUCTION, not inside
`register_*_capability`: R5's refuse-before-suspend law ("works when the
named capability object is injected; stable typed refusal otherwise",
D7) requires a pre-injection call to receive
`{error, {capability_missing, Cap}}` — which the MFA can only deliver if
it is registered before any capability is injected. An
injection-time-registered MFA would answer `undef` pre-injection,
violating R5's acceptance. The two R-numbers together determine the
design: static MFA registration at construction; `register_*_capability`
stores only the capability object. This reading is determined by the
brief's own text (R5 is named a law of the surface), so it is recorded
here rather than raised as a STOP; the tear reviews it with this
evidence.
