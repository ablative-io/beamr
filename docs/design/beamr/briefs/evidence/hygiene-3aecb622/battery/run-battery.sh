#!/bin/zsh
# Gate battery for fix-wave 3aecb622 leg 1 (dispatch 00b809ac).
# The five leg commands are VERBATIM from the dispatch text (= beamr
# gates.json); this harness only sequences them and captures evidence:
# per-leg load line, full log, raw exit code, completion marker.
printf '%s\n' 'REFUSAL: run-battery.sh is a CLOSED RECORD of fix-wave 3aecb622 leg 1. Re-running it would overwrite real evidence from an unidentified tree. Running it requires deliberately deleting this refusal.' >&2
exit 1
set -u
# Absolute worktree root: a relative cd walked one level short on the first
# invocation and every leg redirection failed before any cargo ran (dead
# harness run, no legs invoked — noted for the record, nothing partial).
cd /Users/annabel/Developer/ablative/stack/beamr/.wt-hygiene-3aecb622
EV=docs/design/beamr/briefs/evidence/hygiene-3aecb622/battery

{
  echo "=== battery start (UTC) ==="; date -u
  echo "=== tree ==="; git rev-parse HEAD; git status --porcelain | wc -l | xargs echo "dirty-entries:"
  echo "=== toolchain ==="; rustc -vV; cargo --version; wasm-bindgen-test-runner --version
} > "$EV/battery-header.txt" 2>&1

run_leg() {
  local name="$1"; shift
  { echo "=== load before $name ==="; uptime } >> "$EV/battery-header.txt"
  "$@" > "$EV/$name.log" 2>&1
  local exit_code=$?
  echo "exit=$exit_code" >> "$EV/$name.log"
  echo "$name exit=$exit_code" >> "$EV/exits.txt"
  return 0
}

: > "$EV/exits.txt"
run_leg leg1-fmt          cargo fmt --all --check
run_leg leg2-clippy       cargo clippy --workspace --all-targets -- -D warnings
run_leg leg3-wasm32-check cargo check -p beamr-wasm --target wasm32-unknown-unknown --locked
run_leg leg4-wasm-tests   zsh -c 'wasm-bindgen-test-runner --version && CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test --manifest-path crates/beamr-wasm/Cargo.toml --target wasm32-unknown-unknown --locked'
run_leg leg5-tests        cargo test --workspace

{
  echo "=== load after leg5 ==="; uptime
  echo "=== battery end (UTC) ==="; date -u
  echo "BATTERY COMPLETE: all five legs invoked; per-leg exits in exits.txt"
} >> "$EV/battery-header.txt"
cat "$EV/exits.txt"
