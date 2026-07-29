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
| `m-wall1-publish-before-install` | ordering face (outcomes-before-exits) — the brief-specified mutation, MANDATORY of record. **Face assignment ruled** (Artemis 17:19Z, BUILD BOTH, sha `e177394a…`): Wall 1 owns the VALUE-AT-WAKE face; the ordering face is owned by the publication-order wall and Wall-1b. | **THREE RUNS.** (1) Narrow filter (wall1_ only): GREEN — a FILTER ARTIFACT, not a survival; re-titled `runs/green-m-wall1-narrow-filter-observation.txt`. Wall 1's green under this mutation is DETERMINISTIC, not a race: `terminate_process` finalizes synchronously on the calling thread, so the watcher cannot run until the install has completed regardless of internal ordering — Wall 1 cannot fail at the ordering face, and saying so is the finding. (2) Wide filter: five walls GREEN (wall1/W2/W3/W5/live-process — this mutation reorders fires, it does not skip them; the red-W3/W5/live part of the stated prediction was WRONG, runs govern) and `publication_order` **WEDGED** — its at-park assert panicked inside `thread::scope` while the gate-parked publisher waited on a release channel held outside the scope: a deadlock, killed by owner after ~20 min, recorded RED-and-loud (discovery record: `runs/wedge-m-wall1-publish-before-install-discovery.txt`; the hang also held Phoebus's restitution drain busy ~19 min — owned, disclosed). **RULED a defect in the WALL, not a mutation artifact** (Artemis 17:51Z, sha `c82043f0…`): the deadlock was the wall's own failure path — wedge is not red. Law pinned: no assert that can panic may run while a parked thread's release depends on a later line; release unconditional → join → assert on captured values. (3) The wall is FIXED shape (a) (capture-at-park, assert-after; every at-park capture verified non-blocking at the bytes) and its clean red under this same mutation is captured post-fix — see the face table. Wall-1b provides the scheduler-seam red — see below. |
| `m-wall1-publish-before-install` × **Wall-1b** | ordering face, clean red (strict reading of Hermes's rider) | Wall-1b (`wall1b_ordering_tripwire_outcome_installed_before_exit_publication`) collects at-park observations with NOTHING panicking while the gate is armed, releases, joins, then asserts — a red cannot wedge (shape (a) of the wedge ruling, by construction). Red-first run under this mutation pending (compile-gated). |

### Wedge-law audit (scope-fenced: EXIT-001 walls that arm a gate and assert while a thread is parked)

Ruled 17:51Z (sha `c82043f0…`). Class members and dispositions:
`w2_watch_registered_during_inflight_publication_observes_exactly_one` —
one in-park panic site (record-answer assert): **FIXED shape (a)** (the
at-park registration is captured; all judging moves after release + join).
`publication_order_outcome_tombstone_event_then_watch_fire` — three
in-park panic sites: **FIXED shape (a)**; clean red under the mutation
proven post-fix. `wall1b_ordering_tripwire…` — conformant by
construction. Shape (a) chosen everywhere because every at-park capture
is non-blocking at the bytes (`finalized_reason`/`get`/`take_outcome`
read DashMaps; watch receives use zero timeouts; the parked publisher
holds only `order`) — no helper thread or timeout needed in any failure
path. **Same class, OUTSIDE the fence, reported not fixed:** the
pre-existing characterization test
`receiver_contests_publication_without_misses_under_coordinated_multi_worker_churn`
asserts in its observer thread (outcome-missing panic, duplicate-pid
assert) while that round's publishers are parked, release following the
asserts — a failed round would wedge identically. It predates EXIT-001
and is not one of this lane's walls; flagged here and in the return for
its own lane's ruling. |
| `m-check-then-register` (= R3 order revert) | brief predicts W2 red at lost-wake | **RUN: prediction (b) CONFIRMED — NO wall reds.** Full watch suite 14/14 green under the revert (`runs/obs-m-check-then-register-full.txt`); W2 green 10/10 isolated (`runs/obs-m-check-then-register-w2-10x.txt`). The loss window check-then-register opens is PRE-INSTALL, and the only deterministic park the existing gate offers is post-send (post-install), where the check hits the installed record. This diverges from the brief's "W2 red at lost-wake" forecast and is recorded OBS-007 style — the runs govern. W2's citable lost-wake red is the commit-A skeleton (face in `26a49c1`'s body) and `m-skeleton-both-reverted` below; the pre-install window's unreachability is a **documented limit** of the existing gate — candidate for a pre-install rendezvous in a future lane, the tear to judge. |
| `m-skeleton-both-reverted` | W2 (lost-wake, observed 0) — the demonstrated killer | **RUN: red, exit 101** at the verbatim face: "must observe exactly one notification (0 = lost wake…); observed 0" (`runs/red-m-skeleton-both-reverted-w2.txt`); commit `26a49c1` is the natural exhibit of the same red |

## Face-assignment table (wedge ruling demand 3: every face names its deterministic killer)

| face | wall | deterministic killer | red evidence |
|---|---|---|---|
| unknown-pid honesty (Live-on-miss) | `watch_exit_on_unknown_pid…` | `m-live-on-table-miss` | `runs/red-m-live-on-table-miss-unknownpid.txt` (exit 101) |
| NoRecord truthfulness / durable source | W1 + eviction + consumed walls | `m-sever-durable-source` | `runs/red-m-sever-durable-source.txt` (4 walls red, exit 101) |
| exactly-once token honesty | consumed-token walls (store + scheduler) | `m-token-requires-untaken` | `runs/red-m-token-requires-untaken.txt` (exit 101) |
| notification delivery (fire path) | W3, W5, publication-order, live-process, Wall 1 | `m-fire-skips-sends` | `runs/red-m-fire-skips-sends.txt` (5 walls red, exit 101; W2 green = record answers, prediction (a)) |
| W3 starvation | W3 | `m-w3-first-watch-only` | `runs/red-m-w3-first-watch-only.txt` (exit 101) |
| W4 abandonment leak | W4 | `m-w4-drop-no-deregister` | `runs/red-m-w4-drop-no-deregister.txt` (exit 101) |
| W5 subscriber displacement | W5 | `m-w5-fire-replaces-publish` | `runs/red-m-w5-fire-replaces-publish.txt` (exit 101) |
| value-at-wake (Hermes rider 1+2) | Wall 1 | `m-fire-skips-sends` (fire loss) — Wall 1 CANNOT fail at the ordering face: synchronous finalization leaves no window (narrow-filter green is the exhibit, a filter artifact not a survival) | `runs/red-m-fire-skips-sends.txt`; `runs/green-m-wall1-narrow-filter-observation.txt` |
| **ordering (outcomes-before-exits)** | publication-order wall + **Wall-1b** | `m-wall1-publish-before-install` — **CLEAN red after the wedge-law fix; killed-by-wedge before it** (the wall's own failure path deadlocked; ruled a wall defect, fixed shape (a)) | post-fix clean red: `runs/red-m-wall1-fixed-walls-clean-red.txt` (both walls, exit 101, bounded wall-clock); pre-fix wedge: `runs/wedge-m-wall1-publish-before-install-discovery.txt` |
| W2 lost-wake (observed 0) | W2 | `m-skeleton-both-reverted` (commit-A skeleton = natural exhibit) | `runs/red-m-skeleton-both-reverted-w2.txt` (exit 101) |
| R3 order (register-then-check) | — | `m-check-then-register` kills NOTHING: documented limit (pre-install window unreachable by the post-send gate); the skeleton red is the citable lost-wake exhibit | `runs/obs-m-check-then-register-full.txt`, `…-w2-10x.txt` (all green) |

Post-fix 10× isolated greens for the wedge-law walls + Wall-1b:
`runs/green-postfix-walls-10x.txt` (30/30).

## Battery (pending, last step) — r3-derived runner

Governing runner: **`battery/run-battery-r3.sh`**, launched with BASH
from the worktree root. Preamble (phases 0–2) copied BYTE-VERBATIM from
**CANON r3**, stack-root entry `e269d2c9-0dfa-409e-ada5-b10303118225`,
both hash framings verified at this seat before copying (with-LF
`ff831516…`/11532 B; stripped `99e140b6…`/11531 B; extracted
programmatically, never retyped). r3 folds Seth's five flags at source:
`claim_is_mine` ownership guard, `pid_alive` via ps -p, trap covers HUP,
launch contract in-canon, bash self-guard. Known label discrepancy copied
not fixed: r3's line 2 self-describes "revision 2" (Seth, entry
`c59ba3b1`) — label-only. Disclosed deltas: the in-script instantiation
block (pattern ratified, entry `edb63b89`) and the census extension
(+`wasm-bindgen-test-runner`). Phase 3 = beamr's five legs VERBATIM from
the brief's `.verification` (no nextest — canon A4/A6 N/A, stated; A7
adapted to the TRUE line: doc tests COVERED via leg 5). Phase 4 =
canonical marker with all five leg exits + machine/operator + load
riders. The quiet CENSUS, not the claim, is the quiet-floor proof
(anchor rule 6). Behavioral conformance proof against this file:
claim-path grep hit **line 36** (functional assignment); acquisition
`acquire()` line 77 / loop line 87, strictly before drain-wait line 109;
trap line 69 (EXIT INT TERM **HUP**) with ownership-guarded
`release_claim()` lines 61–68 (`claim_is_mine` line 53, `pid_alive`
line 57). Ordering rider (Amendment 4): this battery queues behind
Phoebus's slot-restitution re-run on r3; the runner's own acquisition
loop handles the wait.

**Run 1 — RED, preserved at `battery/run1-red/`** (launched 18:01:27Z at
head `0974d78`, bash from the worktree root): leg1-fmt exit 1 (real
formatting debt across this lane's impl/test commits — the battery is the
lane's first fmt pass) and leg2-clippy exit 101 (`type_complexity` on the
watch registry's `DashMap<u64, Vec<(u64, Sender<(u64, ExitReason)>)>>`
field — fixed by factoring the `WatchSlot` type alias, ZERO `#[allow]`);
legs 3/4/5 GREEN including the full `cargo test --workspace`. Claim story
of run 1: drain sample quiet, `census-at-start.txt` EMPTY (quiet floor),
claim released as OWN claim — the own-claim-only release, ruled a
DETECTOR (dialect incident ruling, sha `bef7f4f2…`), PASSED: the claim
was not replaced mid-run, the run is not void; Minerva's battery took the
claim 16 seconds after release (clean cooperative handoff). Run 2 runs at
the fix head with the operator-side claim double-record (body captured
verbatim at acquire and immediately before release, match/no-match in the
return).

**Evidence limit, stated as ruled (Apollo's reading of canon, carried by
the dialect ruling): THE CENSUS BRACKETS THE RUN — start and end — AND
NOTHING SAMPLES THE FLOOR DURING IT.** A concurrent battery beginning and
ending inside the window is invisible to both parties and leaves no trace
in this evidence. The censuses here prove QUIET AT THE BOUNDARIES, which
is a weaker claim than QUIET THROUGHOUT, and this evidence claims only
the former. (Canon gap, raised to the anchor owner — not fixed here.)

**Dialect ruling (D4, sha `bef7f4f2…`) and Amendment A5 (entry
`a621f353`, sha `d66e72bf…` — supersedes the D4 do-not-patch line):**
this runner always WROTE the authoritative `key=value` dialect (canon
byte-verbatim). Per A5, three interim guards are now IN the runner as
declared deltas (r3's bytes stay frozen as identity — the
(entry-id, sha256) pair): (i) dialect-tolerant claim reads (`key=value`
and `key: value` both parse, acquisition + flip); (ii) an
unparseable/empty pid is never grounds for a rule-5 clear — reads HELD,
never stale; (iii) no phase flip without an ownership check — claim
re-read at draining→running, own member id + pid confirmed, foreign body
⇒ flip REFUSED loudly (exit 6), run VOID as evidence (detector, not
error). Plus the claim double-record protocol (body verbatim at acquire
and immediately before release; match/no-match reported in the return).
**A5 third delta (ruling `4081cb3d`, sha `0ace70c7…`): at release, an
ABSENT claim is voiding exactly like a foreign one** — the
thief-finishes-first ordering leaves nothing on file, and a quiet no-op
there would publish a green whose quiet-floor premise was violated. The
release path now has all three branches (own / foreign / absent, the
absent branch armed only for a run that actually acquired). Detector
limits, stated as ruled for THIS box's topology: (a) the flip guard
detects AFTER canon's internal `mv` rather than refusing before it —
true pre-flip refusal awaits r4; (b) this box launches runners
independently — no coordinator serialization backs the premise (Dean's
serial-venue ruling does not transfer); (c) the quiet-floor claim rests
on the detectors firing (perpetrator at flip, victim at release, both
victim branches built), not on collision prevention. Pin set is now
FIFTEEN (adds `a621f353`, `77b2c212` null-diff rider, `4081cb3d`).
Amended-file conformance lines: claim-path grep hit **line 36**;
`acquire()` line 91 / loop line 101, strictly before drain-wait
line 132; trap line 83 (EXIT INT TERM HUP) with the three-branch
`release_claim()` lines 61–82; A5 flip guard follows the drain-wait.
Run-2 first attempt: aborted by owner during its acquisition wait (no
claim held, no legs run — parked behind Mercury Toast then Phoebus)
when this delta landed; noted in `battery/claim.log`.

**Run 2 — GREEN, the battery of record** (head `5c0795b`, launched bash
from the worktree root): ALL FIVE LEGS EXIT 0 — fmt 0, clippy 0,
wasm32-check 0, wasm-tests 0 (80 passed / 0 failed), tests 0
(**2041 passed / 0 failed across 70 binaries**, `cargo test --workspace`
— doc tests covered via this leg, stated in
`battery/doc-tests-coverage.txt`). Claim story: acquired 18:17:15Z,
census-at-start EMPTY (the quiet-floor proof of record, boundaries only
per the stated limit), A5 flip ownership check PASSED, own-claim release
18:18:49Z; claim double-record MATCH (bodies identical except the ruled
phase flip draining→running). Dirty entries at start = 6, all the
battery's own in-flight evidence files, zero source modifications.
Wrapped-ness is verifiable from this bundle, never assumed: the
three-delta disclosure header citing `a621f353`, the
claim-body-at-acquire/-at-release pair, the flip-check line and the
release-determination line in `battery/claim.log` are records an
unwrapped canon launch cannot produce.

**Superseded-runner quarantine (claim-runner inventory order, Artemis
18:17Z sha `a9b5ae82…`):** a label is not a control — the superseded
runners were launchable at their committed paths, so both are MOVED off
the launch path to `battery/superseded-quarantine/` with
`.quarantined.txt` suffixes (history preserves the originals at their
commits): `run-battery-v2.zsh` (guarded release but unguarded flip,
kill -0 liveness) and `run-battery-v3-canon.sh` (r2-canon guts:
UNGUARDED trap release — Seth's theft-on-timeout class — plus unguarded
flip and kill -0). Four canon extract copies in the seat scratchpad were
quarantined the same way. The box-wide inventory record is the
deliverable posted on the lane.

**Run 1 QUALIFIED per A5's audit note (not retroactively void):** run 1
completed before `a621f353` and its claim was never re-read at flip
time — it attests a quiet floor at the boundaries that was NOT verified
at the flip. Stated here as the run's own limit. (Its own-claim release
did pass, so the claim was intact at exit.) The lane's mutation runs
never took the claim (ordinary compiles, preflight-only) — the audit
point does not apply to them.

**Supersession chain (all predecessors stay in history, none launch):**
`battery/run-battery-v2.zsh` @ `42c6231` — built honestly against the
six-entry pin; superseded by re-brief 2 before any battery ran; zsh
vessel additionally retired by Seth's flag 5. →
`battery/run-battery-v3-canon.sh` @ `421778f` — derived from `622dedbf`;
superseded by Amendment 3 (canon r3 replaces both its predecessors as
build source) before any battery ran. → `battery/run-battery-r3.sh`
(current).

Battery evidence lands beside this file: per-leg logs + stderr logs,
exits, drain samples, censuses at start/end, load lines, toolchain stamp,
tallies, completion marker — machine and operator named.

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
