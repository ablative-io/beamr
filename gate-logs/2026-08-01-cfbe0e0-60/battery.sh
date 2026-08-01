#!/bin/sh
# #60 battery: dist frame cap + guard-move (Waffles ratified 64 MiB, msg 92cc05d4;
# Cally's behavioral-red conditional DISCHARGED at cfbe0e0 — red-at-base artifact
# in-tree at gate-logs/2026-08-01-red-at-base-60/, measured twice independently).
# WINDOW: declared by Cally Ray 03:40:25Z at her hands (56.55 GiB free), valid for
# this head per its own terms (tree changed only by the test+artifact commit).
# PRE-REGISTERED EXPECTATION (re-registered before this run, register-then-run law):
# workspace tests 2049 -> 2052, moved by EXACTLY THREE.
# Full six-leg gates.json battery, commands VERBATIM. Incremental ON (Waffles'
# standing refusal of CARGO_INCREMENTAL=0). Per-leg stdout+stderr to <leg>.log
# (captured, never suppressed); per-leg rc to <leg>.exit. COMPLETE.marker records
# the RUN FINISHED — not a verdict; verdicts are the per-leg exit codes.
cd /Users/tom/Developer/ablative/stack/beamr || exit 97
LOGDIR=gate-logs/2026-08-01-cfbe0e0-60

git rev-parse HEAD > "$LOGDIR/TREE" 2>&1
date -u +%Y-%m-%dT%H:%M:%SZ > "$LOGDIR/STARTED"

cargo fmt --all --check > "$LOGDIR/fmt.log" 2>&1
echo $? > "$LOGDIR/fmt.exit"

cargo clippy --workspace --all-targets --message-format=json --keep-going -- -D warnings > "$LOGDIR/clippy.log" 2>&1
echo $? > "$LOGDIR/clippy.exit"

cargo check -p beamr-wasm --target wasm32-unknown-unknown --locked > "$LOGDIR/wasm32-check.log" 2>&1
echo $? > "$LOGDIR/wasm32-check.exit"

sh -c 'wasm-bindgen-test-runner --version && CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test --manifest-path crates/beamr-wasm/Cargo.toml --target wasm32-unknown-unknown --locked' > "$LOGDIR/wasm-tests.log" 2>&1
echo $? > "$LOGDIR/wasm-tests.exit"

cargo test --workspace > "$LOGDIR/tests.log" 2>&1
echo $? > "$LOGDIR/tests.exit"

ast-grep scan -r .ast-grep/rules/blocking-call-in-native-bif.yml crates/beamr/src/native/ --json > "$LOGDIR/blocking-call-in-native-bif.log" 2>&1
echo $? > "$LOGDIR/blocking-call-in-native-bif.exit"

{ date -u +%Y-%m-%dT%H:%M:%SZ; du -sk target 2>&1; } > "$LOGDIR/final-footprint.txt"
date -u +%Y-%m-%dT%H:%M:%SZ > "$LOGDIR/COMPLETE.marker"
