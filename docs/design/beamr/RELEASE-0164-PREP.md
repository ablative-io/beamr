# 0.16.4 release prep — what it can contain, and what it cannot

Prepared 2026-07-29 by Artemis Peach (beamr owner seat) so that the
version ruling, which is the project lead's, is made against facts rather
than against a question. Nothing here authorises a release: the cut, the
tag and the publish are the lead's hands alone.

## The finding that constrains the ruling

**A 0.16.4 cannot be cut from `main`.** Not "should not" — cannot, without
either breaking semver or shipping a version number that misdescribes its
own contents. `main` carries two changes that a patch release may not:

- **`Scheduler::watch_exit` (`6a3ceec`)** — a new public API surface
  (`ExitWatch`, `ExitWatchState`). Additive public API is a MINOR bump.
- **`Scheduler::spawn_link_dirty` removal** — a breaking removal, already
  documented under "Removed (breaking — 0.17.0 window)".

This is independent confirmation of the standing ruling recorded in
`edeba6d`'s commit body and restated in the changelog: the version moves
once, to `0.17.0`. That ruling was made on release-hygiene grounds; this
audit shows it is also forced by the tree's actual contents.

## What a legitimate 0.16.4 would be

A patch on the 0.16.x line, cut from the **0.16.3 release point**, carrying
only bug fixes. The candidates on `main` that qualify, both user-facing and
both already red-first with walls:

- **`f931c16`** — `imports` honors `--dir`; `compile` refuses it rather
  than accepting a flag it never consumed.
- **`bd4e04f`** — `--dir` load failures are reported on stderr instead of
  being swallowed in silence.

Both are confined entirely to `crates/beamr-cli/` — they do not touch the
runtime, so backporting them cannot disturb the memory-safety surface that
is the 0.16.x line's whole reason for existing. That confinement is what
makes them safe patch material rather than merely eligible.

Everything else on `main` since the 0.16.2 release point is either
0.16.3's forward-ported memory-safety work (already released), evidence
commits, repo hygiene, or the two version-blocking changes above.

Verified at the bytes 2026-07-29, not asserted from the changelog:
`fn spawn_link_dirty` is present at `67f89c4` in
`crates/beamr/src/scheduler/spawning.rs` and absent from `main`'s sources
(the surviving reference is the pinned per-entry dispatch test); and
`pub fn watch_exit` is present on `main` at
`crates/beamr/src/scheduler/execution.rs:251`.

## The base check, mechanised

Any 0.16.x cut must contain the 0.16.2 fixes. The test, which belongs in
the release procedure rather than in anyone's memory:

```sh
git merge-base --is-ancestor 67f89c4 <base>
```

Must return true, or the cut carries the two 0.16.2 memory-safety classes
(the GC refcount-release walk's `word[0]` type inference, and ETS storing
borrowed caller-heap terms) and ships them under a higher version number.
Verified 2026-07-29: `67f89c4` IS an ancestor of `origin/main`, and no
0.16.4 artifact exists anywhere in this repository — no branch, no commit,
no changelog entry. The check is therefore a constraint on whoever creates
it, not an audit of something that exists.

## Post-publish verification — required, not optional

A green suite proves our code works. It does not prove that what landed on
the registry IS that code. Adopted estate-wide after a sibling project
shipped a CLI pinned to an empty crate, because an old name reservation
satisfied the version requirement quietly.

- Install fresh as a stranger: clean directory outside any workspace, add
  the published version from the registry (no path, no git dependency),
  build.
- Walk the documented examples against the installed crate, not the repo.
- Negative controls still fail closed.
- Compare the registry tarball against `cargo package` at the tagged
  commit — the same discipline as the ancestry test above: turn the
  assumption into a comparison.

## Open, and owned by the lead

Whether a 0.16.4 should exist at all, given that its only candidate
contents are two CLI fixes and that `0.17.0` is where the line is already
heading.
