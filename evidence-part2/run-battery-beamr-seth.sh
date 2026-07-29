#!/bin/bash
# Canonical gate battery — claim-integrated amended form, revision 2 (Seth's flags 1-5 folded).
# Requires: SEAT_NAME, MEMBER_ID, EVIDENCE_DIR (committed gate-logs dir). Refuses to run without them.
# LAUNCH CONTRACT (flag 4): the launch line is load-bearing — launch with bash, from the repo root,
# with every env var the dispatching brief names (e.g. a test-database URL). The script enforces
# what it can below; the dispatcher's brief MUST name the rest and the operator's evidence MUST echo it.
#
# =====================================================================================
# DERIVATION HEADER — beamr part 2, task 3aecb622
# =====================================================================================
# MACHINE:  Dean's box (/Users/deanwhiting/Developer/beamr-part2)
# OPERATOR: Seth Crackers (sub-agent, beamr part 2)
# REPO:     beamr @ branch seth/replay-rebuild-3aecb622, base 192e4a4
#
# CANON IDENTITY (the pair, never the artifact's self-description):
#   stack-root entry e269d2c9-0dfa-409e-ada5-b10303118225
#   sha256 ff831516f8ff74de1f54023ff4af54d80cade6a285b6ff42f5a37dbfda6290e4 (11532B with-LF framing)
#   extracted runnable script sha256 bf404f6a1d4d475442d078b185a993cd2d18c6d708589dd2e33d0ee29f259806
#   Both verified at the bytes on this box before derivation.
#
# KNOWN AND RULED, NOT RE-FLAGGED, NOT FIXED LOCALLY: line 2 above self-describes as
# "revision 2". The LABEL is wrong; the CONTENT is r3; the bytes are FROZEN by ruling.
# Canon identity is the entry-id + hash pair, not the artifact's self-description.
#
# ------------------------------------------------------------------------------------
# THIS IS NOT A NULL DIFF FROM CANON r3. Deltas are DISCLOSED below.
# Per anchor rider 77b2c212 / reads a621f353: canon r3 itself carries two defects, so a
# null-diff claim would be an assertion that this runner INHERITED both. It does not.
# When the canonical artifact carries the defect, verbatim inheritance IS the debt.
# ------------------------------------------------------------------------------------
#
# DISCLOSED A5 DELTAS (all four rules of a621f353), implemented as WRAPPER behaviour.
# CANON r3'S BYTES ARE NOT PATCHED — r4 is landing under another owner; a local patch to
# canon would be exactly the fork the instantiation pattern exists to prevent.
#
#   A5-1  WRITE canon's dialect exactly (key=value, no spaces, no colon form).
#         DELTA: NONE. r3 already complies (write_claim / acquire use printf 'key=%s\n').
#         Deliberately NOT "improved". Disclosed as inherited-correct, not as a null diff.
#
#   A5-2  READ dialect-tolerantly — accept both `key=value` and `key: value`.
#         DELTA: ADDED. r3's `sed -n 's/^pid=//p'` is DIALECT-BLIND: a runner writing
#         `pid: 1234` reads as EMPTY through it. New helper claim_field() accepts both
#         forms and strips optional surrounding space. It replaces all three r3 read
#         sites (claim_is_mine, release_claim, and the acquire loop's holder parse).
#
#   A5-3  AN UNPARSEABLE OR EMPTY PID IS NEVER GROUNDS FOR A RULE-5 CLEAR.
#         DELTA: ADDED. r3 gates its rule-5 clear on `[ -n "$hpid" ]` only, so a pid that
#         parses to a non-numeric value still reaches the liveness check. Here the pid must
#         be non-empty AND all-digits before any liveness check is believed; anything else
#         reads HELD and waits. Cannot-determine fails toward HELD, never toward stale.
#         Rule 5 requires a pid actually read and a liveness check that actually ran.
#
#   A5-4  NO PHASE FLIP WITHOUT AN OWNERSHIP CHECK.
#         DELTA: ADDED. r3's release_claim correctly refuses to remove a claim that is not
#         ours, but write_claim's draining->running flip is an UNCONDITIONAL mv. The exit
#         path was hardened and the write path left open. write_claim now re-reads the
#         claim after the flip and confirms BOTH our member_id AND our pid.
#
#   A5-5  RELEASE-TIME VOIDING ON FOREIGN **OR ABSENT**.
#         DELTA: ADDED. The foreign-claim detector does NOT fire in the ordering where the
#         thief finishes first: A holds; B flips over A's claim; B completes and cleanly
#         releases ITS OWN claim; A completes, goes to release, and finds NO CLAIM AT ALL.
#         A release path treating "nothing on file" as a tidy no-op exits clean and A
#         publishes a green with neither detector firing. There is no innocent reason our
#         own claim is missing — we wrote it, held it, and trapped its release on every
#         exit path. If it is gone, someone took it. The absent branch is checked
#         EXPLICITLY below; it does not structurally inherit the foreign branch's
#         behaviour and would otherwise exit clean by default.
#         Scoped by CLAIM_ACQUIRED: before we ever acquire, a foreign or absent claim is
#         ordinary and voids nothing.
#
# PRE-EMPTION, STATED PRECISELY (and it EXCEEDS the ruled floor — read this):
#   The coordinator's limit — "a wrapper around FROZEN canon cannot refuse before canon's
#   internal `mv`, only detect immediately after" — describes a wrapper invoking canon as a
#   BLACK BOX. This runner is a DERIVATION from the canonical script, so write_claim is our
#   own code: it refuses BEFORE the mv, leaving a foreign claim intact, and re-verifies
#   after. Pre-emption is claimed here because we actually have it, not assumed.
#
#   THIS MATTERS BECAUSE POST-FLIP-ONLY IS VACUOUS. write_claim writes our own content,
#   so a check that reads the claim back after the mv confirms our own write and passes
#   even when a live foreign claim was just clobbered. The first cut of this guard did
#   exactly that; the proof harness caught it (exit 0, flip_determination=OURS_CONFIRMED,
#   foreign claim silently overwritten). A post-flip-only guard is therefore reported as
#   INSUFFICIENT, not merely limited — worth flagging to whoever lands r4.
#
# ALL THREE DETECTORS, SAME MEANING:
#   (a) post-flip ownership check fails  -> our claim was replaced mid-run;
#   (b) release-time "claim on file is NOT ours" (already in frozen canon's exit path)
#       -> we were the VICTIM of a clobber rather than the perpetrator;
#   (c) release-time "claim ABSENT after we acquired" -> a thief flipped over our claim and
#       finished first, releasing its own. Canon exits clean here; we do not.
#   Any firing => the quiet-floor premise is VOID => THE RUN IS VOID AS EVIDENCE,
#   not a green. Reported, never silently re-run over the top of.
#
# WRAPPED-NESS IS VERIFIABLE FROM THE EVIDENCE, NOT ASSUMED (venue requirement).
# The guards bind only runs that go through this wrapper, so an unwrapped launch would
# yield a green indistinguishable from a guarded one. This bundle therefore carries
# records an unwrapped run could not produce:
#   - this disclosure header, citing a621f353 (in the runner, committed);
#   - $EVIDENCE_DIR/dialect-preflight.txt   — the dialect-tolerant reader exercised on BOTH
#     `key=value` and `key: value` before the claim is touched;
#   - $EVIDENCE_DIR/flip-determination.txt  — the flip-time ownership determination;
#   - $EVIDENCE_DIR/release-determination.txt — the release-time foreign-or-absent
#     determination, written on every exit path.
# A reader can tell from the bundle alone that it went through the guards.
#
# DERIVATION PROVENANCE: derived from the byte-verified EXTRACTED CANONICAL script
# (sha256 bf404f6a1d4d475442d078b185a993cd2d18c6d708589dd2e33d0ee29f259806), re-verified on
# this box at derivation time. NOT copied from any runner found lying in a scratchpad, and
# nothing was taken from QUARANTINE-do-not-execute/ (three superseded runners, mode 000,
# one carrying an unguarded `rm -f "$CLAIM"` that deletes a live foreign claim on
# acquisition timeout). Nothing from that directory was read, resurrected, or executed.
#
# QUIET-FLOOR BASIS (cited ALONGSIDE the census, never instead of it):
#   Every battery on this box tonight is sequenced serially by the coordinator — one
#   authorized battery at a time, no concurrent claim holders — and all live lanes here run
#   the same r3 derivation, so they speak one dialect. The acquisition race is therefore
#   structurally absent at this venue rather than merely unobserved. The census below
#   remains the actual proof.
#
# DATABASE, STATED LOUDLY: BEAMR NEEDS NO DATABASE. MERIDIAN_TEST_DATABASE_URL is NOT
# required and is NOT assumed present. No leg here reads one. A leg silently running
# against a wrong or absent database is the silent-variant class the estate is closing;
# this battery has no database surface at all.
#
# LEGS: beamr's five from gates.json, VERBATIM AND IN ORDER — NOT meridian's three.
# Canon r3's claim/census/evidence machinery is reused; its Phase 3 legs are replaced.
#   (1) cargo fmt --all --check
#   (2) cargo clippy --workspace --all-targets -- -D warnings
#   (3) cargo check -p beamr-wasm --target wasm32-unknown-unknown --locked
#   (4) wasm-bindgen-test-runner --version && CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=... cargo test ...
#   (5) cargo test --workspace
#
# NEXTEST: DECLARED N/A. beamr HAS NO NEXTEST STAGE. Canon r3's nextest leg and its A6
# Summary-line extraction are therefore not applicable here. Declared, never silently
# dropped. No nextest-shaped amendment is carried.
#
# DOC TESTS: COVERED by leg 5. `cargo test --workspace` runs doc tests for the workspace's
# library targets. Canon r3's A7 line reads "DOC TESTS: NOT COVERED" — that line was
# written for a runner whose only test leg was nextest, which does not run doc tests. It is
# NOT copied here, because it would be false. Loud and true beats loud and copied; the
# Doc-tests sections are extracted from leg 5's log as proof.
# =====================================================================================
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

