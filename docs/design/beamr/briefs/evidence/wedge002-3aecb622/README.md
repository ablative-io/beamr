# Evidence — WEDGE-002 (fix-wave 3aecb622): churn-test wedge-law examination

Dispatch: Artemis 19:40Z (hash echo REFUSED — transport mutated/truncated
the dispatch bodies; refusal ratified 20:47Z, blob-pointer re-send
pending; work proceeded on the independently-verifiable core). Target:
`receiver_contests_publication_without_misses_under_coordinated_multi_worker_churn`
(exit_observation_tests.rs:91 at `f29c01d`). Machine: Annabel's box;
operator: Diana Plum (b337ce2b-336a-4856-a9d8-54c90496c9fb).

## The red-first observation — PREMISE FALSIFIED, recorded as ruled

The brief: demonstrate the current code "hangs (bounded observation) or
panics-in-park; record which." **RECORDED: panics-in-park with CLEAN
TERMINATION — exit 101 in 5 s; the 180 s in-shell bound never engaged**
(`runs/redfirst-churn-under-mutation.txt`; `timeout`/`gtimeout` are
ABSENT on this box — verified, command -v exit 1 for both — so the bound
was in-shell with pid, timestamps, liveness polling, and
bound-engagement assertion; the run instead terminated on its own, which
is itself the positive observation).

## Why it terminates: conformant by RELEASE-ON-DROP, not by shape (a)

The wedge law forbids an assert that can panic while a parked thread's
release depends on a line AFTER that assert. In this test the release
does NOT depend on a later line: each round's publication-phase handle is
moved INTO the observer thread, so an observer panic (the at-park assert
sites :119/:121/:122, the recv_exit timeout panic, the acknowledge
expects) unwinds and DROPS the handle — disconnect releases every parked
publisher (`wait_at_publication_gate`: `observed.recv()` errors on drop).
The observed chain in the run: panic at :119 → phase drops → producers
release and join → main's `publication_phase_tx.send` fails → expect
panics in main → scope completes → exit 101. Every other failure path
traced at the bytes terminates the same way (main-thread panic paths
leave the observer to time out at EVENT_TIMEOUT, drop, release). The
structural difference from the store wall fixed at `8f3bf57` is exactly
the defect's precondition: there the observer lived OUTSIDE
`thread::scope`, so the panicking thread could not drop it.

## Correction of record (OBS-007: runs govern, divergence stated)

My EXIT-001 audit flag said a failed round "would wedge identically" —
**that characterization was WRONG**. The static shape-scan (asserts
between park and release) found the class's surface; the mechanism scan
(who holds the release when the assert fires) shows conformance. Both the
brief's premise and my flag are corrected by this observation.

## Disposition — reported before restructuring

Per the report-before-restructure discipline: restructuring a test whose
failure path is PROVEN to terminate red would move asserts for no
mechanism gain, with churn risk against "coverage must not narrow."
Recommendation to the dispatcher: close WEDGE-002 as an observation lane
(no code change), with this evidence as the record; alternatively, if the
ruled preference is shape-(a) uniformity over mechanism-sufficiency, the
restructure remains available and the red is already demonstrated either
way. Awaiting the dispatcher's word; nothing was restructured.

Mutation: `mutations/m-wall1-publish-before-install.diff` (copy of the
landed EXIT-001 exhibit), applied to the working tree only and reverted;
tree clean post-revert (0 tracked modifications).
