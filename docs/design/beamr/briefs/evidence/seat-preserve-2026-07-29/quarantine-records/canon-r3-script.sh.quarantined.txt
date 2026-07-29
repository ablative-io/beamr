#!/bin/bash
# Canonical gate battery — claim-integrated amended form, revision 2 (Seth's flags 1-5 folded).
# Requires: SEAT_NAME, MEMBER_ID, EVIDENCE_DIR (committed gate-logs dir). Refuses to run without them.
# LAUNCH CONTRACT (flag 4): the launch line is load-bearing — launch with bash, from the repo root,
# with every env var the dispatching brief names (e.g. a test-database URL). The script enforces
# what it can below; the dispatcher's brief MUST name the rest and the operator's evidence MUST echo it.
set -u

if [ -z "${BASH_VERSION:-}" ]; then   # flag 5: refuse BEFORE the claim, not die mid-claim under zsh
  echo "REFUSING: this script requires bash (PIPESTATUS); launch with bash or exec it directly" >&2
  exit 2
fi
if [ ! -f Cargo.toml ]; then          # flag 4 (partial enforcement): canon Phase 3 is cargo-shaped
  echo "REFUSING: not at a cargo repo root (no Cargo.toml in \$PWD)" >&2
  exit 2
fi

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
  for p in cargo rustc cargo-nextest cargo-clippy clippy-driver rustdoc; do
    pgrep -x "$p" 2>>"$CLAIMLOG" | sed "s/^/$p /"
  done
}

claim_is_mine() {  # flag 1: ownership check — the claim is ours only if its pid line is our pid
  [ -f "$CLAIM" ] && [ "$(sed -n 's/^pid=//p' "$CLAIM" 2>>"$CLAIMLOG")" = "$$" ]
}

pid_alive() {  # flag 2: ps -p, not kill -0 — EPERM on a live foreign-user process must read as ALIVE
  ps -p "$1" > /dev/null 2>>"$CLAIMLOG"
}

release_claim() {
  if claim_is_mine; then
    rm -f "$CLAIM"
    log "claim released (own claim, pid $$)"
  elif [ -f "$CLAIM" ]; then
    log "exit without release: claim on file is NOT ours (holder $(sed -n 's/^pid=//p' "$CLAIM" 2>>"$CLAIMLOG")) — leaving it in place"
  fi
}
trap release_claim EXIT INT TERM HUP   # flag 3: HUP included — a hangup must not orphan the claim

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
    if [ -n "$hpid" ] && ! pid_alive "$hpid"; then   # flag 2 applied at the call site: ps -p, never kill -0
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

# ---- PHASE 3: gate legs (Tom's semantics; stderr kept; exits captured; nothing masked) ----
cargo check \
  --workspace \
  --all-targets \
  --message-format=json \
  --keep-going \
  2>>"$EVIDENCE_DIR/check.stderr.log" |
  tee "$EVIDENCE_DIR/check.json.log" |
  jq -c '
    select(.reason == "compiler-message")
    | select(.message.level == "error" or .message.level == "warning")
    | {
        type: "clippy",
        level: .message.level,
        file: ([.message.spans[]? | select(.is_primary)][0].file_name // .message.spans[0]?.file_name // "unknown"),
        line: ([.message.spans[]? | select(.is_primary)][0].line_start // .message.spans[0]?.line_start // null),
        column: ([.message.spans[]? | select(.is_primary)][0].column_start // .message.spans[0]?.column_start // null),
        lint: (.message.code.code // "compile-error"),
        message: .message.message
      }
  ' > "$EVIDENCE_DIR/check.extract.jsonl"
CHECK_EXIT="${PIPESTATUS[0]}"

cargo clippy \
  --workspace \
  --all-targets \
  --message-format=json \
  --keep-going \
  -- \
  -D warnings \
  2>>"$EVIDENCE_DIR/clippy.stderr.log" |
  tee "$EVIDENCE_DIR/clippy.json.log" |
  jq -c '
    select(.reason == "compiler-message")
    | select(.message.level == "error" or .message.level == "warning")
    | {
        type: "clippy",
        level: .message.level,
        file: ([.message.spans[]? | select(.is_primary)][0].file_name // .message.spans[0]?.file_name // "unknown"),
        line: ([.message.spans[]? | select(.is_primary)][0].line_start // .message.spans[0]?.line_start // null),
        column: ([.message.spans[]? | select(.is_primary)][0].column_start // .message.spans[0]?.column_start // null),
        lint: (.message.code.code // "compile-error"),
        message: .message.message
      }
  ' > "$EVIDENCE_DIR/clippy.extract.jsonl"
CLIPPY_EXIT="${PIPESTATUS[0]}"

NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1 \
  cargo nextest run \
    --workspace \
    --cargo-quiet \
    --message-format libtest-json-plus \
    --no-fail-fast \
    2>>"$EVIDENCE_DIR/nextest.stderr.log" |
  tee "$EVIDENCE_DIR/nextest.json.log" |
  jq -c '
    select(.type == "test" and .event == "failed")
    | {
        type: "nextest",
        test: .name,
        stdout: (.stdout // ""),
        message: "test failed"
      }
  ' > "$EVIDENCE_DIR/nextest.extract.jsonl"
NEXTEST_EXIT="${PIPESTATUS[0]}"

# A6: the Summary line is load-bearing; extract it beside the JSON so the count-match is checkable at the bytes
grep -E 'Summary \[' "$EVIDENCE_DIR/nextest.stderr.log" | tee "$EVIDENCE_DIR/summary-line.txt"

# A7: convert the silent gap to a loud statement
echo "DOC TESTS: NOT COVERED by this battery (no doc-test leg in the canonical script)" | tee "$EVIDENCE_DIR/not-covered.txt"

# ---- PHASE 4: closing evidence + marker ----
census > "$EVIDENCE_DIR/census-at-end.txt"
uptime | tee "$EVIDENCE_DIR/load-after.txt"
FINISHED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
{
  echo "battery=COMPLETE"
  echo "seat=$SEAT_NAME"
  echo "member_id=$MEMBER_ID"
  echo "pid=$$"
  echo "started_at=$STARTED_AT"
  echo "finished_at=$FINISHED_AT"
  echo "check_exit=$CHECK_EXIT"
  echo "clippy_exit=$CLIPPY_EXIT"
  echo "nextest_exit=$NEXTEST_EXIT"
} | tee "$EVIDENCE_DIR/COMPLETE.marker"
log "battery complete; exits check=$CHECK_EXIT clippy=$CLIPPY_EXIT nextest=$NEXTEST_EXIT"
exit 0
