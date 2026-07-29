#!/bin/bash
# Canonical gate battery — claim-integrated amended form, revision 2 (Seth's flags 1-5 folded).
# Requires: SEAT_NAME, MEMBER_ID, EVIDENCE_DIR (committed gate-logs dir). Refuses to run without them.
# LAUNCH CONTRACT (flag 4): the launch line is load-bearing — launch with bash, from the repo root,
# with every env var the dispatching brief names (e.g. a test-database URL). The script enforces
# what it can below; the dispatcher's brief MUST name the rest and the operator's evidence MUST echo it.
set -u

# ================= INSTANTIATION (beamr / EXIT-001, Annabel's box) =================
# r3-derived runner: everything below this block is BYTE-VERBATIM from canon r3,
# stack-root entry e269d2c9-0dfa-409e-ada5-b10303118225 (sha256 with-LF
# ff831516f8ff74de1f54023ff4af54d80cade6a285b6ff42f5a37dbfda6290e4 / 11532B,
# stripped 99e140b60aedcaf166526e975349acc3a56302c00d51fb24ae3a9c4e04bcab3a /
# 11531B — both verified at this seat before copying; extracted
# programmatically from the entry, never retyped), through the end of PHASE 2,
# except the census extension (+wasm-bindgen-test-runner) disclosed in the
# emitted header. Phases 3-4 are beamr's, per re-brief 2 as carried into
# re-brief 3. Supersession chain: 42c6231 (six-pin v2.zsh) -> 421778f
# (622dedbf-derived) -> this file (r3). Predecessors never launch.
SEAT_NAME="Diana Plum"
MEMBER_ID="b337ce2b-336a-4856-a9d8-54c90496c9fa"
WT=/Users/annabel/Developer/ablative/stack/beamr/.wt-exit001-7a1242eb
EVIDENCE_DIR="$WT/docs/design/beamr/briefs/evidence/exit001-7a1242eb/battery"
cd "$WT" || exit 2
# ===================================================================================

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
  for p in cargo rustc cargo-nextest cargo-clippy clippy-driver rustdoc wasm-bindgen-test-runner; do
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
  # Claim double-record, part 2 (dispatch bef7f4f2 addition 1): body verbatim
  # immediately before release, both branches — the pair with
  # claim-body-at-acquire.txt gives the return its match/no-match line.
  if [ -f "$CLAIM" ]; then
    cat "$CLAIM" > "$EVIDENCE_DIR/claim-body-at-release.txt" 2>>"$CLAIMLOG"
  fi
  if claim_is_mine; then
    rm -f "$CLAIM"
    log "claim released (own claim, pid $$)"
  elif [ -f "$CLAIM" ]; then
    log "exit without release: claim on file is NOT ours (holder $(sed -n 's/^pid=//p' "$CLAIM" 2>>"$CLAIMLOG")) — leaving it in place"
  elif [ "${CLAIM_WAS_ACQUIRED:-0}" = "1" ]; then
    # A5 third delta (ruling 4081cb3d): AT RELEASE, AN ABSENT CLAIM IS VOIDING
    # EXACTLY LIKE A FOREIGN ONE. We wrote it, held it, and trapped its release
    # on every exit path — there is no innocent reason it is missing. The
    # thief-finishes-first ordering leaves exactly this state, and a quiet
    # no-op here would publish a green whose quiet-floor premise was violated.
    printf 'ABSENT AT RELEASE (was acquired this run)\n' > "$EVIDENCE_DIR/claim-absent-at-release.txt"
    log "A5 ABSENT-CLAIM DETECTOR (4081cb3d): our claim is MISSING at release — someone took it; quiet-floor premise VOID; RUN VOID AS EVIDENCE; operator posts this to the lane"
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
    # A5 delta (a621f353): dialect-tolerant read — key=value AND key: value both parse.
    hpid="$(printf '%s\n' "$holder" | sed -n -e 's/^pid=[[:space:]]*//p' -e 's/^pid:[[:space:]]*//p' | head -n 1)"
    if [ -z "$hpid" ]; then
      # A5 delta (a621f353): an unparseable or empty pid is NEVER grounds for a
      # rule-5 clear — cannot-determine reads HELD, never stale.
      log "A5 guard: claim pid unparseable/empty — reads HELD, never stale; waiting (sample $n)"
    elif ! pid_alive "$hpid"; then   # flag 2 applied at the call site: ps -p, never kill -0
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
CLAIM_WAS_ACQUIRED=1   # A5 third delta: arms the absent-at-release detector — only a run that
                       # actually held the claim can have it stolen-then-removed.
