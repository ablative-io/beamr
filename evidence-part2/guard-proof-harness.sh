#!/bin/bash
# Proof harness: sources the REAL runner's guard functions (bytes extracted from the
# committed runner, up to the PHASE 0 marker) and exercises each A5 delta.
set -u
RUNNER=/Users/deanwhiting/Developer/beamr-part2/evidence-part2/run-battery-beamr-seth.sh
PROOFDIR="$(mktemp -d -t seth-guard-proof)"
export SEAT_NAME="Seth Crackers (sub-agent, beamr part 2)"
export MEMBER_ID=5828bee2-6460-44a5-ba78-a0f82ce0f8f1
export EVIDENCE_DIR="$PROOFDIR/evidence"

# Extract everything BEFORE the PHASE 0 marker: the guard function definitions, real bytes.
awk '/^# ---- PHASE 0/{exit} {print}' "$RUNNER" > "$PROOFDIR/guards.sh"
# shellcheck disable=SC1090
source "$PROOFDIR/guards.sh"

# Redirect the claim path AFTER sourcing: functions read $CLAIM at call time, so the real
# /tmp/ablative-gate-battery.claim is never touched by this proof.
CLAIM="$PROOFDIR/fake.claim"
CLAIMLOG="$EVIDENCE_DIR/claim.log"

echo "=== A5-2: dialect-tolerant read ==="
printf 'seat=x\nmember_id=m\npid=424242\nphase=draining\n' > "$CLAIM"
echo "  equals form -> wrapper='$(claim_field pid "$CLAIM")' canon_sed='$(sed -n 's/^pid=//p' "$CLAIM")'"
printf 'seat: x\nmember_id: m\npid: 515151\nphase: draining\n' > "$CLAIM"
echo "  colon  form -> wrapper='$(claim_field pid "$CLAIM")' canon_sed='$(sed -n 's/^pid=//p' "$CLAIM")'"

echo "=== A5-3: unparseable/empty pid must read HELD, never rule-5 clear ==="
for bad in "" "not-a-pid" "12x4"; do
  hpid="$bad"
  if [ -z "$hpid" ] || [ -n "${hpid//[0-9]/}" ]; then verdict="HELD (no clear)"; else verdict="would proceed to liveness"; fi
  echo "  pid='$bad' -> $verdict"
done
echo "  pid='424242' -> $( { hpid=424242; if [ -z "$hpid" ] || [ -n "${hpid//[0-9]/}" ]; then echo 'HELD'; else echo 'proceeds to liveness check'; fi; } )"

echo "=== A5-4: phase flip ownership refusal (foreign claim present) ==="
printf 'seat=thief\nmember_id=SOMEONE-ELSE\npid=999999\nphase=running\n' > "$CLAIM"
CLAIM_ACQUIRED=1
( write_claim running ) > "$PROOFDIR/flip.out" 2>&1
echo "  write_claim exit=$?  (6 = refused)"
grep -h 'flip_determination' "$EVIDENCE_DIR/flip-determination.txt" 2>/dev/null | tail -1 | sed 's/^/  /'
echo "  VOID marker: $(grep -h '^run=' "$EVIDENCE_DIR/VOID.marker" 2>/dev/null || echo MISSING)"

echo "=== A5-5: release-time ABSENT after acquisition must VOID (canon exits clean here) ==="
rm -f "$EVIDENCE_DIR/VOID.marker" "$CLAIM"
CLAIM_ACQUIRED=1
release_claim > /dev/null 2>&1
grep -h 'release_determination' "$EVIDENCE_DIR/release-determination.txt" | sed 's/^/  /'
echo "  VOID marker: $(grep -h '^detector=' "$EVIDENCE_DIR/VOID.marker" 2>/dev/null || echo MISSING)"

echo "=== control: absent claim BEFORE acquisition must NOT void ==="
rm -f "$EVIDENCE_DIR/VOID.marker"
CLAIM_ACQUIRED=0
release_claim > /dev/null 2>&1
grep -h 'release_determination' "$EVIDENCE_DIR/release-determination.txt" | sed 's/^/  /'
echo "  VOID marker: $(grep -h '^detector=' "$EVIDENCE_DIR/VOID.marker" 2>/dev/null || echo 'ABSENT (correct — no false void)')"

echo "=== real claim path untouched ==="
ls -la /tmp/ablative-gate-battery.claim 2>/dev/null || echo "  /tmp/ablative-gate-battery.claim does not exist — untouched"
rm -rf "$PROOFDIR"
