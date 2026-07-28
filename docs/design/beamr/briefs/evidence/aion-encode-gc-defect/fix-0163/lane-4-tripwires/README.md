# AION-ENCODE-GC-DEFECT — 0.16.3 backport, lane 4: tripwire walls

**Arc/base/tear:** see `../lane-1-mechanical/README.md` — same branch off
`67f89c4`, lane 4 of the lane-at-a-time sequence, extending the lane-3
span (head `c7609b6`). Scope: the dispatch's two tripwire walls, each
mutation-proven to discriminate. Unlike lanes 1–3 these walls are GREEN
at their own commit: they pin properties that are already true so that a
future refactor breaks a test instead of a production system.

## WALL A — mailbox bump-only fact

`message_copy_is_bump_only_heap_full_is_send_error_not_collection`
(`mailbox/mod.rs` tests). The NAMED LOAD-BEARING FACT from the audit:
message copy allocates via bare `Heap::alloc` (bump-only); `HeapFull`
propagates as `SendError`; the copy path cannot collect; bump allocation
never moves existing data. The audit's mailbox SAFE verdicts
(`copy_binary` / `copy_proc_bin` / `copy_sub_binary`) rest on this fact —
a reroute through a collecting allocator would convert those sites into
crossings of the audited silent-corruption class. The wall sends an
inline binary that cannot fit the receiver heap and asserts ALL the
fact's observables: `SendError::HeapFull`, nothing enqueued, zero
old-generation residue (`old_used() == 0` — nothing collected), the
receiver's pre-existing cell intact, the sender's bytes exact.

Signature note the wall also pins: the copy path takes `&mut Heap`, not
`&mut Process` — a collecting reroute cannot even compile without a
signature change that lands in this test file.

## WALL B — gate3 binary_part owned-copy fact

`gate3_binary_part_owned_copy_survives_forced_collection`
(`native/stdlib_stubs/gc_rooting_tests.rs`). CONSUMER-LOAD-BEARING: the
endorsed AION-DROPSTART-HARDENING lane rerouted aion onto gate3's
`erlang:binary_part/3` variant (`gate3_bifs/additional.rs`), whose SAFE
verdict rests on its owned copy before the allocating call. The wall runs
the exact lane-1 probe geometry (inline `<<1..=40>>`, part 10,20, forced
collection asserted) against THIS variant and demands exact bytes — green
today precisely because of the owned copy.

## Mutation proofs (committed as DIFF FILES + red runs; never applied in any commit)

- `mutations/mutation-a-mailbox-swallow-heapfull.diff` — `send` swallows
  the copy error (`unwrap_or(Term::NIL)`) instead of propagating
  `SendError`. Kills WALL A (`expect_err` fails; a message is enqueued).
  Stated limitation, honestly: a full reroute-through-collecting-allocator
  mutation cannot be expressed at the current `&mut Heap` signature — the
  signature constraint is itself one of the wall's pinned facts.
- `mutations/mutation-b-gate3-unowned-slice.diff` — reverts the owned
  copy in `gate3_bifs/additional.rs`, passing the heap slice straight
  into `alloc_binary`. Kills WALL B with the audited face: zeroed bytes
  where `[11..=30]` is expected — byte-identical to the lane-3 probe and
  the lane-1 red.

Red runs for both mutations live in `runs/`, each labeled with the exact
tree state (walls commit + mutation applied locally) and the capture
pipeline's exit attribution.