log "claim acquired, phase=draining"
# Claim double-record, part 1 (dispatch bef7f4f2 addition 1): body verbatim at acquire.
cat "$CLAIM" 2>>"$CLAIMLOG" | tee "$EVIDENCE_DIR/claim-body-at-acquire.txt" >> "$CLAIMLOG"

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
# A5 delta (a621f353): NO PHASE FLIP WITHOUT AN OWNERSHIP CHECK — re-read the
# claim and confirm own member id AND pid before draining->running. A foreign
# body means the claim was REPLACED MID-RUN: the flip is REFUSED loudly, the
# quiet-floor premise is void, and the run is VOID AS EVIDENCE (detector, not
# error); the operator posts the recorded body to the lane. Exit 6.
FLIP_BODY="$(cat "$CLAIM" 2>>"$CLAIMLOG")" || FLIP_BODY=""
flip_pid="$(printf '%s\n' "$FLIP_BODY" | sed -n -e 's/^pid=[[:space:]]*//p' -e 's/^pid:[[:space:]]*//p' | head -n 1)"
flip_member="$(printf '%s\n' "$FLIP_BODY" | sed -n -e 's/^member_id=[[:space:]]*//p' -e 's/^member[-_]id:[[:space:]]*//p' | head -n 1)"
if [ "$flip_pid" != "$$" ] || [ "$flip_member" != "$MEMBER_ID" ]; then
  printf '%s\n' "$FLIP_BODY" | tee "$EVIDENCE_DIR/replaced-claim-at-flip.txt" >> "$CLAIMLOG"
  log "A5 FLIP REFUSED (a621f353): claim on file is not ours at draining->running — REPLACED MID-RUN; quiet-floor premise VOID; RUN VOID AS EVIDENCE; operator posts the body above to the lane"
  exit 6
fi
write_claim running
log "floor quiet at census; phase=running (A5 flip ownership check passed: pid + member id confirmed ours)"
census > "$EVIDENCE_DIR/census-at-start.txt"   # the census, not the claim, is the quiet-floor proof

