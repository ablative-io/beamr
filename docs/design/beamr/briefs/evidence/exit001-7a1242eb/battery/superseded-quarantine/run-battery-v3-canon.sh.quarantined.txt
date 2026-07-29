#!/bin/bash
# Canonical gate battery — claim-integrated amended form.
# Requires: SEAT_NAME, MEMBER_ID, EVIDENCE_DIR (committed gate-logs dir). Refuses to run without them.
set -u

# ================= INSTANTIATION (beamr / EXIT-001, Annabel's box) =================
# Canon-instantiated runner: preamble below is BYTE-VERBATIM from stack-root
# entry 622dedbf-ff42-4405-a6a5-9ffcfb00676c (sha256 with-LF 7773e852... /
# 10163B, stripped 5b75314a... / 10162B — both verified at this seat before
# copying), except the two disclosed deltas named in battery-header.txt:
# this instantiation block (the canon's own required parameters, set
# per-box, absolute, never inherited by fork) and the census extension
# (+wasm-bindgen-test-runner). Phases 3-4 are beamr's, per re-brief 2.
SEAT_NAME="Diana Plum"
MEMBER_ID="b337ce2b-336a-4856-a9d8-54c90496c9fa"
WT=/Users/annabel/Developer/ablative/stack/beamr/.wt-exit001-7a1242eb
EVIDENCE_DIR="$WT/docs/design/beamr/briefs/evidence/exit001-7a1242eb/battery"
cd "$WT" || exit 2
# ===================================================================================

CLAIM=/tmp/ablative-gate-battery.claim
: "${SEAT_NAME:?SEAT_NAME must be set (operator seat name)}"
: "${MEMBER_ID:?MEMBER_ID must be set (full member UUID, copied never typed)}"
: "${EVIDENCE_DIR:?EVIDENCE_DIR must be set (committed evidence dir for this run)}"
mkdir -p "$EVIDENCE_DIR"
CLAIMLOG="$EVIDENCE_DIR/claim.log"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

log() { printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" | tee -a "$CLAIMLOG"; }

census() {  # exact-name census of compile actors; output = "name pid" lines, empty = quiet
  local p
  for p in cargo rustc cargo-nextest cargo-clippy clippy-driver rustdoc wasm-bindgen-test-runner; do
    pgrep -x "$p" 2>>"$CLAIMLOG" | sed "s/^/$p /"
  done
}

release_claim() {
  if [ -f "$CLAIM" ]; then
    rm -f "$CLAIM"
    log "claim released"
  fi
}
trap release_claim EXIT INT TERM

write_claim() {  # $1 = phase; started_at preserved across the phase flip
  printf 'seat=%s\nmember_id=%s\npid=%s\nstarted_at=%s\nphase=%s\n' \
    "$SEAT_NAME" "$MEMBER_ID" "$$" "$STARTED_AT" "$1" > "$CLAIM.tmp.$$" \
    && mv "$CLAIM.tmp.$$" "$CLAIM"
}

acquire() {  # atomic create via noclobber; failure output recorded, never discarded
  ( set -o noclobber; printf 'seat=%s\nmember_id=%s\npid=%s\nstarted_at=%s\nphase=draining\n' \
      "$SEAT_NAME" "$MEMBER_ID" "$$" "$STARTED_AT" > "$CLAIM" ) 2>>"$CLAIMLOG"
}

# ---- PHASE 0: load + census before anything ----
uptime | tee "$EVIDENCE_DIR/load-before.txt"

# ---- PHASE 1: acquire the claim (hold, never race; ceiling 60 min; timeout = LOUD exit 4) ----
n=0
until acquire; do
  if [ -f "$CLAIM" ]; then
    holder="$(cat "$CLAIM" 2>>"$CLAIMLOG")" || holder="(claim vanished mid-read)"
    hpid="$(printf '%s\n' "$holder" | sed -n 's/^pid=//p')"
    if [ -n "$hpid" ] && ! kill -0 "$hpid" 2>>"$CLAIMLOG"; then
      # Rule 5 floor: record verbatim, clear, proceed — loudly, never silently.
      printf '%s\n' "$holder" | tee "$EVIDENCE_DIR/stale-claim-record.txt" >> "$CLAIMLOG"
      log "STALE CLAIM (holder pid $hpid dead) recorded above — OPERATOR MUST post its verbatim contents to the launcher's lane with this run's evidence; clearing and proceeding"
      rm -f "$CLAIM"
      continue
    fi
    log "claim held by: ${holder//$'\n'/ } — waiting (sample $n)"
  fi
  n=$((n+1))
  if [ "$n" -ge 120 ]; then
    log "ACQUISITION TIMEOUT after 60 minutes — refusing loudly"
    exit 4
  fi
  sleep 30
done
log "claim acquired, phase=draining"

# ---- PHASE 2: drain-wait under the held claim (30s samples, all recorded; ceiling 60 min; timeout = LOUD exit 5) ----
n=0
while :; do
  BUSY="$(census)"
  { printf -- '--- drain sample %s ---\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"; printf '%s\n' "${BUSY:-quiet}"; uptime; } >> "$EVIDENCE_DIR/drain-samples.log"
  [ -z "$BUSY" ] && break
  n=$((n+1))
  if [ "$n" -ge 120 ]; then
    log "DRAIN TIMEOUT after 60 minutes — floor never quieted; refusing loudly, releasing claim"
    exit 5
  fi
  sleep 30
done
write_claim running
log "floor quiet at census; phase=running"
census > "$EVIDENCE_DIR/census-at-start.txt"   # the census, not the claim, is the quiet-floor proof

# ---- battery header: provenance, pin set, toolchain (self-describing evidence) ----
{
  echo "=== battery start (UTC) ==="; date -u
  echo "=== machine/operator ==="; echo "Annabel's box / $SEAT_NAME ($MEMBER_ID)"
  echo "=== tree ==="; git rev-parse HEAD; git status --porcelain | wc -l | xargs echo "dirty-entries:"
  echo "=== toolchain ==="; rustc -vV; cargo --version; wasm-bindgen-test-runner --version
  echo "=== provenance (re-brief 2 item 6) ==="
  echo "preamble: BYTE-VERBATIM from stack-root entry 622dedbf-ff42-4405-a6a5-9ffcfb00676c @ sha256 7773e852dbb9358f0e804779853d5536a939351d0324293813c67b842d84e807 (with-LF, 10163B) / 5b75314a50200aee3a004ce242fc030a9129f0749a23e103d1a212c0ab1788fc (stripped, 10162B), both verified at this seat"
  echo "disclosed deltas from verbatim: (1) instantiation block = the canon's required SEAT_NAME/MEMBER_ID/EVIDENCE_DIR + per-box absolute worktree, set in-script (instantiation pattern ratified, entry edb63b89); (2) census extension: +wasm-bindgen-test-runner (beamr's foreign-runner class — extension, not fork)"
  echo "=== launch environment (load-bearing, stated loud — Seth flag 4) ==="
  echo "launched from worktree root $WT via bash (never zsh — canon is #!/bin/bash + PIPESTATUS); SEAT_NAME/MEMBER_ID/EVIDENCE_DIR set in-script by the instantiation block (values above); NO database URL is needed for beamr's legs — that absence is stated here rather than silent"
  echo "canon defects inherited as written (estate stance: canon-owner's to amend, never patched per-seat): exit-4 trap releases an unheld claim; kill -0 reads EPERM as dead; SIGHUP untrapped — Seth's flags 1-3, entry c527324b; contention risk noted, not silent"
  echo "legs: beamr's FIVE, VERBATIM from EXIT-001 brief .verification @ 8f2b7c3 (= gates.json; ruling entry 85d5781b). beamr has NO nextest leg: canon amendments A4 (--all-targets drop on nextest) and A6 (Summary-line count-match) are N/A here, stated rather than silently dropped"
  echo "=== convention pin set (eight entries, re-brief 2, restated verbatim) ==="
  echo "1. 4b8b38e1-c99c-49ca-a9cb-692fa073e3c1 — anchor (CLAIM CONVENTION v2)"
  echo "2. e903b4ad-209e-4478-a656-6c8d83357ae9 — Amendment 1 (phase field; abort property; retirement mechanics)"
  echo "3. c6d998bc-f9c0-497c-9b85-725ff1e9f195 — Amendment 2 (fork reality; dispatch-time check; rule-5 floor/ceiling)"
  echo "4. aa92a18c-59cd-49c0-b606-10ab7037c301 — Addendum to Amendment 2 (behavioral check standard; structural question parked)"
  echo "5. 91ba17f9-e1aa-43c0-b9d6-beaed18538c7 — Seth's record update (stricter rule-5 variant WITHDRAWN, grounds)"
  echo "6. c3ee8385-cbd1-463c-8b86-43da2a684547 — ratification (rule-5 floor uniform on both boxes; ceiling clause with fail-loudly lesson)"
  echo "7. bdce0b78-ebcb-44b3-bbc0-07780b476f6f — superseding pointer (canon carries preamble at source)"
  echo "8. 622dedbf-ff42-4405-a6a5-9ffcfb00676c — the claim-integrated canonical script (preamble source)"
  echo "=== quiet-floor proof: census-at-start.txt (the census, not the claim — anchor rule 6) ==="
} > "$EVIDENCE_DIR/battery-header.txt" 2>&1

# ---- PHASE 3: beamr's five gate legs, VERBATIM from the EXIT-001 brief (stderr kept per A5; exits via PIPESTATUS; nothing masked, no || true) ----
{ echo "=== load before leg1-fmt ==="; uptime; } >> "$EVIDENCE_DIR/battery-header.txt"
cargo fmt --all --check \
  > "$EVIDENCE_DIR/leg1-fmt.log" 2>"$EVIDENCE_DIR/leg1-fmt.stderr.log"
FMT_EXIT="${PIPESTATUS[0]}"
log "leg1-fmt exit=$FMT_EXIT [Annabel's box / $SEAT_NAME]"

{ echo "=== load before leg2-clippy ==="; uptime; } >> "$EVIDENCE_DIR/battery-header.txt"
cargo clippy --workspace --all-targets -- -D warnings \
  > "$EVIDENCE_DIR/leg2-clippy.log" 2>"$EVIDENCE_DIR/leg2-clippy.stderr.log"
CLIPPY_EXIT="${PIPESTATUS[0]}"
log "leg2-clippy exit=$CLIPPY_EXIT [Annabel's box / $SEAT_NAME]"

{ echo "=== load before leg3-wasm32-check ==="; uptime; } >> "$EVIDENCE_DIR/battery-header.txt"
cargo check -p beamr-wasm --target wasm32-unknown-unknown --locked \
  > "$EVIDENCE_DIR/leg3-wasm32-check.log" 2>"$EVIDENCE_DIR/leg3-wasm32-check.stderr.log"
WASM32_CHECK_EXIT="${PIPESTATUS[0]}"
log "leg3-wasm32-check exit=$WASM32_CHECK_EXIT [Annabel's box / $SEAT_NAME]"

{ echo "=== load before leg4-wasm-tests ==="; uptime; } >> "$EVIDENCE_DIR/battery-header.txt"
( wasm-bindgen-test-runner --version && CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test --manifest-path crates/beamr-wasm/Cargo.toml --target wasm32-unknown-unknown --locked ) \
  > "$EVIDENCE_DIR/leg4-wasm-tests.log" 2>"$EVIDENCE_DIR/leg4-wasm-tests.stderr.log"
WASM_TESTS_EXIT="${PIPESTATUS[0]}"
log "leg4-wasm-tests exit=$WASM_TESTS_EXIT [Annabel's box / $SEAT_NAME]"

{ echo "=== load before leg5-tests ==="; uptime; } >> "$EVIDENCE_DIR/battery-header.txt"
cargo test --workspace \
  > "$EVIDENCE_DIR/leg5-tests.log" 2>"$EVIDENCE_DIR/leg5-tests.stderr.log"
TESTS_EXIT="${PIPESTATUS[0]}"
log "leg5-tests exit=$TESTS_EXIT [Annabel's box / $SEAT_NAME]"

# A7 ADAPTED, NOT COPIED (re-brief 2 item 3): the canon's "not covered" line would be FALSE here.
echo "DOC TESTS: COVERED via leg5-tests (cargo test --workspace runs doc tests)" | tee "$EVIDENCE_DIR/doc-tests-coverage.txt"

# ---- PHASE 4: closing evidence + marker (canonical shape; five leg exits; standing riders) ----
census > "$EVIDENCE_DIR/census-at-end.txt"
uptime | tee "$EVIDENCE_DIR/load-after.txt"
{ echo "=== load after leg5 ==="; uptime; echo "=== battery end (UTC) ==="; date -u; } >> "$EVIDENCE_DIR/battery-header.txt"
FINISHED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
{
  echo "battery=COMPLETE"
  echo "machine=Annabel's box"
  echo "seat=$SEAT_NAME"
  echo "member_id=$MEMBER_ID"
  echo "pid=$$"
  echo "started_at=$STARTED_AT"
  echo "finished_at=$FINISHED_AT"
  echo "fmt_exit=$FMT_EXIT"
  echo "clippy_exit=$CLIPPY_EXIT"
  echo "wasm32_check_exit=$WASM32_CHECK_EXIT"
  echo "wasm_tests_exit=$WASM_TESTS_EXIT"
  echo "tests_exit=$TESTS_EXIT"
} | tee "$EVIDENCE_DIR/COMPLETE.marker"
log "battery complete; exits fmt=$FMT_EXIT clippy=$CLIPPY_EXIT wasm32-check=$WASM32_CHECK_EXIT wasm-tests=$WASM_TESTS_EXIT tests=$TESTS_EXIT"
exit 0
