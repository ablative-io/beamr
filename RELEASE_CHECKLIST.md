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

- [ ] The release-obligations gate passes: `scripts/release-obligations-gate.py` (also runs automatically inside `scripts/release.sh`, dry-run included). A red here means `RELEASE-OBLIGATIONS.json` holds an obligation due at this cut — resolve it in that file (state the resolution) or record a lead's ruling as its new reason. ⛔ Publishing by bare `cargo publish` without this gate bypasses the ruled path (`ablative/docs` `tracking/ruling-release-obligations-20260809.md`) — use `scripts/release.sh`.
- [ ] Audit non-test production source for `eprintln!`, `dbg!`, `todo!(`, `unimplemented!(`, and unacceptable `panic!(`.
- [ ] Confirm no known open memory-safety findings ship unfixed without a recorded project-lead ruling (`docs/REVIEW-23-07.md` is the standing open register).
- [ ] Create the local tag only after all validation gates pass. ⛔ The tag namespace is **per-crate**: `beamr-vX.Y.Z`, `gleam-types-vX.Y.Z`, `beamr-cli-vX.Y.Z`, `beamr-wasm-vX.Y.Z`. The bare `vX.Y.Z` form is pre-0.19.0 and is dead — this line said `vX.Y.Z` until 0.19.4 and nearly produced a tag in the abandoned series. Tag the **last commit of the release sitting** (usually the evidence commit), not the version-bump commit: `beamr-v0.19.3` → `145212e`, two commits after the bump.
- [ ] Write the tag **after** `scripts/release.sh --publish`, not before. This line is a lower bound on *when*, not an ordering against publish: every annotation from 0.19.2 on carries the published crate's `cksum`, and the commit is taken from the published crate's own `.cargo_vcs_info.json` rather than inferred. Verify the publish at the source first — crates.io API `checksum`/`yanked`, `shasum -a 256` of the `.crate` served by static.crates.io, and that artifact's `.cargo_vcs_info.json` sha1 against `git ls-remote` on main. `release.sh` reporting success is a claim, not a receipt.
- [ ] Push the tag in the same sitting the release commit lands — the v0.15.3–v0.16.2 tag gap (BEAMR-MISSING-016X-TAGS) came from deferring this.
- [ ] Do not push tags or run `cargo publish` without project-lead approval.
