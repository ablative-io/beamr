# RAIL 6 serialization audit — beamr suite under in-binary parallelism

Fix-wave `3aecb622-e7d1-431c-aaa1-80efb5db63dd`, hygiene brief step 0.
Audited at tree `74c7d3c` (branch point of `diana/beamr-hygiene-3aecb622`), 2026-07-29.
Scope per the dispatch: beamr's actual exposure is **in-binary parallelism under
`cargo test --workspace`** (tests within one binary run on parallel threads by
default; test binaries themselves serialize). Not nextest — beamr has no nextest
stage; the nextest framing belongs to frame's rig.

## Verdict

**No serialization dependence blocks leg 5 as it stands.** The suite is
parallel-safe by deliberate design in every class checked, with one genuine
low-severity exposure recorded below as a lead for a future lane (no code change
here; this step is deliverable-only by the brief's own text).

## Classes checked, at the bytes

**Network ports — clean.** Every bind in the suite is ephemeral
(`127.0.0.1:0` / `(Ipv4Addr::LOCALHOST, 0)`): `inet_bifs.rs`,
`tcp_bifs_tests.rs`, `udp_bifs.rs`, `distribution_mesh_handshake.rs`, the
distribution e2e binaries. No fixed port anywhere; no port-collision surface.

**Working directory — clean.** Zero `set_current_dir` calls estate-wide in
`crates/`.

**Temp paths — clean, with documented intent.** Every real-filesystem temp path
is uniquified by pid and/or nanos: `beamr-cli/src/tests.rs`
(`temp_replay_log_path`, `write_temp_beam`), `sample_workflow_e2e.rs` (which
carries its own comment: "a fixed shared path lets concurrent `cargo test`
invocations on the same host delete each other's output mid-assertion"),
`differential.rs`, `file_meta_bifs_tests.rs` (`temp_path` = name + pid). The
fixed `"/tmp/beamr-stat-test"` / `"/tmp/beamr-list-test"` strings in
`file_meta_bifs_tests.rs` are assertion *values* matched against a fake
`IoOp` sink — no real filesystem touch.

**Process-global telemetry state — mitigated, coverage verified complete.**
The known-flake class the brief names. `telemetry::test_lock`
(`telemetry/mod.rs:23-41`) serializes every test that installs process-global
OTel providers or drives the `INSTRUMENTS` `OnceLock` (`metrics.rs:123`).
Verified 1:1 at the bytes: all provider-install call sites sit directly under a
guard acquisition — `scheduler/tests.rs` :727/:728, :804/:805, :915/:916,
:1030/:1031; `spans.rs` :334 (sole test in its module using
`install_test_provider`); `lifecycle_tests.rs` :280, :309. Residual bleed
(guarded tests observing each other's spans across the shared slot) is handled
by the pid-narrowing idiom, documented in-tree at `spans.rs:346-348`. Note:
`telemetry` is **not** a default feature (`lib.rs:46-47`,
`Cargo.toml default = [...]`), so none of this compiles into leg 5's binaries —
exposure exists only in explicit `--features telemetry` runs.

**File-level statics in integration binaries — disjoint by design.**
`bif_registry_replacement.rs`: 1 test, statics unshared by construction.
`dirty_scheduler.rs`: 4 tests, per-test statics with explicit intent
(`:419-421`: "A dedicated counter so this test never races the shared
NORMAL_PROGRESS"). `suspend_wakeup.rs`: `PHASE` vs `DIRTY_PHASE`/
`DIRTY_INVOCATIONS`, one test each. `suspend_reexec.rs` /
`suspend_result_binary.rs`: `RUNS` maps keyed by per-invocation id — safe under
parallel tests by construction.

**Environment mutation — ONE live exposure, low severity, recorded as a lead.**
`otp_stubs/tests.rs:124` `os_putenv_and_unsetenv_round_trip` exercises
`bif_os_putenv`/`bif_os_unsetenv` (`erlang_stubs.rs:88-120`), which perform
process-global `std::env::set_var`/`remove_var` inside `unsafe` blocks whose
SAFETY comments explicitly acknowledge that concurrent environment access from
other threads "can be undefined on some platforms" (edition 2024 — the calls
are unsafe precisely for this). The test is logically collision-free (unique
key `BEAMR_TEST_B170_PUTENV`, previous value saved and restored), but it runs
in the beamr unit binary alongside parallel test threads that concurrently
*read* the environment via `std::env::temp_dir()` (TMPDIR lookup) in at least
`file_meta_bifs_tests.rs` and `differential`-adjacent unit modules — i.e., the
exact concurrent read/write interleaving the SAFETY contract defers is
exercised by the default parallel run. Probability is low (narrow window,
macOS/Linux libc tolerance in practice) and no flake has been attributed to it,
but it is a real serialization dependence under RAIL 6's definition.

*Lead for a future lane (not this brief):* either an `env_lock` mirroring
`telemetry::test_lock` held by the round-trip test and any future env-touching
tests, or moving the round-trip behind a spawned-subprocess harness. Cheap
either way; not reached for here because step 0 is a no-code deliverable.

**beamr-cli binary — clean, with a forward note for this brief's own legs.**
Current tests are pure `parse_args` value assertions plus pid+nanos-unique temp
files; no env mutation, no shared state. The leg A/B walls added by this brief
must preserve that: per-test unique fixture directories, no shared fixture
mutation at runtime.

**wasm leg — out of scope.** Leg 4 runs under `wasm-bindgen-test-runner`,
single-threaded; in-binary parallelism does not apply.

## Standing consequence for this brief's battery

Leg 5 (`cargo test --workspace`) runs with default in-binary parallelism, as
the repo intends: nothing in the suite requires `--test-threads=1`, and this
audit discharges RAIL 6 for the parallel leg at this seat. If leg 5 shows a red
that smells like contention: disclose the load line, re-run quiet, treat only a
solo-surviving red as real — and carry any loaded red forward as a lead per the
corrected load-evidence law, with the putenv exposure above as the first place
to look for an environment-shaped flake.
