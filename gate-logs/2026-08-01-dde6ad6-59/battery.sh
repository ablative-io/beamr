#!/bin/sh
# #59 battery: statistics/1 refusal — native meets its own wall (GO #59-1, Cally Ray
# 02:52Z; WINDOW DECLARED 03:07:41Z at her hands, df -k /System/Volumes/Data = 72.33 GiB
# free; my corroborating figure 71.2 GiB). Full six-leg gates.json battery, commands
# VERBATIM from gates.json at the tree below. Incremental ON (Waffles' standing refusal
# of CARGO_INCREMENTAL=0). Per-leg stdout+stderr captured to <leg>.log (captured, never
# suppressed); per-leg rc to <leg>.exit. COMPLETE.marker records that the RUN FINISHED —
# it is not a verdict; verdicts are the per-leg exit codes. Leg 6 is UNPIPED; its own rc
# is the gate.
cd /Users/tom/Developer/ablative/stack/beamr || exit 97
LOGDIR=gate-logs/2026-08-01-dde6ad6-59

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
