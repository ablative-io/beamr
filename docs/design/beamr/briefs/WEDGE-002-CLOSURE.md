# WEDGE-002 — closed as an observation lane, and the wedge law amended

Ruled 2026-07-29 by Artemis Peach (beamr owner seat). Executor: Diana
Plum, Annabel's box. Evidence: branch `diana/beamr-wedge002-3aecb622`
at `c27ad4d` (base `f29c01d`, evidence-only, no source change).

## Disposition

**No code change.** The brief's premise falsified under its own
red-first instruction, and the executor reported before restructuring,
as the brief required. Restructuring a failure path that is *proven* to
terminate would move asserts for no mechanism gain, against a standing
"coverage must not narrow" constraint.

## What was actually observed

Under `m-wall1-publish-before-install`, the churn test
(`receiver_contests_publication_without_misses_under_coordinated_multi_worker_churn`)
terminates **clean red: exit 101 in 5 s**. The 180 s bound never
engaged. The brief anticipated the fork — "hangs … or panics-in-park;
record which" — and the record is: panics-in-park, terminating.

## Why it terminates — verified at the owner's bytes, not taken from the return

Each round's publication-phase handle is a **local inside the observer
closure** (`exit_observation_tests.rs:108` `move`, bound at `:111`).
The at-park panic at `:119` unwinds and drops it; the drop disconnects
the gate; parked publishers are released; main's send at `:142` then
fails with `SendError` and panics. That two-panic chain is exactly what
the run log shows.

The release is not incidental. `exit_events.rs:382-386`:

```rust
if gate.published.send(()).is_ok() {
    // Disconnection means the observer failed and is unwinding; do not
    // turn its finite receive timeout into a stuck publisher thread.
    let _ = gate.observed.recv();
}
```

Release-on-drop is **designed and documented at the gate**. The
contrast case is verified too: `8f3bf57`'s own commit body records that
the store wall wedged because its release channel "lived outside" the
`thread::scope`, so the panicking thread could not drop it.

## The amendment — and it corrects the law's author, not the executor

The law as ratified (17:51Z, `c82043f0…`) reads: *no assert that can
panic may run while a parked thread's release depends on a line after
it.* That is a **surface** test. It flags this test and the store wall
identically, and would have ordered a restructure of a conformant test.
**That is a false positive in the law itself.** It was caught only
because the brief mandated report-before-restructure; had the brief
said "apply shape (a)," the churn would have been paid and the law's
over-breadth would still be latent.

**Amended, ruled:** the surface scan is the **screen**; the mechanism
scan is the **verdict**. A surface hit is a *question*, not a finding.
A wedge requires one of:

- **(i)** the release is disconnect-insensitive (a barrier or condvar
  that cannot observe the owner going away), or
- **(ii)** the unwind does not reach the release's owner (the handle
  lives outside the scope that panics).

Store wall = **(ii)**. This churn test = **neither** — conformant.

Two corrections of record are the executor's own, both in her evidence
README: the brief's hang premise, and her EXIT-001 audit flag that a
failed round "would wedge identically." A third is mine: the law's
phrasing above.

## Battery

**None, deliberately.** The evidence branch changes no source. A canon
battery on it would be a green certifying nothing — the same false-green
shape this project has been closing all night. Athena's conditional
launch word goes unspent. Whether this evidence merges to `main` is a
separate landing decision at fix-wave close; if it merges, it carries a
battery then. The battery rule is deferred, not excepted.

## Why these two payloads are in the repository

`WEDGE-002.md` and `WEDGE-002-DRIFT-r5.md` are the dispatch bodies,
**byte-identical to what was sent**. They are here because the message
transport truncated four dispatches and swapped two bodies between
envelopes on 2026-07-29; the executor's hash echo failed on both, and
that refusal was correct — hash-inline had confirmed matches all night
and this was the first time it caught anything.

Re-sending the same way risked reproducing the defect, so the payloads
moved to a carrier with a self-verifying identity, the shape the estate
adopted for canon after the r4b hash confusion: **bytes land in a repo
with a blob hash; the message carries path + hash + rationale rather
than the bytes.** A git blob cannot arrive truncated or swapped — it
either resolves to the stated hash or it does not.

Dispatch hashes, trailing newline stripped per the standing framing
rule, verified in this tree:

| file | sha256 | bytes |
|---|---|---|
| `WEDGE-002.md` | `c2b5288515bb234c2dd36ed94f557e2aa4307df9d1dce6c0d2f9fa57804d4f8f` | 3873 |
| `WEDGE-002-DRIFT-r5.md` | `85371e5fcd43e4872d44b2d66c6e6322152b21cd36a3a93c9791c171c6e9ad2c` | 2688 |

Reproduce either without trusting this table:

```sh
printf '%s' "$(git cat-file blob <blob>)" | shasum -a 256
```

**Note the drift note is now superseded in part by this closure**: it
directs derivation from the canon r5 blob for a battery, and this
ruling orders no battery. It is retained unaltered because it is the
dispatch record, and because evidence is never rewritten forward.
