# beamr — Assets Pack (pipeline toolkit)

*What a reviewer, verifier, or implementer agent needs to do beamr work
well. Written for dispatch through the norn/aion pipeline. Companion to
`beamr-ledger.md`.*

## 1. Domain-specific review prompts

**GC/rooting review** (any change touching BIFs, opcodes, heap, or terms):
- Does any BIF hold a heap-derived `Term` across an allocation without
  parking it in an x-register / `with_rooted`? (Args are NOT GC roots.)
- Does every `ensure_space` call pass an honest `live` count? (`put_list`,
  `put_tuple2`, `update_record` pass `X_REG_COUNT` for a documented reason.)
- If a rooted field was added to `Process`: are `roots()`,
  `replace_roots()`, AND `rebase_roots_from` all updated, in the same
  iteration order?
- New boxed layout? Verify the four coordinated edits: writer + accessor,
  `BoxedTag::from_bits`, GC's `rewrite_copied_object`, and
  `constant_pool::rebase_boxed_block_terms`.
- Any ProcBin path: is every copy `retain`ed and every reclaim `release`d?
  Does `virtual_binary_heap` accounting stay balanced?

**Scheduler/concurrency review** (adversarial, per the existing gate-bar
practice):
- Slot-lock discipline: is `wake_process`/`deliver_down_messages` ever
  reachable while a slot lock is held?
- Executing-shadow correctness: do new delivery paths write to pending
  metadata when the slot is `Executing`, and does store-back merge them?
- Suspension identity: is every completion published under a call id, and
  does the consumer drop stale ids and re-check liveness after insert?
- Tombstone ordering: native slices check tombstones FIRST; bytecode paths
  check after every store-back. Does the change preserve both?
- Does anything wake a gated (host-await/dirty) suspension on message
  arrival? That double-submits side effects — hard reject.

**JIT review**:
- ABI: does generated code touch `Y(n)`? The register file is 1024 words of
  X only until the ABI brief lands — reject any Y-relative access.
- Every new lowering: is the differential proptest extended to cover it?
- Deopt safety: are partial register writes before the deopt point
  idempotent under bytecode re-execution?
- Safepoints: allocation sites recorded? (Offsets are logical indices
  today — flag any change that pretends they are PC offsets.)
- Generation keying: does the change preserve exact-generation cache lookup?

**Loader/ETF review**:
- Is every new decode path charged against `DecodeBudget` (nodes, bytes,
  atoms, depth)? Unbudgeted paths reopen zip-bomb/OOM holes.
- Import-table integrity: does every import keep its slot even when
  unresolved/denied?

**Term-representation review**:
- NIL is the exact word `0b011`; tag-only tests are wrong.
- Untrusted values through `try_small_int`/`try_pid` only (infallible
  constructors truncate in release).
- Small-int/bignum canonical form preserved on every new integer path
  (`=:=` breaks otherwise).

## 2. Verification methodology

- **The gate bar, always**: `cargo fmt --check`; `cargo check -p beamr`;
  `cargo test -p beamr --lib` + `--test '*'`; `cargo clippy --all-targets
  -- -D warnings`. No file >500 code lines (400 entrypoints). No
  `unwrap()`/`expect()` outside `#[cfg(test)]`.
- **JIT work**: differential proptest (interpreter vs JIT on generated
  programs) is mandatory, not optional; criterion benches
  (`benches/jit_comparison.rs`) must not regress.
- **Distribution work**: multi-OS-process kill-9 testing, not unit tests.
  The existing `distribution_e2e` / `distribution_mesh_handshake` /
  `pg_distribution_e2e` shapes are the template. Simultaneous-dial and
  down-but-unreaped-redial cases must be exercised explicitly.
- **GC/refcount work**: `memory_stability.rs` and `no_box_leak.rs` are the
  regression harnesses; extend them rather than writing parallel ones.
- **Cooperative/wasm work**: native test gate covers the shared surface;
  browser-only behavior needs `wasm-pack test` (rAF/Promise round-trips
  cannot be verified headless-native — do not fake them).
- **Fixture discipline**: new `.beam` fixtures are committed binaries with
  their `.erl`/`.gleam` source beside them and a compile comment. Never a
  build step.
- **Suspension/native work**: the `suspend_reexec` / `suspend_wakeup` /
  `suspend_result_binary` tests encode the protocol's hardest cases
  (tail-call parks, lost-wakeup races, binary results). Read them before
  touching the protocol; extend them with any new case.

## 3. Design documents required before implementation

| Work item | Prerequisite doc |
|---|---|
| Capability parameterization | Joint capability model doc with frame F-1b (grant shapes, enforcement points, wasm interaction with ruling #5) |
| Multi-subscriber conn-down hook | One-page API sketch (ordering + reentrancy guarantees for subscribers) |
| JIT frame ops | Register-file ABI change doc (X+Y layout, who allocates, deopt implications) |
| Replay recording | Determinism-altitudes doc (beamr events vs liminal replay vs aion history — joint with those domains) |
| Browser transport | Framing-over-WS design (handshake mapping, keepalive semantics) |
| AOT | Full design phase; do not brief from the north-star doc alone |

## 4. Specialized agents worth having

- **beamr-gc-reviewer**: knows the rooting rules above; reads any diff
  touching `gc/`, `process/heap`, `native/` allocation paths.
- **beamr-concurrency-adversary**: red-teams scheduler/suspension changes
  against the invariant list; the gate bar already prescribes adversarial
  review for delicate concurrency — make it a standing agent.
- **beamr-jit-reviewer**: Cranelift-literate; owns the differential-test
  extension check and the ABI rules.
- **differential-runner**: executes interpreter-vs-JIT property runs at
  scale on demand (cheap to automate, catches what code review cannot).

## 5. Implementation constraints (standing orders)

1. The gate bar is non-negotiable; run it before reporting done.
2. No regression to the bytecode path — the interpreter is the reference
   semantics; JIT/AOT match it, never the reverse.
3. New BIFs: registered in the static tables (no ad-hoc registration),
   assigned an honest `Capability`, dirty-kind if they can block, and GC
   rooting rules applied.
4. New unsafe blocks carry a written justification comment and trigger the
   concurrency adversary review.
5. Public API changes (anything liminal/haematite/aion import) get a
   downstream-impact note in the PR — the two-tier consumer map in
   `stack-integration.md` says who to check.
6. Follow `docs/terminology.md` exactly in code comments, briefs, and
   reviews — the project's vocabulary is pinned and drift is treated as
   error.