# ---- battery header: provenance, pin set, toolchain (self-describing evidence) ----
{
  echo "=== battery start (UTC) ==="; date -u
  echo "=== machine/operator ==="; echo "Annabel's box / $SEAT_NAME ($MEMBER_ID)"
  echo "=== tree ==="; git rev-parse HEAD; git status --porcelain | wc -l | xargs echo "dirty-entries:"
  echo "=== toolchain ==="; rustc -vV; cargo --version; wasm-bindgen-test-runner --version
  echo "=== provenance ==="
  echo "preamble: BYTE-VERBATIM from CANON r3, stack-root entry e269d2c9-0dfa-409e-ada5-b10303118225 @ sha256 ff831516f8ff74de1f54023ff4af54d80cade6a285b6ff42f5a37dbfda6290e4 (with-LF, 11532B) / 99e140b60aedcaf166526e975349acc3a56302c00d51fb24ae3a9c4e04bcab3a (stripped, 11531B), both verified at this seat; supersession chain 622dedbf -> 0b229d0f (r2) -> e269d2c9 (r3, current). KNOWN LABEL DISCREPANCY, copied not fixed: r3's own line 2 self-describes as 'revision 2' (Seth, anchor entry c59ba3b1) — label-only, content unambiguous, correction is the anchor owner's"
  echo "disclosed deltas from verbatim: (1) instantiation block = the canon's required SEAT_NAME/MEMBER_ID/EVIDENCE_DIR + per-box absolute worktree, set in-script (pattern ratified, entry edb63b89); (2) census extension: +wasm-bindgen-test-runner (beamr's foreign-runner class — extension, not fork)"
  echo "=== A5 deltas (Amendment A5, entry a621f353 — interim guards in force before r4; r3 bytes stay frozen as identity) ==="
  echo "A5 write dialect: this runner WRITES canon's key=value dialect exactly (inherited byte-verbatim from r3's write_claim/acquire) — stated per A5 item 1, not left inferred"
  echo "A5 delta (i): dialect-tolerant claim READS — holder pid parsed from both key=value and key: value forms (acquisition loop + flip check)"
  echo "A5 delta (ii): unparseable/empty pid is NEVER grounds for a rule-5 clear — reads HELD, never stale (cannot-determine assumes the answer that prevents harm)"
  echo "A5 delta (iii): NO PHASE FLIP WITHOUT OWNERSHIP CHECK — claim re-read at draining->running, own member id + pid confirmed; foreign body => flip REFUSED loudly, exit 6, run VOID AS EVIDENCE (detector, not error)"
  echo "A5 delta (iv) (ruling 4081cb3d): AT RELEASE, AN ABSENT CLAIM IS VOIDING EXACTLY LIKE A FOREIGN ONE — a run that acquired the claim and finds nothing at release was robbed by the thief-finishes-first ordering; detector fires loudly, run VOID AS EVIDENCE"
  echo "=== detector limits, stated as ruled (4081cb3d — venue: Annabel's box, NOT Dean's serial topology) ==="
  echo "(a) the flip guard DETECTS AFTER canon's internal mv rather than refusing before it — true pre-flip refusal awaits r4"
  echo "(b) this box launches runners INDEPENDENTLY — no coordinator serialization backs the premise (Dean's-box detect-and-void venue ruling does NOT transfer)"
  echo "(c) the quiet-floor claim therefore rests on the detectors firing (perpetrator detects at flip, victim at release, BOTH branches of the victim's check built), not on collision prevention"
  echo "=== wrapped-ness, verifiable never assumed (Hermes via Seth; this box has no entry control) ==="
  echo "the A5 deltas bind only runs through THIS wrapper; an unwrapped direct canon launch would produce a green indistinguishable EXCEPT by the bundle. This wrapper emits records an unwrapped run cannot: this three-delta disclosure header citing a621f353, the claim double-record pair (claim-body-at-acquire/-at-release), the flip-time ownership check line in claim.log, and the release-time own/foreign/ABSENT determination line. A bundle missing those did not go through the wrapper — read it, never infer it"
  echo "protocol delta (dispatch bef7f4f2 addition 1): claim body recorded verbatim TWICE — claim-body-at-acquire.txt and claim-body-at-release.txt — for the return's match/no-match line"
  echo "exit vocabulary: 4 = acquisition timeout; 5 = drain timeout; 6 = A5 flip refusal (claim replaced mid-run)"
  echo "legs: beamr's FIVE, VERBATIM from EXIT-001 brief .verification @ 8f2b7c3 (= gates.json; ruling entry 85d5781b). beamr has NO nextest leg: canon amendments A4/A6 are N/A here, stated rather than silently dropped"
  echo "=== launch environment (load-bearing, stated loud — canon LAUNCH CONTRACT) ==="
  echo "launched with bash from the worktree root $WT (canon self-guards: bash refusal + Cargo.toml root check); SEAT_NAME/MEMBER_ID/EVIDENCE_DIR set in-script by the instantiation block (values above); NO database URL or other env is needed for beamr's legs — that absence is stated here rather than silent"
  echo "=== convention pin set (twelve entries, re-brief 3, restated verbatim) ==="
  echo "1. 4b8b38e1-c99c-49ca-a9cb-692fa073e3c1 — anchor"
  echo "2. e903b4ad-209e-4478-a656-6c8d83357ae9 — Amendment 1"
  echo "3. c6d998bc-f9c0-497c-9b85-725ff1e9f195 — Amendment 2"
  echo "4. aa92a18c-59cd-49c0-b606-10ab7037c301 — Addendum (behavioral standard)"
  echo "5. 91ba17f9-e1aa-43c0-b9d6-beaed18538c7 — Seth's record (variant withdrawn)"
  echo "6. c3ee8385-cbd1-463c-8b86-43da2a684547 — ratification (rule-5 floor uniform)"
  echo "7. bdce0b78-ebcb-44b3-bbc0-07780b476f6f — superseding pointer (canon-at-source)"
  echo "8. edb63b89-51d6-43ea-9d9a-914f94d36fe5 — instantiation pattern ratified"
  echo "9. c527324b-9057-49dd-b6c7-d112a3157811 — Seth's five flags"
  echo "10. 4a7bce95-2ba7-4e5e-a5a7-5aae25045225 — Amendment 3 (rule-4 mechanism + r3)"
  echo "11. e269d2c9-0dfa-409e-ada5-b10303118225 — CANON r3, the build source"
  echo "12. 3e5a93ca-234d-4ea2-bb9b-35701f6b86c7 — Amendment 4 (holder-spared kills; slot restitution)"
  echo "13. a621f353-78c4-4453-93fa-309c38bdee98 — Amendment A5 (claim body is pinned bytes; write path guarded)"
  echo "14. 77b2c212 — null-diff rider (a null diff from canon is no longer a conformance claim; disclosed deltas are the correct shape)"
  echo "15. 4081cb3d — absent-claim voiding at release + venue ruling (per-box topology decides)"
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

# A7 ADAPTED, NOT COPIED: the canon's "not covered" line would be FALSE here.
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
