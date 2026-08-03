# RF-003-D3 — SIX-LEG BATTERY at the final head

Head under test: `8d31abcf539c93604136d93962645bce04070c58`
(this evidence commit adds only `gate-logs/`, no source).
Window: `2026-08-03T01:30:15Z` → `2026-08-03T01:33:05Z`
(`battery-start.utc` / `battery-end.utc`).

Legs are **VERBATIM from `gates.json` at 1624f43** — six, not five. RF-003.json's
five-leg verification text predates the ast-grep leg; gates.json governs.

## REFERENCE-FORM AMENDMENT — FIRST INSTANCE, binding

- Every leg's log captured by **REDIRECT**. No `tee`, no pipeline anywhere near
  an rc.
- Every leg's rc captured by **`echo $?` on its own line into its OWN file**,
  `leg-N.rc`.
- **The tables below QUOTE the artifact files.** No instrument was re-read to
  populate them and no rc is restated from memory.
- No producer-side silence redirection anywhere: no `2>/dev/null`, no
  `|| true`, no `-q`.
- Leg 4's verbatim `&&`-chain is wrapped in `{ … }` so BOTH halves are captured;
  the rc is still the chain's own status and no pipeline is introduced.
- Leg 6 is **never piped** — gates.json's own note ("there is no pipefail, so a
  pipe would report the downstream tool's status") is honoured.

## The six legs

| # | leg | command (verbatim, gates.json) | rc (quoted from `leg-N.rc`) | log artifact |
| --- | --- | --- | --- | --- |
| 1 | fmt | `cargo fmt --all --check` | **0** | `leg-1-fmt.log` (empty — nothing to reformat) |
| 2 | clippy | `cargo clippy --workspace --all-targets --message-format=json --keep-going -- -D warnings` | **0** | `leg-2-clippy.json` + `leg-2-clippy.stderr` |
| 3 | wasm32-check | `cargo check -p beamr-wasm --target wasm32-unknown-unknown --locked` | **0** | `leg-3-wasm32check.log` |
| 4 | wasm-tests | `wasm-bindgen-test-runner --version && CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test --manifest-path crates/beamr-wasm/Cargo.toml --target wasm32-unknown-unknown --locked` | **0** | `leg-4-wasmtests.log` |
| 5 | tests | `cargo test --workspace` | **0** | `leg-5-tests.log` |
| 6 | blocking-call-in-native-bif | `ast-grep scan -r .ast-grep/rules/blocking-call-in-native-bif.yml crates/beamr/src/native/ --json` | **0** | `leg-6-astgrep.json` + `leg-6-astgrep.stderr` |

**6/6 GREEN in one sitting.**

### Leg 2 — clippy, structured verdict as well as exit code

gates.json's verdict for this leg is `exit-code-of-cmd` (rc 0 above). Its
`extract` jq was ALSO applied to `leg-2-clippy.json` as a cross-check:
**0 lines emitted** — no message at level `error` or `warning` anywhere in the
workspace. `leg-2-clippy.stderr` shows `Compiling beamr`, `Compiling beamr-wasm`
— the closure was fresh, not replayed from cache.

### Leg 4 — wasm tests, AS PARSED

```
awk '/^test result:/ {n++; p+=$4; f+=$6; i+=$8} END {...}' leg-4-wasmtests.log
blocks=2 passed=80 failed=0 ignored=0
```

### Leg 5 vs the REGISTERED BASELINE

Parse method, applied identically to both logs:

```
awk '/^test result:/ {n++; p+=$4; f+=$6; i+=$8} END {print "blocks="n" passed="p" failed="f" ignored="i}'
```

| quantity | baseline (`../rf003d3-baseline/baseline-test.log`) | leg 5 (`leg-5-tests.log`) | delta |
| --- | --- | --- | --- |
| harness result blocks | 72 | **72** | 0 — as required |
| passed | 2060 | **2064** | **+4** |
| failed | 0 | **0** | 0 |
| ignored | 0 | **0** | 0 |

**The +4 is accounted for by DIFFING THE NAMED-TEST LISTS, not by arithmetic.**
Artifact `leg-5-nametest-diff.log` (rc in `leg-5-nametest-diff.rc`), method:
`grep '^test <name> ... ok'` from each log → strip to the name → `sort` → `comm`.
Line counts 2060 and 2064 agree with the harness totals, which is itself a check
that the extraction lost nothing.

ADDED (4, named exactly):

```
distribution::etf::tests::deframe_boundary_accepts_exactly_max_and_refuses_one_over
distribution::etf::tests::deframe_refuses_extreme_header_without_allocating
distribution::etf::tests::deframe_refuses_length_above_max_dist_frame_bytes
distribution::etf::tests::deframe_zero_length_frame_does_not_hit_the_cap
```

REMOVED: **none** — the `## REMOVED` section of the artifact is empty.

These are exactly this lane's four new tests. Nothing else moved.

### Leg 6 — ast-grep

`leg-6-astgrep.json` content is `[]`; `leg-6-astgrep.stderr` is empty. Exit 0 =
no blocking construct in any native BIF body.

## Boundary — df before EVERY leg

Threshold **47710208 KiB**, noun and instrument in `THRESHOLD.txt`. Each file
echoes the value it read and its own verdict. No reading fell below threshold,
so no leg was held.

| before leg | KiB available (quoted from the .df) | artifact | margin over threshold |
| --- | --- | --- | --- |
| 1 fmt | 99699732 | `boundary-1-fmt.df` | +51989524 |
| 2 clippy | 99699612 | `boundary-2-clippy.df` | +51989404 |
| 3 wasm32-check | 99374704 | `boundary-3-wasm32check.df` | +51664496 |
| 4 wasm-tests | 99127504 | `boundary-4-wasmtests.df` | +51417296 |
| 5 tests | 98550572 | `boundary-5-tests.df` | +50840364 |
| 6 ast-grep | 97615384 | `boundary-6-astgrep.df` | +49905176 |
| after the battery | 100536560 | `boundary-7-post-battery.df` | +52826352 |

Lowest reading of the whole battery: **97615384 KiB**, clear of the threshold by
49905176 KiB (~47.6 GiB).

## du bracket

| point | du -sk (KiB) | artifact |
| --- | --- | --- |
| battery start | 6568908 | `du-start.txt` |
| battery end (**peak**) | 8654356 | `du-end.txt` |
| after `rm -rf target/*/incremental` (standing) | **5732144** | `du-post-incremental-delete.txt` |

`target/debug/incremental` measured **4139392 KiB** before deletion — the whole
of it was reclaimed plus nothing else; the delete is the standing post-battery
step, not a cleanup improvised to make a number look better.

Whole-lane peak is the battery end: **8654356 KiB ≈ 8.25 GiB**, INSIDE the
registered ~9.5 GiB measured-adjacent price and well inside the ~10.5 GiB
reportable ceiling. Post-delete residue 5732144 KiB ≈ 5.47 GiB.

## Prohibition grep

Run and recorded separately with its own per-grep rc artifacts:
`PROHIBITION.md`, `prohibition-{1,2,3}-*.log` / `.rc`. Summary: the two
production framing sites are both cap-first; ONE constant, no second literal;
one out-of-lane finding in `handshake.rs`, reported not fixed.
