# beamr Ledger — what shipped, and where the truth lives

As of 2026-07-28, beamr main a0359a2.

Verified crate versions (from `Cargo.toml` files at this tree): `beamr` **0.16.2**,
`beamr-cli` **0.4.0**, `beamr-wasm` **0.7.0**, `gleam-types` **0.4.3**. The top released
heading in `CHANGELOG.md` is `## 0.16.2 — 2026-07-23`, above it an open `## Unreleased`
(0.17.0 breaking window).

Verified test figures: a **static** count of test attributes in `crates/` gives **2036**
`#[test]`/`#[tokio::test]` functions (beamr 2008, beamr-cli 23, beamr-wasm 3,
gleam-types 2) plus **80** `#[wasm_bindgen_test]`. This is a source count, not a runner
count — attribute counting cannot see cfg-gating, feature-gating, or per-case expansion.
The most recent runner total stated in an evidence commit is **2007 passed / 69 suites**
plus the **80** wasm-bindgen tests, at `ba5c5fa` (battery over tree `ef4fb90`). The
0.16.2 release battery recorded 1998 passed / 69 suites at `5206e7a`.

## Provenance

This doc replaces three files removed in the same commit; their full text stays in git
history. Their machine-readable twins (`checklist.json`, `stories.json`, `design.json`)
are retained — brief JSONs and dispatch tooling still reference them. The same commit
also trues the root `RELEASE_CHECKLIST.md`, which was hardcoded to a "Release 0.4.0"
that shipped months ago.

- **`docs/design/beamr/BRIEF-TRACKER.md`** — a phase/brief status table. It described the
  repo roughly four months in the past: header line `Current version: **0.3.15** | Tests:
  **726** | Published: 2026-06-07`, a team roster of retired agents (Bono, Dave Evans,
  Adam Clayton, Larry Mullen Jr), Phases 2, 3 and 4 marked `NOT STARTED`, and rows
  stopping at B-155 (159 distinct B-numbers) while 181 brief files exist on disk.
- **`docs/design/beamr/CHECKLIST.md`** — 190 checklist items, **190 unticked, 0 ticked**,
  including items (`C1` workspace members, `C10` `cargo check` clean, `C11` clippy clean)
  that the current release gate proves green on every battery run.
- **`docs/design/beamr/RUNBOOK.md`** — a dispatch runbook that issued brief work to the
  retired team by shelling `.meridian/workflows/onatopp-dev-norn/benchmark.sh` per brief,
  notifying "Bono". The script still exists; the dispatch model it serves does not.

## What shipped

Phase names and brief ranges are the old tracker's. Status is what the tree shows now.

| Phase | Old tracker status | Actual | Evidence |
|---|---|---|---|
| 0 — Foundation (B-001–B-045) | COMPLETE | shipped | `crates/beamr/src/{term,atom,loader,interpreter,scheduler,gc,mailbox,supervision}/` |
| 0.5 — Hardening (B-046–B-055) | 8 landed, 2 in progress | shipped | `crates/beamr/src/capability/{sandbox,audit}.rs`; `crates/beamr/src/replay/` |
| 1 — Critical gaps (B-056–B-071) | BRIEF WRITING | shipped | briefs B-056–B-071 on disk; `871c999` |
| 2 — Refc binaries / dirty sched (B-072–B-085) | NOT STARTED | shipped | `crates/beamr/src/term/{shared_binary,binary_ref,sub_binary}.rs`; `crates/beamr/src/scheduler/dirty.rs` |
| 2 — ETF (B-086–B-089) | NOT STARTED | shipped | `crates/beamr/src/etf/{encode,decode,tags}.rs`; `ab743fe` (B-087), `8e1edba` (B-088) |
| 2 — ETS (B-090–B-099) | NOT STARTED | shipped | `crates/beamr/src/ets/` (set, bag, ordered_set, match_spec, copy, owned_key); `3a138ef` (B-096), `2c13d11` (B-099) |
| 2 — Port/IO (B-100–B-111) | NOT STARTED | shipped | `crates/beamr/src/io/{uring,ring,thread_pool,resource,standard_io}.rs`; `crates/beamr/src/native/inet_bifs.rs` |
| 3 — Distribution (B-112–B-125) | NOT STARTED | shipped | `crates/beamr/src/distribution/` (node, resolver, connection, handshake, atom_cache, control, control_link, remote_link, global, pg); `7329d89` (B-119), `992e796` (B-125) |
| 3 — erlang BIF batches (B-126–B-129) | NOT STARTED | shipped | `crates/beamr/src/native/` BIF modules (`bifs.rs`, `dictionary_bifs.rs`, `etf_bifs.rs`, `ets_bifs.rs`, …) |
| 4a — JIT via Cranelift (B-130–B-138) | NOT STARTED | shipped | `crates/beamr/src/jit/` (~25 modules incl. `aot.rs`, `cache.rs`, `profiler.rs`, `ir_*.rs`); `cranelift-* = 0.131.2` behind the `jit` feature; `e5355fa` (B-138) |
| 4b — Deterministic replay (B-139–B-143) | NOT STARTED | shipped | `crates/beamr/src/replay/{recorder,driver,debugger,file}.rs`; `cfc70fe` (B-141), `08734cb` (B-140) |
| 4c — WASM target (B-144–B-148) | NOT STARTED | shipped | crate `beamr-wasm` 0.7.0; `52aece7` (B-145); WPORT-1–WPORT-9 arc (see `WASM-PORT-ARC.md`) |
| 4d — Capability security (B-149–B-151) | NOT STARTED | shipped | `crates/beamr/src/capability/{mod,sandbox,audit}.rs`; `46939ab` (B-149) |
| 4e — Observability (B-152–B-155) | NOT STARTED | shipped | `crates/beamr/src/telemetry/{spans,metrics,lifecycle}.rs`; `opentelemetry = 0.32.0` behind the `telemetry` feature; `8bb5979` (B-152) |

