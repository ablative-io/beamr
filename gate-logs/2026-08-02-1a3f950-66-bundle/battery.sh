#!/bin/sh
# BUNDLE-66 battery: the six-leg gates.json battery for the beamr leg of BRIEF-66 r2
# (brief sha256 5535413528f783e38a1b6cd637a7a064ba7017df9981947d6985196652f1d130,
# re-verified at fire time). Commands are VERBATIM from gates.json at the tree below,
# authored once in the brief. Incremental ON (Waffles' standing refusal of
# CARGO_INCREMENTAL=0) and NOT deleted here — the incremental measure-then-delete
# bracket is coordinator teardown.
#
# Capture, not suppression: per-leg stdout+stderr go to <leg>.log; nothing is sent to
# /dev/null, no `|| true`, no `-q`. Per-leg rc is captured DIRECT with `echo $?`,
# never through a pipeline. Leg 6 is UNPIPED (no pipefail on this box, so a pipe
# would report the downstream tool's status instead of ast-grep's) and its --json
# line numbers are ZERO-indexed against ast-grep's one-indexed human report.
#
# COMPLETE.marker records that the RUN FINISHED. It is NOT a verdict — the verdicts
# are the per-leg exit codes in <leg>.exit.
#
# DISK GATE: a `df -k /System/Volumes/Data` reading is written to boundary-<leg>.df
# BEFORE EVERY LEG and checked against THRESHOLD (41943040 KiB avail). A boundary
# reading under the bar means the current leg finishes and NOTHING further starts:
# the script writes HALTED-UNDER-BAR and exits without starting that leg or any later
# one.
cd /Users/tom/Developer/ablative/stack/beamr-66 || exit 97
LOGDIR=gate-logs/2026-08-02-1a3f950-66-bundle
BAR=41943040

# boundary <legname> — write the pre-leg df reading, then gate on it.
# Returns 0 to proceed, 1 to halt. The df reading is written by a bare producing
# command; the parse afterwards consumes the written FILE, not a pipe off df.
boundary() {
  {
    date -u +%Y-%m-%dT%H:%M:%SZ
    df -k /System/Volumes/Data
  } > "$LOGDIR/boundary-$1.df" 2>&1
  avail=$(awk '$1 ~ /^\/dev\// {print $4}' "$LOGDIR/boundary-$1.df")
  if [ -z "$avail" ]; then
    printf 'boundary-%s.df did not yield an avail field; halting rather than guessing\n' "$1" \
      > "$LOGDIR/HALTED-UNDER-BAR"
    return 1
  fi
  if [ "$avail" -lt "$BAR" ]; then
    printf 'HALTED before leg %s: %s KiB avail < %s KiB bar (df -k /System/Volumes/Data)\n' \
      "$1" "$avail" "$BAR" > "$LOGDIR/HALTED-UNDER-BAR"
    date -u +%Y-%m-%dT%H:%M:%SZ >> "$LOGDIR/HALTED-UNDER-BAR"
    return 1
  fi
  return 0
}

git rev-parse HEAD > "$LOGDIR/TREE" 2>&1
date -u +%Y-%m-%dT%H:%M:%SZ > "$LOGDIR/STARTED"

boundary fmt || exit 0
cargo fmt --all --check > "$LOGDIR/fmt.log" 2>&1
echo $? > "$LOGDIR/fmt.exit"

boundary clippy || exit 0
cargo clippy --workspace --all-targets --message-format=json --keep-going -- -D warnings > "$LOGDIR/clippy.log" 2>&1
echo $? > "$LOGDIR/clippy.exit"

boundary wasm32-check || exit 0
cargo check -p beamr-wasm --target wasm32-unknown-unknown --locked > "$LOGDIR/wasm32-check.log" 2>&1
echo $? > "$LOGDIR/wasm32-check.exit"

boundary wasm-tests || exit 0
sh -c 'wasm-bindgen-test-runner --version && CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test --manifest-path crates/beamr-wasm/Cargo.toml --target wasm32-unknown-unknown --locked' > "$LOGDIR/wasm-tests.log" 2>&1
echo $? > "$LOGDIR/wasm-tests.exit"

boundary tests || exit 0
cargo test --workspace > "$LOGDIR/tests.log" 2>&1
echo $? > "$LOGDIR/tests.exit"

boundary blocking-call-in-native-bif || exit 0
ast-grep scan -r .ast-grep/rules/blocking-call-in-native-bif.yml crates/beamr/src/native/ --json > "$LOGDIR/blocking-call-in-native-bif.log" 2>&1
echo $? > "$LOGDIR/blocking-call-in-native-bif.exit"

{ date -u +%Y-%m-%dT%H:%M:%SZ; du -sk target 2>&1; } > "$LOGDIR/final-footprint.txt"
date -u +%Y-%m-%dT%H:%M:%SZ > "$LOGDIR/COMPLETE.marker"
