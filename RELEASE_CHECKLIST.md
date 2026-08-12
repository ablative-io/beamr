# Release checklist

Applies to every release. The version being released ("X.Y.Z" below) comes from
`CHANGELOG.md`'s top entry and the crate `Cargo.toml`s — this file never names one.

Do not publish crates, push tags, or create a GitHub release without explicit
approval. **Approval may come from the project lead or from the delegated
approvers (Cally Ray, Waffles) — it does not have to come from the project
lead personally, and a delegated approval must not be routed back to him for
confirmation.**

All family crate publishing is centralized at the project lead's machine.
**This is a requirement about WHERE the publish runs, not about WHO types it.**
Work executing on that machine — including an agent session running in the
project lead's checkout — satisfies it. It is not a second approval step and
must never be read as one.

⛔ **Once approval exists, do not hold the release.** Waiting on the project
lead when he has already delegated is the specific failure this paragraph
exists to prevent, not the caution it is asking for. If a release is ready and
approved, publish it.

## Version and metadata

- [ ] Confirm every released crate's `Cargo.toml` carries the X.Y.Z being released and `CHANGELOG.md`'s top released heading matches.
- [ ] Confirm internal path dependencies include matching crates.io version fallbacks (`beamr -> gleam-types`, `beamr-cli -> beamr`).
- [ ] Confirm published crates declare license, description, repository, and readme metadata.
- [ ] Confirm `cargo metadata --no-deps --format-version 1` contains no stale-version or `git+https://` publish blockers.

## Publish dry-runs

- [ ] Run `cargo publish --dry-run -p gleam-types`.
- [ ] After `gleam-types` dry-run succeeds, run `cargo publish --dry-run -p beamr`.

## Validation gates

- [ ] Run the full `gates.json` battery at the release commit — fmt, clippy `-D warnings`, wasm32-check, wasm-tests, workspace tests — and land its evidence commit bound to the tree hash. This is the release gate of record; the legs below are the manual fallback, not a substitute.
- [ ] Run `cargo bench --package beamr --no-run`.
- [ ] Run `cargo doc --package beamr --no-deps`.

## Release constraints

- [ ] Audit non-test production source for `eprintln!`, `dbg!`, `todo!(`, `unimplemented!(`, and unacceptable `panic!(`.
- [ ] Confirm no known open memory-safety findings ship unfixed without a recorded project-lead ruling (`docs/REVIEW-23-07.md` is the standing open register).
- [ ] Create local tag `vX.Y.Z` only after all validation gates pass.
- [ ] Push the tag in the same sitting the release commit lands — the v0.15.3–v0.16.2 tag gap (BEAMR-MISSING-016X-TAGS) came from deferring this.
- [ ] Do not push tags or run `cargo publish` without project-lead approval.
