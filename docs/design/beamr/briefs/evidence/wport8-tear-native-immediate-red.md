# WPORT-8 tear finding — native-caller hang on immediate-success completions (RED evidence)

**Finding source:** WPORT-8 build tear, Artemis Peach, 2026-07-23 — the ONE
fail-class finding holding the PASS. **Fix ruling (Artemis, binding):**
red-first three walls (native put-success, native get-absent, native
delete-success, each deadline-bound), then the `OwnedTerm::immediate`
fallback in `build_completion`; deviations agreed with the domain owner
first.

## The defect, at `14f0cfc` bytes

`validate_response` normalizes KvPut/KvDelete success to
`Ok(JsValue::TRUE)` and KvGet-absent to `UNDEFINED`; `success_term` maps
both to IMMEDIATE atoms with ZERO detached allocations;
`take_detached_result` returns `None` whenever `detached_allocations` is
empty (`crates/beamr/src/native/context/mod.rs:1703-1712`) — a legitimate
immediate root is indistinguishable from construction failure at that
call; `build_completion`'s `?` propagates the `None`; `finish_request`'s
else-arm returns with NO delivery and NO counter. A parked native handler
awaiting any of those three outcomes hangs forever — happy path, native
caller class. Bytecode callers are safe (the outer tagged tuple always
allocates); native error legs are safe (the detail binary allocates).

The house pattern for the valid immediate-root case already exists:
`crates/beamr-wasm/src/convert.rs:34` and `:62` use
`take_detached_result(term).unwrap_or_else(|| OwnedTerm::immediate(term))`.

**Rider (same arm):** the `finish_request` else-comment claimed "deliver
the honest rejected leg rather than nothing" while the code delivers
nothing, and claimed the arm unreachable while this finding reaches it —
the comment is trued in the fix commit.

## The three walls (this commit, RED)

Each wall bounds the await at two macrotask turns — the same bound the
existing verbatim-contract wall uses — and asserts the recorded envelope,
so the red state fails BOUNDED instead of hanging (house test
discipline: a wall that hangs proves nothing).

- `native_kv_put_success_delivers_immediate_true_envelope` — native
  `wasm_kv:put/2` → `{ok, true}` envelope, then a bytecode get proves the
  store took the value (round trip).
- `native_kv_get_absent_delivers_immediate_undefined_envelope` — native
  `wasm_kv:get/1` on an absent key → `{ok, undefined}` envelope.
- `native_kv_delete_success_delivers_immediate_true_envelope` — seeded
  bytecode put, native `wasm_kv:delete/1` → `{ok, true}` envelope, then a
  bytecode get proves the key is gone.

CI carriers move in this commit with the names (WPORT-7 discipline):
77 → 80, three names added. CI at THIS commit is expected RED — the
red-first pair greens at the fix commit (the aac5c4a→2cbd3ae precedent).

## Red run (walls committed in this commit, code unfixed)

Environment: rustc 1.95.0 (59807616e 2026-04-14), Node v26.5.0,
wasm-bindgen-test-runner 0.2.123, darwin arm64; tree = this commit minus
this evidence file's fix; filter `native_kv`.

```text
running 3 tests
test capability::tests::native_kv_put_success_delivers_immediate_true_envelope ... FAIL
test capability::tests::native_kv_get_absent_delivers_immediate_undefined_envelope ... FAIL
test capability::tests::native_kv_delete_success_delivers_immediate_true_envelope ... FAIL

---- capability::tests::native_kv_put_success_delivers_immediate_true_envelope output ----
    beamr-wasm panicked: assertion `left == right` failed: native put success delivers the immediate {ok, true} envelope
      left: []
     right: [Array [String("ok"), Bool(true)]] (crates/beamr-wasm/src/capability_tests.rs:844:5)

---- capability::tests::native_kv_get_absent_delivers_immediate_undefined_envelope output ----
    beamr-wasm panicked: assertion `left == right` failed: native get-absent delivers the immediate {ok, undefined} envelope
      left: []
     right: [Array [String("ok"), Null]] (crates/beamr-wasm/src/capability_tests.rs:869:5)

---- capability::tests::native_kv_delete_success_delivers_immediate_true_envelope output ----
    beamr-wasm panicked: assertion `left == right` failed: native delete success delivers the immediate {ok, true} envelope
      left: []
     right: [Array [String("ok"), Bool(true)]] (crates/beamr-wasm/src/capability_tests.rs:893:5)

test result: FAILED. 0 passed; 3 failed; 0 ignored; 77 filtered out; finished in 0.04s
```

The empty `left: []` sinks ARE the finding: the completion was dropped,
nothing resumed the parked handler, and only the bounded await turned the
hang into a visible red.
