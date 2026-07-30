#!/usr/bin/env bash
# Rebuild the beamr-wasm browser bundle and stage all probe artifacts.
# Zero new deps; asserts pinned wasm-bindgen 0.2.123 and OTP 29 erlc before use.
# The OTP check asserts that erlc and its sibling erl share an installation before querying erl.
set -euo pipefail

HARNESS="$(cd "$(dirname "$0")" && pwd)"
REPO=/Users/annabel/Developer/ablative/stack/beamr
WT="$REPO/.worktrees/probesitting"
TARGET="$REPO/target"
WBG=/Users/annabel/Developer/ablative/artemis-artifacts/tools/wbg-0.2.123/bin

echo "== worktree HEAD (must be a399b54; panic-source.diff applied UNCOMMITTED) =="
git -C "$WT" rev-parse HEAD
git -C "$WT" diff --stat

echo "== cargo build (wasm32-unknown-unknown, release, -p beamr-wasm) =="
( cd "$WT" && CARGO_TARGET_DIR="$TARGET" cargo build --release --target wasm32-unknown-unknown -p beamr-wasm --locked )

echo "== wasm-bindgen (pinned 0.2.123 — asserted below) =="
export PATH="$WBG:$PATH"
WBG_PATH="$(command -v wasm-bindgen)"
WBG_VERSION_OUTPUT="$("$WBG_PATH" --version)"
WBG_VERSION="${WBG_VERSION_OUTPUT#wasm-bindgen }"
printf 'wasm-bindgen version: %s\n' "$WBG_VERSION"
printf 'wasm-bindgen resolved path: %s\n' "$WBG_PATH"
if [[ "$WBG_VERSION" != "0.2.123" ]]; then
  printf 'wasm-bindgen pin mismatch: wanted 0.2.123, got %s, resolved path %s\n' \
    "$WBG_VERSION" "$WBG_PATH" >&2
  exit 1
fi
"$WBG_PATH" "$TARGET/wasm32-unknown-unknown/release/beamr_wasm.wasm" \
  --target web --no-typescript --out-dir "$HARNESS/web/pkg"

echo "== compile workloads (OTP 29 erlc — asserted before use) =="
ERLC_PATH="$(command -v erlc)"
ERLC_ERL="${ERLC_PATH%/*}/erl"
if [[ ! -x "$ERLC_ERL" ]]; then
  printf 'cannot determine OTP version for erlc at %s: sibling erl is not executable at %s\n' \
    "$ERLC_PATH" "$ERLC_ERL" >&2
  exit 1
fi
physical_tool_path() {
  local tool_path="$1"
  local tool_dir
  local link_target

  tool_dir="$(cd -P "$(dirname "$tool_path")" && pwd)"
  tool_path="$tool_dir/$(basename "$tool_path")"
  while [[ -L "$tool_path" ]]; do
    link_target="$(readlink "$tool_path")"
    if [[ "$link_target" = /* ]]; then
      tool_path="$link_target"
    else
      tool_path="$(dirname "$tool_path")/$link_target"
    fi
    tool_dir="$(cd -P "$(dirname "$tool_path")" && pwd)"
    tool_path="$tool_dir/$(basename "$tool_path")"
  done
  printf '%s\n' "$tool_path"
}
ERLC_PHYSICAL_PATH="$(physical_tool_path "$ERLC_PATH")"
ERLC_ERL_PHYSICAL_PATH="$(physical_tool_path "$ERLC_ERL")"
printf 'erlc physical path: %s\n' "$ERLC_PHYSICAL_PATH"
printf 'sibling erl physical path: %s\n' "$ERLC_ERL_PHYSICAL_PATH"
if [[ "${ERLC_PHYSICAL_PATH%/*}" != "${ERLC_ERL_PHYSICAL_PATH%/*}" ]]; then
  printf 'cannot attribute OTP version to this erlc: erlc and sibling erl resolve to different physical directories\n' >&2
  printf 'erlc physical path: %s\n' "$ERLC_PHYSICAL_PATH" >&2
  printf 'sibling erl physical path: %s\n' "$ERLC_ERL_PHYSICAL_PATH" >&2
  exit 1
fi
ERLC_OTP="$("$ERLC_ERL" -noshell -eval 'io:format("~s", [erlang:system_info(otp_release)]), halt().')"
printf 'erlc OTP major version: %s\n' "$ERLC_OTP"
printf 'erlc resolved path: %s\n' "$ERLC_PATH"
if [[ "$ERLC_OTP" != "29" ]]; then
  printf 'erlc OTP pin mismatch: wanted 29, got %s, resolved path %s\n' \
    "$ERLC_OTP" "$ERLC_PATH" >&2
  exit 1
fi
( cd "$HARNESS/workloads" && for m in panic_probe throttle_probe strand_probe io_probe; do "$ERLC_PATH" "$m.erl"; done )

echo "== stage artifacts into web/artifacts =="
cp "$HARNESS"/workloads/*.beam "$HARNESS/web/artifacts/"
FX="$REPO/crates/beamr-wasm/fixtures"
cp "$FX"/fetch_chain_a.beam "$FX"/fetch_chain_b.beam "$FX"/fetch_chain_c.beam \
   "$FX"/fetch_cycle_ping.beam "$FX"/fetch_cycle_pong.beam "$HARNESS/web/artifacts/"

echo "== done =="
ls "$HARNESS/web/pkg"
ls "$HARNESS/web/artifacts"
