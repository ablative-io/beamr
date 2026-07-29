# Evidence of record — beamr hygiene set (fix-wave 3aecb622, leg 1)

Dispatch `00b809ac` (Artemis Peach → Diana Plum …c9fb, 2026-07-29 16:08Z;
body integrity: sha256 `e072201d…dce518`, 5354 B, echoed and MATCHED).
Executed at Diana's seat, worktree `.wt-hygiene-3aecb622`, branch
`diana/beamr-hygiene-3aecb622` from main `74c7d3c`.

## Commit chain

| commit | what |
|---|---|
| `16ff2dc` | Step 0 — RAIL 6 in-binary-parallelism audit (`rail6-serialization-audit.md`) |
| `7bd75c9` | Leg A WALL, RED at commit (red output + exit 101 in commit body) |
| `f931c16` | Leg A fix — imports carries `--dir`, compile refuses it; walls green 26/26 |
| `628a02e` | Leg B WALL, RED at commit — corrupt `.beam` swallowed in silence (real binary, stderr contract) |
| `bd4e04f` | Leg B fix — per-file stderr warning + aggregate count; stdout byte-identical; wall green |
| `b3510e7` | Leg C — junk removed (`.commit-msg.tmp`, `aion.db`), exact-name ignores |
| `b83e86e` | Leg D — CHANGELOG version-hold sentence; `crates/beamr/Cargo.toml` untouched at 0.16.2 |
| `8ea660e` | rustfmt truing of the leg A/B additions (battery run 1's only red) |

## Gate battery (beamr gates.json five legs, verbatim)

**Run 2 = the gate, at `8ea660e`, ALL FIVE LEGS GREEN.**
Logs: `battery/leg*.log`, exits: `battery/exits.txt`, header (UTC stamps,
tree, toolchain, per-leg load lines): `battery/battery-header.txt`.

| leg | command (verbatim) | exit |
|---|---|---|
| 1 fmt | `cargo fmt --all --check` | 0 |
| 2 clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 0 |
| 3 wasm32-check | `cargo check -p beamr-wasm --target wasm32-unknown-unknown --locked` | 0 |
| 4 wasm-tests | `wasm-bindgen-test-runner --version && CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test --manifest-path crates/beamr-wasm/Cargo.toml --target wasm32-unknown-unknown --locked` | 0 |
| 5 tests | `cargo test --workspace` | 0 |

**Denominators / tallies (beamr has no nextest; cargo test totals are the
record):** leg 5 = **2026 passed, 0 failed** across **70** test-binary
`test result` lines (includes the 4 new wall/parse tests of this brief);
leg 4 = **80 passed, 0 failed** wasm tests. Completion marker present at
header end; toolchain `rustc 1.97.1 / cargo 1.97.1 /
wasm-bindgen-test-runner 0.2.125`, host `aarch64-apple-darwin`.

**Load disclosure (every run):** host load ~3.9–4.8 (1-min) across both
battery runs and all leg boundaries — this seat's own builds plus normal
desktop background; no other battery ran on this box. No leg was red under
load in run 2, so no re-run-quiet arbitration was needed.

**Run 1 (preserved, `battery/run1-red-fmt/`):** at `b83e86e`, leg 1 RED
(exit 1, rustfmt line-width on this brief's own additions — deterministic,
not load), legs 2–5 green (same 2026/0 and 80/0 tallies). Void as a gate;
kept as the record of why `8ea660e` exists. Also for the record: the first
invocation of the harness died on a wrong relative `cd` before any leg
ran — zero cargo invocations, nothing partial (note in `run-battery.sh`).

**Tree state at the gate run:** `git status --porcelain` showed exactly
one entry — the untracked `battery/` evidence directory this run was
generating. Zero modified tracked files (dirty=false in the sense that
matters; the evidence lands in the docs-only commit after the run, per
the established evidence shape).

## Leads carried forward (load-evidence law: a lead is never nothing)

- **RAIL 6 lead (from `rail6-serialization-audit.md`):** the
  `os:putenv/unsetenv` round-trip test performs process-global env
  mutation (edition-2024 `unsafe`) in the same test binary where parallel
  threads read env via `temp_dir()`. Low severity, no observed flake,
  remedy sketched (env-lock mirroring `telemetry::test_lock`, or a
  spawned-subprocess harness). Not reached here — step 0 is
  deliverable-only.
- **Observed adjacent gap (not touched, outside scope):** `replay`
  silently drops `--dir` exactly as `imports` did; same silent-drop
  class, needs its own ruling on carry-vs-refuse.