Beyond the old tracker's plan: the wasm port arc (WPORT-1–9), a wasm conformance floor
now on the release gate, and the 0.16.x memory-safety work (`67f89c4`).

## Brief inventory

`docs/design/beamr/briefs/` holds **181 distinct B-numbers**: B-001 through B-177, plus
the split briefs B-023a/b/c and B-040a/b. Alongside them are non-numbered brief families:
`WPORT-1`–`WPORT-9` (+ ground packs), `JIT-001/002`, `NATIVE-001/002/003`, `EMB-001/002`,
`ENC-001`, and `REAL-ERLC-ADMISSION-SCOPING.md` (landed at HEAD `ac74e4c`).

The retired tracker's highest row was **B-155**. **B-156 through B-177 — 22 briefs — have
files on disk, most with verified landing commits, and were never recorded in it.**
Verified anchors:

- B-156–B-164 entered main via merge `e6e3245` (2026-06-09).
- B-165–B-168: `a373fa8`; B-169–B-172: `6b076e6` (both 2026-06-09).
- B-173: `c3376c0`. B-174: `fc03639`. B-175/B-176/B-177: `af68ee6` (2026-06-10).
- Work commits in this range include `fa70fbc` (B-157 AOT), `5b41015` (B-159 AOT typed),
  `a2651cb` (B-160 binary IR), `b4ece25` (B-161 closure IR), `5458306` (B-163 messaging
  IR), `b2ea43f` (B-164 map opcodes), `68423a1` (B-167 runtime.rs split), `e76857c`
  (B-168), `8853165` (B-171), `1e46d78` (B-176).
- B-153, B-154, B-158, B-162, B-165, B-166 have brief files but no commit naming them in
  `git log --oneline --all`; their disposition is UNVERIFIED here.

## Where the truth lives now

- **Versions and what changed** — `CHANGELOG.md`. Top released heading `0.16.2 —
  2026-07-23`; `Unreleased` holds the 0.17.0 breaking window.
- **Architecture and design** — `docs/design/beamr/DESIGN.md`.
- **The wasm port arc** — `docs/design/beamr/WASM-PORT-ARC.md` (standing arc authority,
  WPORT rungs and the conformance classing ledger).
- **Open findings** — `docs/REVIEW-23-07.md`, the 2026-07-23 external review at `fd71c5e`:
  C1 and C2 are fixed and shipped in 0.16.2; H1–H6, the MEDIUM block, and the
  latent/dormant section are the standing open register.
- **Release gate** — `gates.json` is the authority: a 5-leg battery (fmt, clippy
  `-D warnings`, wasm32-check, wasm-tests under `wasm-bindgen-test-runner`, workspace
  tests), with evidence commits bound to tree hashes. `RELEASE_CHECKLIST.md` (root) is
  the release *procedure*; versions always come from `Cargo.toml`/`CHANGELOG.md`.
- **Probe-sitting harness** — `docs/design/beamr/probes/harness/` (committed at
  `a0359a2`): the WPORT-3/6/7 sittings reproduction path, runnable from any seat.
- **Tags — known gap.** `git tag --list` returns 51 tags stopping at **v0.15.2**. No tag
  exists for 0.15.3, 0.15.4, 0.16.0, 0.16.1 or 0.16.2, all of which have CHANGELOG
  sections and release commits on main (0.16.2 = `67f89c4`). Tracked as
  BEAMR-MISSING-016X-TAGS; the gap is wider than the 0.16.x name suggests.