# ---- A5-2 DELTA: dialect-tolerant claim reader (a621f353 rule 2) ----
# Accepts BOTH `key=value` and `key: value`; strips optional surrounding space.
# Canon r3's `sed -n 's/^pid=//p'` is dialect-blind and reads `pid: 1234` as EMPTY,
# which (through canon's rule-5 path) lets a LIVE claim rule as a dead-holder stale
# claim. This helper replaces every r3 read site. We still WRITE key=value (A5-1).
claim_field() {  # $1 = key, $2 = file
  [ -f "$2" ] || return 1
  sed -n "s/^$1[[:space:]]*[=:][[:space:]]*//p" "$2" 2>>"$CLAIMLOG" | head -n 1
}

claim_is_mine() {  # flag 1: ownership check — the claim is ours only if its pid line is our pid
  [ -f "$CLAIM" ] && [ "$(claim_field pid "$CLAIM")" = "$$" ]
}

pid_alive() {  # flag 2: ps -p, not kill -0 — EPERM on a live foreign-user process must read as ALIVE
  ps -p "$1" > /dev/null 2>>"$CLAIMLOG"
}

# ---- A5-4 / release-detector: mark the run VOID AS EVIDENCE ----
VOID_REASON=""
mark_void() {  # $1 = which detector, $2 = detail
  VOID_REASON="$1: $2"
  {
    echo "run=VOID_AS_EVIDENCE"
    echo "detector=$1"
    echo "detail=$2"
    echo "seat=$SEAT_NAME"
    echo "member_id=$MEMBER_ID"
    echo "pid=$$"
    echo "observed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "meaning=quiet-floor premise VOID; this run is NOT a green and must NOT be re-run over the top of without reporting"
  } | tee "$EVIDENCE_DIR/VOID.marker" >> "$CLAIMLOG"
  log "RUN VOID AS EVIDENCE — $1: $2"
}

