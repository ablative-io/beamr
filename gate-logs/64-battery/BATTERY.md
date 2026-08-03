# BRIEF #64 — battery, six legs verbatim from gates.json at 92ca6c4

Run against the implementation commit `9628eca`, on branch
`artemis/dist-byte-bounds` in `/Users/tom/Developer/ablative/stack/beamr-64`.

- Battery start (UTC): `2026-08-03T00:49:20Z`
- Battery end (UTC): `2026-08-03T00:53:18Z`

Every leg's return code was read by `echo $?` on its own line, directly after
the command, never through a pipeline. Every leg's output was captured by
redirect. No producer-side silence redirection anywhere: no `2>/dev/null`, no
`|| true`, no `-q`. Leg 6 was NOT piped.

## Verdict: 6 / 6 GREEN

| # | Leg | Return code | Result |
| --- | --- | --- | --- |
| 1 | fmt | **0** | clean |
| 2 | clippy | **0** | 0 error/warning compiler-messages extracted |
| 3 | wasm32-check | **0** | clean |
| 4 | wasm-tests | **0** | 80 passed, 0 failed |
| 5 | tests | **0** | 2060 passed, 0 failed, 0 ignored, 72 blocks |
| 6 | blocking-call-in-native-bif | **0** | `[]` — no findings |

### Leg 1 — `cargo fmt --all --check`
rc 0. Log: `leg-1-fmt.log` (empty — nothing to reformat).

### Leg 2 — `cargo clippy --workspace --all-targets --message-format=json --keep-going -- -D warnings`
rc 0. Logs: `leg-2-clippy.json` (stdout), `leg-2-clippy.stderr`.
Applying gates.json's own extractor over the JSON — every `compiler-message`
at level `error` or `warning` — yields **0 rows**.

### Leg 3 — `cargo check -p beamr-wasm --target wasm32-unknown-unknown --locked`
rc 0. Log: `leg-3-wasm32check.log`.
Relevant because D5's envelope is 4 GiB, which does not fit a 32-bit `usize`;
the accept-side meter is `u64` throughout for exactly this reason.

### Leg 4 — wasm-tests
`wasm-bindgen-test-runner --version && CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test --manifest-path crates/beamr-wasm/Cargo.toml --target wasm32-unknown-unknown --locked`
rc 0. Runner version `wasm-bindgen-test-runner 0.2.123`. 80 passed, 0 failed.
Log: `leg-4-wasmtests.log`.

### Leg 5 — `cargo test --workspace`
rc 0. Log: `leg-5-tests.log`.

| Quantity | Baseline (`e8aa4f1`) | This run | Delta |
| --- | --- | --- | --- |
| passed | 2052 | **2060** | **+8** |
| failed | 0 | **0** | 0 |
| ignored | 0 | 0 | 0 |
| harness result blocks | 72 | **72** | 0 |
| `^error` lines | 0 | 0 | 0 |

The +8 is accounted for EXACTLY by the eight new tests, established by diffing
the named-test lists of the two logs rather than by inference. **8 added, 0
removed, 0 renamed away.** Named exactly:

1. `distribution::sender::tests::control_frame_encoded_sizes_measured_at_the_bytes`
2. `distribution::sender::tests::data_lane_bounds_retained_bytes_not_just_slot_count`
3. `distribution::sender::tests::control_lane_slot_cap_bounds_retained_bytes_below_one_max_data_frame`
4. `distribution::sender::tests::data_lane_releases_every_reservation_once_the_drain_completes`
5. `distribution::sender::tests::data_lane_byte_refusal_drops_promptly_without_blocking`
6. `distribution::sender::tests::queued_byte_charges_do_not_keep_the_sender_alive`
7. `distribution::connection::tests::the_accept_envelope_derives_the_sixty_four_peer_design_target`
8. `distribution::connection::tests::accept_loop_declines_inbound_peers_beyond_the_residency_envelope`

Note on the two counts: the named-test lists hold 2049 (baseline) and 2057
(this run) entries against totals of 2052 and 2060. The constant 3-line
shortfall is the doctest harness, which prints its test names in a different
format that the name extractor does not match. The DELTA is +8 on both
measures, which is what the reconciliation turns on.

### Leg 6 — `ast-grep scan -r .ast-grep/rules/blocking-call-in-native-bif.yml crates/beamr/src/native/ --json`
rc 0, output `[]`, stderr empty. Log: `leg-6-astgrep.json`, `leg-6-astgrep.stderr`.
Not piped. No findings, so the zero-indexed-vs-one-indexed line-number caveat
does not arise.

## Disk boundaries

Threshold: **47185920 KiB available** on `/System/Volumes/Data` (45 GiB).
Instrument: `df -k /System/Volumes/Data`. Recorded before EVERY leg; each
boundary file echoes the value it read.

| Before leg | UTC | KiB available | Over threshold |
| --- | --- | --- | --- |
| 1 fmt | 2026-08-03T00:49:21Z | 104502332 | YES |
| 2 clippy | 2026-08-03T00:49:35Z | 104501052 | YES |
| 3 wasm32-check | 2026-08-03T00:49:46Z | 104499720 | YES |
| 4 wasm-tests | 2026-08-03T00:50:14Z | 104253652 | YES |
| 5 tests | 2026-08-03T00:50:46Z | 103686820 | YES |
| 6 ast-grep | 2026-08-03T00:53:11Z | 102342912 | YES |

Never within 2x of the threshold. No leg was held or curtailed.

Raw: `boundary-{1..6}-*.df`, threshold file `THRESHOLD.txt`.

## Worktree size (`du -sk` on ../beamr-64)

| Point | KiB | GiB |
| --- | --- | --- |
| Before any build (baseline registration) | 21896 | 0.02 |
| Battery start | 7521504 | 7.17 |
| Battery end | 9673464 | 9.23 |

Against RULING 5's ~8 GiB cold price, the finished worktree is **9.23 GiB** —
about 1.2 GiB over the estimate. Disclosed rather than rounded: the estimate
was a price, and the measurement came in above it. Headroom was never in
question (102 GiB available at the tightest boundary).
