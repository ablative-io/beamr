# Evidence of record — EXIT-001: notification-only per-pid exit watches

Task `7a1242eb-742b-48be-ba7e-d961f2c49f04`; dispatch `2ce539f6` (Artemis
Peach → Diana Plum …c9fb, 2026-07-29 16:29Z); brief pinned at
`docs/design/beamr/briefs/EXIT-001.json` @ `8f2b7c3` (freeze `1c6d886` +
three amendments). Build base: main `192e4a4`. Branch:
`diana/beamr-exit001-7a1242eb`, worktree `.wt-exit001-7a1242eb`,
machine: Annabel's box, operator: Diana Plum (…c9fb).

**Hash-framing convention of record (ruled by Artemis 16:52Z, forward
from then):** dispatch/ruling integrity hashes cover the payload with any
trailing newline STRIPPED, and the reported byte count is of the stripped
form — transports trim trailing LF unpredictably, so no-LF is the only
framing both sides can always reconstruct. (The 16:49Z ruling relay,
2103 B with-LF, is the last artifact of the old ambiguity; both its and
the dispatch's echoes MATCHED.)

**Path mapping note (OBS-007 shape):** the dispatch names this directory
`evidence/exit001-7a1242eb/`; the brief's internal `files` lists say
`evidence/exit-001/`. The dispatch is the later ruling and governs;
every brief reference to `evidence/exit-001/…` resolves here.

## Pin-shift re-verify (required before code; done before code)

Every line pin in the brief's purpose/task sections re-read at `192e4a4`:
**all unmoved, zero forward corrections** — hygiene (the only delta from
`74c7d3c`) touched CLI and docs only. Verified: `exit_tombstones.rs`
:60-69/:78-92/:128-132/:139-149/:178-235 (incl. `drop(order)` :229,
publish :231-233)/:238-241/:257-266; `exit_events.rs` :122/:144-151/
:167-199/:246-249; `execution.rs` :142-156/:177-179/:194-196/:213-215;
`mod.rs` :126.

## Commit chain

| commit | what |
|---|---|
| `26a49c1` | WALLS RED at the skeleton: 12 red walls, faces verbatim in the commit body; characterization (single-use subscription) GREEN at base with intent doc; test-only gate split (`wait_for_publication`/`release_publication`) |
| `6a3ceec` | Implementation: fire path appended after publish (outside the mutex), register-then-check, typed public seam, R4 mechanism pin, CHANGELOG. 27/27 green; W2+W3 10× isolated = 20/20 |
| `b638603` | R5 records: `sibling-run-until-exit.md`, `liminal-handoff.md` |

## OQ rulings (taken here, as the dispatch instructs; input weighed, not inherited)

**OQ-A — fire OUTSIDE the writer mutex. RULED: OUTSIDE.** Reasoning of
record: (i) the two axes are distinct — install-before-publish is the
invariant Wall 1 pins and the race ruling's grounds cite; inside-vs-outside
the mutex is a separate question that publication-outside-the-lock already
answers for the existing event, so OUTSIDE preserves every ruled ordering
while INSIDE would change the hot exit path; (ii) INSIDE's stronger
invariant (a pre-lock registrant always gets a real fire) buys nothing any
current consumer needs — register-then-check makes the duplicate harmless,
which is cheaper than lengthening a mutex every process death by
O(watches-on-pid) sends; (iii) Vesper's aion-side input agrees, and her
drainer's requirement (append-after-publish) is met by construction.
The fire site sits after `events.publish` at the end of `insert_inner`,
after `drop(order)` — nothing inserted between installation and the
existing publish, nothing reordered (REVIEW POINT 1 honored; asserted by
`publication_order_outcome_tombstone_event_then_watch_fire`).

**OQ-B — the no-record answer's shape. RULED: TYPED registration answer**
(`ExitWatchState::Live / AlreadyExited / NoRecord`). Reasoning of record:
(i) liminal's §5.5 asks registration itself to report already-dead — the
typed answer does it at the call site and lets the waiter skip arming a
deadline; (ii) the hard part was making `NoRecord` truthful, and the
ground supplied the proof: finalization installs the durable record
BEFORE removing the pid from the process table
(`execution/core.rs` `cleanup_exited_process` → `finalize_exited_process`),
so the sequence register → record-check → table-check → record-RECHECK is
race-free — a second record miss proves the pid never ran; (iii) the
alternative (always-a-watch, first receive carries the state) keeps one
code path but re-blurs "still running" vs "unknown" at exactly the seam
the honesty rule exists for. Vesper's input agrees; ruled on the
reasoning above, not on the agreement.

## Per-live-watch byte cost (R2 acceptance)

Registry side: one `(u64, Sender)` vec entry — 16 bytes on the supported
64-bit layout — plus its share of the pid's `Vec` buffer and one DashMap
entry per WATCHED PID (not per watch). Channel side: one bounded(1)
crossbeam channel allocation per watch (fixed header + one
`(u64, ExitReason)` slot), freed when both ends drop; the `ExitWatch`
handle itself is 4 words. Steady-state residue after fire or drop: ZERO —
unlike the deliberate 40-byte token, watches leave nothing behind
(`w4_registry_holds_no_entries_after_abandonment_or_fire`).

## Mutation evidence (`mutations/` + `runs/`; diffs NEVER applied to the tree)

Convention per lane-4 tripwires: each wall killed by its own minimal
mutation, red run recorded. Status at the compile hold (Cally's
claim-convention ruling, 16:49Z — runs resume under preflight):

| mutation | kills (designed) | status |
|---|---|---|
| `m-live-on-table-miss` | unknown-pid wall (Live face) | **RUN: red, exit 101** (`runs/red-m-live-on-table-miss-unknownpid.txt`). **Observation of record:** it does NOT kill W1 — the store-level record check answers first. W1 has two-layer defense; the non-kill is filed (`runs/obs-m-live-on-table-miss-does-not-kill-w1.txt`), not discarded. |
| `m-sever-durable-source` | W1 (NoRecord face) + eviction wall + consumed-token walls | **RUN: red, exit 101** — all four designed targets failed (W1, eviction, both consumed walls), 2 unrelated filter-matches passed (`runs/red-m-sever-durable-source.txt`) |
| `m-token-requires-untaken` | consumed-token walls (store + scheduler) | **RUN: red, exit 101** — exactly the two consumed walls, nothing else (`runs/red-m-token-requires-untaken.txt`) |
| `m-fire-skips-sends` (= R2 fire-path revert) | W3, publication-order, W5, live-process, Wall 1 | **RUN: red, exit 101** — all five designed targets failed AND W2 passed in the same run (`runs/red-m-fire-skips-sends.txt`). **Prediction (a), stated before running, CONFIRMED:** W2 stays GREEN under this revert — the record answers exactly-one at the post-send park with no fire at all; held across 10×10 isolated reps (`runs/obs-m-fire-skips-sends-w2-green-10x.txt`). This diverges from the brief's "W2/W3 red" forecast and is reported as an observation per OBS-007 — the runs govern. |
| `m-w3-first-watch-only` | W3 (starvation face) | **RUN: red, exit 101** — exactly W3 (`runs/red-m-w3-first-watch-only.txt`) |
| `m-w4-drop-no-deregister` | W4 (abandonment face) | **RUN: red, exit 101** — exactly W4 (`runs/red-m-w4-drop-no-deregister.txt`) |
| `m-w5-fire-replaces-publish` | W5 (subscriber-missed face) | **RUN: red, exit 101** — exactly W5 (`runs/red-m-w5-fire-replaces-publish.txt`) |
| `m-wall1-publish-before-install` | Wall 1 (value face) — the brief-specified mutation, MANDATORY of record | run pending — first attempt held at preflight (live battery claim, Phoebus Anzac 17:06Z; convention honored, nothing launched) |
| `m-check-then-register` (= R3 order revert) | brief predicts W2 red at lost-wake | authored, run pending. **Prediction, stated before running:** NO wall reds — the loss window check-then-register opens is PRE-INSTALL, and the only deterministic park the existing test hook offers is post-send (post-install), where the check simply hits the installed record. If confirmed: W2's citable lost-wake red is the commit-A skeleton (both halves reverted; face recorded in `26a49c1`'s body and reproducible via `m-skeleton-both-reverted`), and the pre-install window's unreachability is a documented limit of the existing gate — candidate for a pre-install rendezvous in a future lane, the tear to judge. |
| `m-skeleton-both-reverted` | W2 (lost-wake, observed 0) — the demonstrated killer | authored, run pending; commit `26a49c1` is the natural exhibit of the same red |

