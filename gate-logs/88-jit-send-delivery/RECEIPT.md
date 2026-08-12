# #88 JIT-SEND-DELIVERY — release battery receipt

Pin: commit `0a5d1ac298d13e3ffc8756dd2feafcb8c1d5f4a6`, tree
`2bb4e720819cc4df642cbeaa006ca8acf24f5303`, branch `artemis/jit-send-delivery`
(off `2551841` = origin/main at 0.18.1). Runner: `runner.sh` beside this file —
the bytes that ran.

## Verdict: 6/6 rc 0. COMPLETE marker DERIVED (legs_declared=6, legs_scored=6).

| leg | name | rc |
|---|---|---|
| 1 | fmt | 0 |
| 2 | clippy (`-D warnings`) | 0 |
| 3 | wasm32-check | 0 |
| 4 | wasm-tests | 0 |
| 5 | tests (workspace) | 0 |
| 6 | blocking-call-in-native-bif | 0 |

Legs were read FROM the committed `gates.json` at run time, so this battery
cannot drift from CI by transcription. Denominator (6) came from that file and
was asserted non-empty, numeric, ≥1, and stderr-clean before any leg ran.

## Named axes, reconciled EXACT against the 0.18.1 prior

Prior at `2551841` (#44 receipt): **result-lines 72 / passed 2080 / failed 0 /
ignored 0**. This run: **result-lines 73 / passed 2088 / failed 0 / ignored 0**.

* result-lines **+1** — one new test binary, `tests/jit_send_delivery_gate.rs`.
* passed **+8**, every one named:
  * six lib walls in `jit::runtime_message::tests` —
    `compiled_cross_process_send_routes_through_local_send_facility`,
    `compiled_self_send_ticks_sender_clock_and_delivers`,
    `non_pid_destination_aborts_with_the_interpreter_badarg`,
    `remote_destination_without_distribution_aborts_with_noconnection`,
    `replay_mismatch_restores_clock_and_aborts_with_interpreter_error`,
    `missing_facility_falls_through_exactly_like_the_interpreter`;
  * two integration arms — `jit_compiled_cross_process_send_delivers` and
    `interpreted_cross_process_send_delivers_control`.
* failed and ignored unchanged at 0.

Nothing else moved. No test was renamed, retired, or filtered out.

## Tree check — AMENDED FORM (three artifacts, both ends)

`tree.dirty.raw` is `git status --porcelain` verbatim; `tree.dirty.filtered`
drops exactly one line, `?? .claude/skills/` (untracked tooling, never staged);
`tree.dirty.rc` is the COUNT of surviving lines, written with `wc`, never with
`grep -c` — grep exits 1 on a zero count, which would turn a genuinely clean
tree into "the producer failed". The raw file is kept so the filter itself can
be audited: a filtered artifact alone cannot show what it removed.

Pre-run **0**. Post-run **0** (`*.post` files). The post-run check is not
ceremony: a leg that wrote into the worktree would otherwise ship bytes the pin
never covered.

## Lane walls and the CI-only extra

| check | rc |
|---|---|
| `cargo test -p beamr --lib jit_send` | 0 |
| `cargo test -p beamr --test jit_send_delivery_gate` | 0 |
| `cargo check -p beamr --no-default-features --features cooperative,json` | 0 |

The last is the §5 carry from the 0.17.0 battery — the one thing CI uniquely
covered that no declared leg does.

## What this battery does NOT say

It says the tree at `0a5d1ac` is green and that the delivery walls pass. It says
**nothing** about the aion incident: the send-drop defect's attribution to that
crash was RETRACTED earlier (the five crashed compiled functions do not call the
send helper). The case for shipping this fix rests on silent message loss and
replay-clock corruption **on their own merits**, evidenced by the packaged-bytes
census across the affected version range — never on the retracted attribution.

Boundary: `df` floor 25,000,000 KiB asserted before every leg; see
`boundary-df.txt`. Teardown: `du` target 12,750,536 KiB, `df` free 56,226,820 KiB
at close.
