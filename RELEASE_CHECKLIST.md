# Release 0.16.3 checklist

Do not publish crates, push tags, or create a GitHub release without explicit approval from the project lead.

## Version and metadata

- [x] Confirm `crates/beamr/Cargo.toml` has `version = "0.16.3"`.
- [x] Confirm `crates/gleam-types/Cargo.toml` is unchanged (`0.4.3`) — not part of this release.
- [x] Confirm internal path dependencies carry semver-compatible crates.io fallbacks (`beamr-cli`/`beamr-wasm` → `beamr = "0.16.0"`, `beamr` → `gleam-types = "0.4.3"`).
- [ ] Run `cargo publish --dry-run -p beamr` (project-lead machine; not performed at the build seat).

## Validation gates

- [x] Full 5-leg battery green at the release head (fmt / clippy `-D warnings` / wasm32-check / wasm-tests / workspace tests), run cold from the clean committed tree, evidence committed with `gates.py verify` VERIFIED and the toolchain stamped (rustc/clippy 1.97.1).

## Release constraints

- [ ] Audit non-test production source for `eprintln!`, `dbg!`, `todo!(`, `unimplemented!(`, and unacceptable `panic!(`.
- [x] **Memory-safety line — RULED EXCEPTION (project lead, 2026-07-28).** This release ships with known remaining JIT-helper staleness sites, disclosed plainly in `CHANGELOG.md` under "Known remaining JIT sites": `jit_bs_start_match`'s stale source-Term write (D1), the helper-argument staleness class (D3), and the accumulated-results class (F3) — all requiring ABI-level GC rooting owned by RF-006 on the 0.17.0 line, not backportable in a patch. Tom's decision, relayed on the record by coordination 2026-07-28, is the authority for this checked box; there is no silent memory-safety checkbox. All *backportable* known GC-safety defects on this line are fixed in this release (thirteen consumers, red-first, torn PASS per lane).
- [ ] Create local tag `v0.16.3` only after all validation gates pass.
- [ ] Do not push `v0.16.3` until project-lead approval is recorded.
- [ ] Do not run `cargo publish` without project-lead approval.
