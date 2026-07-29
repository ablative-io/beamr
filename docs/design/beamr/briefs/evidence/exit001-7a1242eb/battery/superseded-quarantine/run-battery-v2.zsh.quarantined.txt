#!/bin/zsh
# EXIT-001 five-leg gate battery — CLAIM CONVENTION v2 conformant runner.
# Task 7a1242eb-742b-48be-ba7e-d961f2c49f04; machine: Annabel's box;
# operator: Diana Plum (b337ce2b-336a-4856-a9d8-54c90496c9fa).
#
# Spec: THE PIN (re-brief Artemis 17:03Z, sha256 57f990a5…, stripped
# framing; amends dispatch 17ba6688…) — built against exactly these six
# entries on ablative/stack, all under the anchor, restated verbatim:
#   1. 4b8b38e1-c99c-49ca-a9cb-692fa073e3c1 — anchor (CLAIM CONVENTION v2)
#   2. e903b4ad-209e-4478-a656-6c8d83357ae9 — Amendment 1 (phase field; abort property; retirement mechanics)
#   3. c6d998bc-f9c0-497c-9b85-725ff1e9f195 — Amendment 2 (fork reality; dispatch-time check; rule-5 floor/ceiling)
#   4. aa92a18c — Addendum to Amendment 2 (behavioral check standard; structural question parked)
#   5. 91ba17f9 — Seth's record update (stricter rule-5 variant WITHDRAWN, grounds)
#   6. c3ee8385 — ratification (rule-5 floor uniform on both boxes; ceiling clause with fail-loudly lesson)
# Reference shape: Minerva's instrument. Fresh authorship — the landed
# hygiene runner (evidence/hygiene-3aecb622/battery/run-battery.sh) is
# pre-v2, evidence-bearing, and is never edited in place (Amendment 1 item 4).
#
# Shape: take the claim FIRST at /tmp/ablative-gate-battery.claim (atomic
# create; body carries seat, member id, pid, started-at UTC, phase), THEN
# drain-wait UNDER the held claim — 30-second samples, 60-minute ceiling,
# every sample recorded. Refusal ONLY on timeout: loud, recorded,
# run-stopping (Amendment 1 item 2 — never quietly proceed past the
# ceiling). The quiet CENSUS (zero foreign cargo/rustc), not the claim, is
# what the evidence cites as proof of quiet floor (anchor rule 6).
# phase: "draining" at create -> "running" when legs begin (Amendment 1 item 1).
#
# Rule-5 handling is the FLOOR EXACTLY, the uniform shape (stricter
# variants WITHDRAWN per pin entries 5+6): a STALE claim (holder pid dead)
# has its verbatim contents recorded — in the drain-record, which lands as
# committed evidence under this task, and the operator posts it to the lane
# — then is CLEARED and the run proceeds; failure-to-clear is loud (retry
# with a message), never a silent fall-through. Distinct case: a claim held
# by a LIVE holder is anchor rule 2, not rule 5 — this runner holds
# (samples every 30 s under the same 60-minute ceiling, recorded) and never
# races it; on ceiling, loud refusal.
#
# The five leg commands are VERBATIM from the EXIT-001 brief's
# .verification (= beamr gates.json; ruling entry 85d5781b, Waffles
# 2026-07-29 12:15Z) — sequenced here, never re-derived.
set -u

# Per-box absolute paths, explicit (Amendment 2 item 1 — never inherited by fork)
WT=/Users/annabel/Developer/ablative/stack/beamr/.wt-exit001-7a1242eb
EV=$WT/docs/design/beamr/briefs/evidence/exit001-7a1242eb/battery
CLAIM=/tmp/ablative-gate-battery.claim
SEAT="Diana Plum"
MEMBER=b337ce2b-336a-4856-a9d8-54c90496c9fa
CEILING_SECS=3600
SAMPLE_SECS=30

cd "$WT" || { echo "FATAL: worktree missing at $WT"; exit 3 }
mkdir -p "$EV"
DRAIN=$EV/drain-record.txt
: > "$DRAIN"

note() { echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) [$SEAT/$MEMBER @ Annabel's box] $*" | tee -a "$DRAIN" }

CLAIM_OWNED=0
release_claim() {
  if [[ $CLAIM_OWNED -eq 1 ]]; then
    rm -f "$CLAIM"
    CLAIM_OWNED=0
    note "claim released"
  fi
}
trap 'release_claim' EXIT INT TERM

write_claim() {  # $1 = phase
  cat > "$CLAIM" <<EOF
seat=$SEAT
member=$MEMBER
pid=$$
started=$STARTED_AT
phase=$1
tree=$(git -C "$WT" rev-parse HEAD)
EOF
}

census() {  # foreign compile census: any cargo/rustc/cargo-nextest not ours.
  # During drain this runner launches no compiles, so every match is foreign.
  ps -axo pid,command | grep -E '(^|/)(cargo|rustc|cargo-nextest)( |$)' | grep -v grep
}

ELAPSED=0

# --- acquire the claim (atomic create; contention policy per header) ---
STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
while true; do
  if (set -C; write_claim draining) 2>/dev/null; then
    CLAIM_OWNED=1
    note "claim TAKEN (atomic create), phase=draining, pid=$$, started=$STARTED_AT"
    break
  fi
  HOLDER_PID=$(grep '^pid=' "$CLAIM" 2>/dev/null | cut -d= -f2)
  if [[ -n "${HOLDER_PID:-}" ]] && ! kill -0 "$HOLDER_PID" 2>/dev/null; then
    note "STALE CLAIM found (holder pid $HOLDER_PID dead) — verbatim contents recorded below for the lane, then cleared (rule-5 floor, pin entries 5+6):"
    sed 's/^/    stale> /' "$CLAIM" | tee -a "$DRAIN"
    while ! rm -f "$CLAIM" || [[ -e "$CLAIM" ]]; do
      note "LOUD: stale-claim clear FAILED at $CLAIM — retrying in 5s (never a silent fall-through)"
      sleep 5
    done
    note "stale claim cleared; proceeding"
    continue
  fi
  note "claim held by live holder (pid ${HOLDER_PID:-unknown}) — holding, never racing; sample at ${ELAPSED}s"
  if [[ $ELAPSED -ge $CEILING_SECS ]]; then
    note "REFUSED — TIMEOUT at ${CEILING_SECS}s waiting for a live claim to clear. Run STOPPED; nothing launched."
    exit 42
  fi
  sleep $SAMPLE_SECS; ELAPSED=$((ELAPSED + SAMPLE_SECS))
done

# --- drain-wait UNDER the held claim: 30 s samples, 60-minute ceiling ---
while true; do
  FOREIGN=$(census)
  COUNT=$(echo -n "$FOREIGN" | grep -c . )
  note "drain sample at ${ELAPSED}s: foreign cargo/rustc count=$COUNT"
  if [[ $COUNT -gt 0 ]]; then
    echo "$FOREIGN" | sed 's/^/    foreign> /' >> "$DRAIN"
  else
    note "CENSUS QUIET: zero foreign cargo/rustc — this census line is the quiet-floor proof of record (anchor rule 6)"
    break
  fi
  if [[ $ELAPSED -ge $CEILING_SECS ]]; then
    note "REFUSED — DRAIN TIMEOUT at ${CEILING_SECS}s: floor never quiet. Loud, recorded, run-stopping (Amendment 1 item 2). Claim released; NO legs launched."
    exit 42
  fi
  sleep $SAMPLE_SECS; ELAPSED=$((ELAPSED + SAMPLE_SECS))
done

# --- phase flip: draining -> running (Amendment 1 item 1) ---
write_claim running
note "claim phase -> running; legs begin"

# --- battery proper (carried forward from the hygiene shape) ---
{
  echo "=== battery start (UTC) ==="; date -u
  echo "=== machine/operator ==="; echo "Annabel's box / $SEAT ($MEMBER)"
  echo "=== tree ==="; git rev-parse HEAD; git status --porcelain | wc -l | xargs echo "dirty-entries:"
  echo "=== toolchain ==="; rustc -vV; cargo --version; wasm-bindgen-test-runner --version
  echo "=== convention pin set (self-describing evidence; re-brief 57f990a5…, restated verbatim) ==="
  echo "1. 4b8b38e1-c99c-49ca-a9cb-692fa073e3c1 — anchor (CLAIM CONVENTION v2)"
  echo "2. e903b4ad-209e-4478-a656-6c8d83357ae9 — Amendment 1 (phase field; abort property; retirement mechanics)"
  echo "3. c6d998bc-f9c0-497c-9b85-725ff1e9f195 — Amendment 2 (fork reality; dispatch-time check; rule-5 floor/ceiling)"
  echo "4. aa92a18c — Addendum to Amendment 2 (behavioral check standard; structural question parked)"
  echo "5. 91ba17f9 — Seth's record update (stricter rule-5 variant WITHDRAWN, grounds)"
  echo "6. c3ee8385 — ratification (rule-5 floor uniform on both boxes; ceiling clause with fail-loudly lesson)"
  echo "=== quiet-floor proof: census (cited, per anchor rule 6) ==="
  tail -n 3 "$DRAIN"
} > "$EV/battery-header.txt" 2>&1

run_leg() {
  local name="$1"; shift
  { echo "=== load before $name ==="; uptime } >> "$EV/battery-header.txt"
  "$@" > "$EV/$name.log" 2>&1
  local exit_code=$?
  echo "exit=$exit_code" >> "$EV/$name.log"
  echo "$name exit=$exit_code [Annabel's box / $SEAT]" >> "$EV/exits.txt"
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
  echo "BATTERY COMPLETE: all five legs invoked; per-leg exits in exits.txt; drain record in drain-record.txt"
} >> "$EV/battery-header.txt"
cat "$EV/exits.txt"