## Battery (pending, last step) — CLAIM CONVENTION v2

beamr gates.json five legs VERBATIM from the brief's `.verification`
(identical to the hygiene battery's legs), run at the final head via the
fresh v2-conformant runner **`battery/run-battery-v2.zsh`** (runner-v2
remediation dispatch, Artemis 17:00Z sha `17ba6688…`, re-brief 17:03Z sha
`57f990a5…`; the landed hygiene runner is pre-v2 evidence and is never
edited in place). Runner shape: claim FIRST at
`/tmp/ablative-gate-battery.claim` (atomic create; seat/member/pid/
started/phase/tree; phase draining→running), THEN drain-wait under the
held claim — 30 s samples, 60-minute ceiling, every sample recorded in
`battery/drain-record.txt`, refusal only on timeout (loud, run-stopping);
rule-5 stale handling at the floor exactly; trap-release on every exit
path. The quiet CENSUS (zero foreign cargo/rustc), not the claim, is the
evidence's quiet-floor proof (anchor rule 6). Built against the frozen
six-entry pin set (anchor `4b8b38e1` + Amendments 1 `e903b4ad` /
2 `c6d998bc` + addendum `aa92a18c` + withdrawal `91ba17f9` +
ratification `c3ee8385`), restated verbatim in the runner header and in
the emitted `battery-header.txt`. Behavioral conformance proof: claim-path
grep hit line 46 (functional assignment); acquisition line 90 strictly
before drain-wait line 114; trap/release lines 60–67 covering every exit
path. Evidence lands beside this file with the hygiene-battery shape:
per-leg logs, exits, load lines, toolchain stamp, tallies, completion
marker — machine and operator named.

## The no-edit list (R3 acceptance, stated for the handoff)

`take_exit_outcome`, `peek_exit_reason`, `run_until_exit`,
`take_exit_error`, `take_exit_exception`: behaviourally UNCHANGED — their
existing tests pass with **zero edits** in this branch's diff. aion and
liminal-server require NO change for this brief to land (R4/boundary);
Vesper Lynd is the aion-side authority of record for the
publication-path change, whose two review points are honored as tests
(REVIEW POINT 1: `watch_on_live_process_fires_on_real_exit_and_outcome_is_takeable_at_wake`
and the publication-order wall; REVIEW POINT 2:
`watch_after_outcome_consumed_reports_reason_from_token` (a) and, for (b),
no watcher can observe a death whose outcome is not yet claimable —
pinned by the same ordering walls).