CLAIM_ACQUIRED=0   # set to 1 only once OUR claim exists on file

release_claim() {
  # Recorded on EVERY exit path, so the bundle always carries a release determination.
  local determination detail
  if claim_is_mine; then
    rm -f "$CLAIM"
    determination="OURS_RELEASED"
    detail="own claim, pid $$"
    log "claim released (own claim, pid $$)"
  elif [ "$CLAIM_ACQUIRED" != "1" ]; then
    # Exited before we ever held a claim: a foreign or absent claim is ordinary here and
    # voids nothing. Scoping the detectors this way keeps an acquisition timeout (exit 4)
    # from falsely voiding a run that never had a claim to lose.
    determination="NOT_ACQUIRED"
    detail="exited before acquiring; nothing of ours to release"
    log "exit before acquisition — nothing of ours to release"
  elif [ -f "$CLAIM" ]; then
    # Frozen canon already carries this refusal. Per coordinator ruling it is the SAME
    # detector as the post-flip check: it is how we discover we were the VICTIM of a
    # clobber rather than the perpetrator.
    determination="FOREIGN_VOID"
    detail="claim on file belongs to pid $(claim_field pid "$CLAIM"), member $(claim_field member_id "$CLAIM")"
    log "exit without release: claim on file is NOT ours (holder $(claim_field pid "$CLAIM")) — leaving it in place"
    mark_void "release-time-foreign" "$detail"
  else
    # A5-5: ABSENT is voiding exactly as FOREIGN is. Canon exits clean here — that is the
    # hole. We wrote this claim, held it, and trapped its release; if it is gone, a thief
    # flipped over it and released its own first.
    determination="ABSENT_VOID"
    detail="our claim is GONE at release though we acquired it — no innocent cause exists"
    log "exit without release: our claim is ABSENT though we acquired it"
    mark_void "release-time-absent" "$detail"
  fi
  {
    echo "release_determination=$determination"
    echo "detail=$detail"
    echo "claim_acquired=$CLAIM_ACQUIRED"
    echo "our_pid=$$"
    echo "our_member_id=$MEMBER_ID"
    echo "observed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } > "$EVIDENCE_DIR/release-determination.txt"
}
trap release_claim EXIT INT TERM HUP   # flag 3: HUP included — a hangup must not orphan the claim

write_claim() {  # $1 = phase; started_at preserved across the phase flip
  # ---- A5-4 DELTA: ownership check at the phase flip (a621f353 rule 4) ----
  # Canon r3's flip is an UNCONDITIONAL mv with no ownership check: the exit path was
  # hardened and the write path left open, and that asymmetry clobbered a live claim on
  # another box.
  #
  # THE CHECK IS PRE-FLIP, AND THAT IS DELIBERATE. A post-flip-only check is VACUOUS:
  # write_claim writes our own content and would then read back our own write, so it
  # confirms OURS_CONFIRMED even with a foreign claim on file. (Verified — the first cut of
  # this guard did exactly that, and the proof harness caught it: exit 0,
  # flip_determination=OURS_CONFIRMED, foreign claim silently clobbered.)
  #
  # The coordinator's stated limit — "a wrapper around frozen canon cannot refuse before
  # canon's internal mv, only detect immediately after" — describes a wrapper that invokes
  # canon as a BLACK BOX. This runner is a DERIVATION: write_claim is our own code, so we
  # can and do refuse BEFORE the mv. That is strictly stronger than the ruled floor, and it
  # is claimed here as pre-emption we actually have rather than a limit we do not need.
  local holder_pid holder_member flip_verdict
  holder_pid="$(claim_field pid "$CLAIM")"
  holder_member="$(claim_field member_id "$CLAIM")"
  if [ ! -f "$CLAIM" ]; then
    flip_verdict="REFUSED_ABSENT"        # our claim vanished before the flip — same theft
  elif [ "$holder_pid" != "$$" ] || [ "$holder_member" != "$MEMBER_ID" ]; then
    flip_verdict="REFUSED_NOT_OURS"
  else
    flip_verdict="OURS_CONFIRMED"
  fi
  {
    echo "flip_determination=$flip_verdict"
    echo "phase=$1"
    echo "checked=PRE-FLIP (before the mv), plus post-flip re-verification below"
    echo "holder_pid_before_flip=$holder_pid"
    echo "holder_member_before_flip=$holder_member"
    echo "our_pid=$$"
    echo "our_member_id=$MEMBER_ID"
    echo "observed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } >> "$EVIDENCE_DIR/flip-determination.txt"
  if [ "$flip_verdict" != "OURS_CONFIRMED" ]; then
    log "REFUSING PHASE FLIP ($flip_verdict): claim is not ours BEFORE the mv (holder pid '$holder_pid' member '$holder_member', ours pid '$$' member '$MEMBER_ID') — foreign claim left intact"
    mark_void "phase-flip-$flip_verdict" "claim replaced mid-run; holder pid '$holder_pid' member '$holder_member'"
    exit 6
  fi

  printf 'seat=%s\nmember_id=%s\npid=%s\nstarted_at=%s\nphase=%s\n' \
    "$SEAT_NAME" "$MEMBER_ID" "$$" "$STARTED_AT" "$1" > "$CLAIM.tmp.$$" \
    && mv "$CLAIM.tmp.$$" "$CLAIM"

  # Post-flip re-verification: closes the narrow window between the pre-flip read and the
  # mv. Cheap, and it is the only part a black-box wrapper could have done at all.
  local after_pid after_member
  after_pid="$(claim_field pid "$CLAIM")"
  after_member="$(claim_field member_id "$CLAIM")"
  {
    echo "post_flip_pid=$after_pid"
    echo "post_flip_member=$after_member"
    echo "post_flip_verdict=$( [ "$after_pid" = "$$" ] && [ "$after_member" = "$MEMBER_ID" ] && echo OURS_CONFIRMED || echo RACED_NOT_OURS )"
  } >> "$EVIDENCE_DIR/flip-determination.txt"
  if [ "$after_pid" != "$$" ] || [ "$after_member" != "$MEMBER_ID" ]; then
    log "PHASE FLIP RACED: claim is not ours immediately after the mv (holder pid '$after_pid' member '$after_member')"
    mark_void "phase-flip-raced" "claim replaced between pre-flip check and mv; holder pid '$after_pid' member '$after_member'"
    exit 6
  fi
}

acquire() {  # atomic create via noclobber; failure output recorded, never discarded
  ( set -o noclobber; printf 'seat=%s\nmember_id=%s\npid=%s\nstarted_at=%s\nphase=draining\n' \
      "$SEAT_NAME" "$MEMBER_ID" "$$" "$STARTED_AT" > "$CLAIM" ) 2>>"$CLAIMLOG"
}

# ---- PHASE 0: load + census before anything ----
uptime | tee "$EVIDENCE_DIR/load-before.txt"

# ---- WRAPPED-NESS PROOF: dialect-tolerant reader exercised before the claim is touched ----
# An unwrapped run cannot produce this record. Both dialects are read from throwaway
# fixtures; canon r3's blind `sed -n 's/^pid=//p'` is shown failing on the colon form.
DIALECT_FIXTURE="$(mktemp -t ablative-dialect-probe)"
{
  echo "a5_delta_reference=a621f353 (rider 77b2c212)"
  printf 'seat=probe\nmember_id=probe-member\npid=424242\nphase=draining\n' > "$DIALECT_FIXTURE"
  echo "equals_form_read_by_wrapper=$(claim_field pid "$DIALECT_FIXTURE")"
  echo "equals_form_read_by_canon_sed=$(sed -n 's/^pid=//p' "$DIALECT_FIXTURE")"
  printf 'seat: probe\nmember_id: probe-member\npid: 515151\nphase: draining\n' > "$DIALECT_FIXTURE"
  echo "colon_form_read_by_wrapper=$(claim_field pid "$DIALECT_FIXTURE")"
  echo "colon_form_read_by_canon_sed=$(sed -n 's/^pid=//p' "$DIALECT_FIXTURE")  <-- EMPTY under canon: the defect"
  echo "we_write_dialect=key=value (A5-1, unchanged from canon)"
  echo "observed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee "$EVIDENCE_DIR/dialect-preflight.txt"
rm -f "$DIALECT_FIXTURE"

# ---- PHASE 1: acquire the claim (hold, never race; ceiling 60 min; timeout = LOUD exit 4) ----
n=0
until acquire; do
  if [ -f "$CLAIM" ]; then
    holder="$(cat "$CLAIM" 2>>"$CLAIMLOG")" || holder="(claim vanished mid-read)"
    hpid="$(claim_field pid "$CLAIM")"   # A5-2: dialect-tolerant, replaces r3's blind sed
    # ---- A5-3 DELTA: unparseable/empty pid fails toward HELD (a621f353 rule 3) ----
    # r3 gates its rule-5 clear on `[ -n "$hpid" ]` alone. Here the pid must be non-empty
    # AND all-digits before any liveness result is believed. When the instrument cannot
    # determine the answer, assume the answer that prevents harm: HELD, never stale.
    if [ -z "$hpid" ] || [ -n "${hpid//[0-9]/}" ]; then
      log "CANNOT DETERMINE HOLDER PID (read '$hpid' — empty or non-numeric): reading as HELD, NOT stale. No rule-5 clear. Waiting (sample $n)"
    elif ! pid_alive "$hpid"; then   # flag 2 applied at the call site: ps -p, never kill -0
      # Rule 5 floor: record verbatim, clear, proceed — loudly, never silently.
      printf '%s\n' "$holder" | tee "$EVIDENCE_DIR/stale-claim-record.txt" >> "$CLAIMLOG"
      log "STALE CLAIM (holder pid $hpid dead) recorded above — OPERATOR MUST post its verbatim contents to the launcher's lane with this run's evidence; clearing and proceeding"
      rm -f "$CLAIM"
      continue
    else
      log "claim held by: ${holder//$'\n'/ } — waiting (sample $n)"
    fi
  fi
  n=$((n+1))
  if [ "$n" -ge 120 ]; then
    log "ACQUISITION TIMEOUT after 60 minutes — refusing loudly"
    exit 4
  fi
  sleep 30
done
CLAIM_ACQUIRED=1   # from here on, a foreign OR absent claim at release is VOIDING (A5-5)
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

# ---- PHASE 3: beamr's five gates.json legs, VERBATIM AND IN ORDER ----
# Per leg: stderr kept in a committed log, exit captured via PIPESTATUS, nothing masked,
# no `|| true`. Load is disclosed per leg via uptime.

uptime | tee "$EVIDENCE_DIR/load-leg1.txt"
cargo fmt --all --check \
  2>>"$EVIDENCE_DIR/fmt.stderr.log" |
  tee "$EVIDENCE_DIR/fmt.log"
FMT_EXIT="${PIPESTATUS[0]}"
log "leg 1 fmt exit=$FMT_EXIT"

uptime | tee "$EVIDENCE_DIR/load-leg2.txt"
cargo clippy --workspace --all-targets -- -D warnings \
  2>>"$EVIDENCE_DIR/clippy.stderr.log" |
  tee "$EVIDENCE_DIR/clippy.log"
CLIPPY_EXIT="${PIPESTATUS[0]}"
log "leg 2 clippy exit=$CLIPPY_EXIT"

uptime | tee "$EVIDENCE_DIR/load-leg3.txt"
cargo check -p beamr-wasm --target wasm32-unknown-unknown --locked \
  2>>"$EVIDENCE_DIR/wasm32-check.stderr.log" |
  tee "$EVIDENCE_DIR/wasm32-check.log"
WASM32_CHECK_EXIT="${PIPESTATUS[0]}"
log "leg 3 wasm32-check exit=$WASM32_CHECK_EXIT"

uptime | tee "$EVIDENCE_DIR/load-leg4.txt"
( wasm-bindgen-test-runner --version \
    && CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
       cargo test --manifest-path crates/beamr-wasm/Cargo.toml --target wasm32-unknown-unknown --locked ) \
  2>>"$EVIDENCE_DIR/wasm-tests.stderr.log" |
  tee "$EVIDENCE_DIR/wasm-tests.log"
WASM_TESTS_EXIT="${PIPESTATUS[0]}"
log "leg 4 wasm-tests exit=$WASM_TESTS_EXIT"

uptime | tee "$EVIDENCE_DIR/load-leg5.txt"
cargo test --workspace \
  2>>"$EVIDENCE_DIR/tests.stderr.log" |
  tee "$EVIDENCE_DIR/tests.log"
TESTS_EXIT="${PIPESTATUS[0]}"
log "leg 5 tests exit=$TESTS_EXIT"

# Coverage statements: true for THIS repo's battery, not copied from another runner.
echo "NEXTEST: N/A — beamr has no nextest stage in gates.json. Declared N/A, not silently dropped." \
  | tee "$EVIDENCE_DIR/nextest-na.txt"
{
  echo "DOC TESTS: COVERED by leg 5 (cargo test --workspace runs doc tests for library targets)."
  echo "Proof — Doc-tests sections found in leg 5's log:"
  grep -E '^[[:space:]]*Doc-tests' "$EVIDENCE_DIR/tests.log" "$EVIDENCE_DIR/tests.stderr.log" 2>/dev/null \
    || echo "(no Doc-tests section matched — INVESTIGATE, do not assume coverage)"
} | tee "$EVIDENCE_DIR/doc-tests-coverage.txt"

# ---- PHASE 4: closing evidence + marker ----
census > "$EVIDENCE_DIR/census-at-end.txt"
uptime | tee "$EVIDENCE_DIR/load-after.txt"
git status --porcelain | tee "$EVIDENCE_DIR/tree-status-at-battery.txt"
FINISHED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
{
  echo "battery=COMPLETE"
  echo "repo=beamr"
  echo "machine=Dean's box"
  echo "operator=$SEAT_NAME"
  echo "seat=$SEAT_NAME"
  echo "member_id=$MEMBER_ID"
  echo "pid=$$"
  echo "started_at=$STARTED_AT"
  echo "finished_at=$FINISHED_AT"
  echo "head_sha=$(git rev-parse HEAD)"
  echo "tree_sha=$(git rev-parse HEAD^{tree})"
  echo "fmt_exit=$FMT_EXIT"
  echo "clippy_exit=$CLIPPY_EXIT"
  echo "wasm32_check_exit=$WASM32_CHECK_EXIT"
  echo "wasm_tests_exit=$WASM_TESTS_EXIT"
  echo "tests_exit=$TESTS_EXIT"
  echo "nextest_exit=N/A (beamr has no nextest stage)"
  echo "database=NONE REQUIRED, NONE ASSUMED (beamr needs no database)"
  echo "void_reason=${VOID_REASON:-none}"
} | tee "$EVIDENCE_DIR/COMPLETE.marker"
log "battery complete; exits fmt=$FMT_EXIT clippy=$CLIPPY_EXIT wasm32-check=$WASM32_CHECK_EXIT wasm-tests=$WASM_TESTS_EXIT tests=$TESTS_EXIT"
exit 0
