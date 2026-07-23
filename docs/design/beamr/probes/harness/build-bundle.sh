#!/usr/bin/env bash
# Rebuild the beamr-wasm browser bundle and stage all probe artifacts.
# Zero new deps; uses the pinned wasm-bindgen 0.2.123 and OTP 29 erlc on PATH.
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

echo "== wasm-bindgen (pinned 0.2.123 — verify below) =="
export PATH="$WBG:$PATH"
wasm-bindgen --version
wasm-bindgen "$TARGET/wasm32-unknown-unknown/release/beamr_wasm.wasm" \
  --target web --no-typescript --out-dir "$HARNESS/web/pkg"

echo "== compile workloads (OTP 29 erlc) =="
( cd "$HARNESS/workloads" && for m in panic_probe throttle_probe strand_probe io_probe; do erlc "$m.erl"; done )

echo "== stage artifacts into web/artifacts =="
cp "$HARNESS"/workloads/*.beam "$HARNESS/web/artifacts/"
FX="$REPO/crates/beamr-wasm/fixtures"
cp "$FX"/fetch_chain_a.beam "$FX"/fetch_chain_b.beam "$FX"/fetch_chain_c.beam \
   "$FX"/fetch_cycle_ping.beam "$FX"/fetch_cycle_pong.beam "$HARNESS/web/artifacts/"

echo "== done =="
ls "$HARNESS/web/pkg"
ls "$HARNESS/web/artifacts"
