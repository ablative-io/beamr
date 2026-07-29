BRIEF — WEDGE-002: apply the deadlocking-wall law to the churn test flagged out-of-fence during EXIT-001. Ready to dispatch when a slot opens. Author: Artemis Peach (…c9fa), coordinator seat, Tom's box. Status: DRAFT-READY, not yet dispatched, no executor named.

═══ WHAT AND WHY ═══

TARGET: the pre-existing test `receiver_contests_publication_without_misses_under_coordinated_multi_worker_churn` (crates/beamr/src/scheduler — locate by name at the executor's own bytes; do not trust this brief for a line number, the tree has moved since the flag).

THE DEFECT CLASS, already ruled and already fixed elsewhere: the test carries assert sites that can panic while a parked thread's release depends on a line AFTER the assert. Under the deadlocking-wall law (ruled 17:51Z, sha c82043f0…, ratified): a wall whose FAILURE PATH DEADLOCKS cannot land — a future regression would hang CI instead of redding. Wedge is not red.

PROVENANCE OF THE FLAG: Diana Plum's EXIT-001 evidence (branch landed at main f29c01d, merge parent 2da58c4). She found the class while fixing the same defect in the EXIT-001 walls at commit 8f3bf57, honored the audit fence (this test is not an EXIT-001 wall), and reported it in the evidence README rather than touching it. This brief is that report converted to a lane item.

═══ THE FIX SHAPE — PROVEN, NOT NOVEL ═══

Apply shape (a) from 8f3bf57, which fixed four sites of the same class and is the ruled pattern:
1. CAPTURE AT PARK: every value the at-park asserts need is read into locals while parked, using only non-blocking reads.
2. RELEASE UNCONDITIONALLY: the parked thread's release happens before any assert that can panic — no assert between park and release.
3. JOIN, THEN ASSERT: assertions run on the captured values after the join.

The executor MUST verify at their own bytes that every at-park capture in this test is genuinely non-blocking (8f3bf57's justification: DashMap reads, zero-timeout receives, order-only holds). If any capture in THIS test blocks, that is a finding to report before restructuring, not a detail to work around.

═══ WALL LAW APPLIES TO THE FIX ITSELF ═══

This is a change to a test's failure path, so it needs proof the failure path now terminates:
- RED FIRST: under a deliberate mutation that makes the test's assertion fail, demonstrate the CURRENT code hangs (bounded observation, timeout + kill, never an unbounded wait) or panics-in-park — record which.
- Then apply the restructure and demonstrate the SAME mutation now produces a clean red: exit 101, bounded wall-clock, faces recorded verbatim.
- The mutation is never committed applied; diff in evidence per standing practice.

═══ CONSTRAINTS ═══

- Code bar: zero new #[allow]; no unwrap/expect/panic outside cfg(test) (this IS cfg(test) — panics in asserts are fine, panics that can fire while something is parked are the defect).
- Behavioral: the test's COVERAGE must not narrow. It exists to contest publication under coordinated multi-worker churn without misses; the restructure moves WHERE asserts run, never WHAT they assert. State this in the evidence.
- Branch off current main (f29c01d at authoring — executor re-verifies), push the branch, NEVER main. Return fields per harness R2. Battery per current canon — derive from the r4b blob (haematite gate-logs/canon-gate-battery-r4b.sh, blob 1b63111b…, sha256 8b7bedfc…/20471), identity is (blob, sha256), never a label. Runner identity: MEMBER_ID derived-or-dispatcher-checked per the identity refusal adopted tonight — the executor echoes their id in the return and I check it.
- Venue: any box with a slot. If Annabel's box: sequencing word from ATHENA required before launch.

SIZE: small — one test restructured, one mutation, one battery. The battery is the cost; the edit is an afternoon's care.
