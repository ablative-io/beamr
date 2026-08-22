#!/usr/bin/env bash
# accessor-diff-p2.sh — the three measurements the battery CANNOT make.
#
# WHY THIS EXISTS. Phase 1 runs gates.json verbatim, which is right for a gate
# but leaves two holes in a DIFFERENTIAL:
#
#   HOLE 1 — TRUNCATION. The `tests` and `tests-all-features` legs carry no
#   --no-fail-fast, so cargo stops at the FIRST failing test target. A red set
#   compared under truncation is only equal "up to the first failure". A genuine
#   land-only failure sitting behind that first failure is invisible, and the
#   invisibility looks exactly like cleanliness. P2A/P2B rerun both test legs
#   with --no-fail-fast to get the COMPLETE failing-test multiset.
#
#   HOLE 2 — LOAD ASYMMETRY. Phase 1 runs base first and land second. Another
#   fleet run is live on this box, so the two arms may have met different load.
#   One of the five reds under test is a THREAD-INVENTORY test — precisely the
#   kind that contention perturbs. If base met a busy box and land met a quiet
#   one, a difference in that test would be an artefact of scheduling order, and
#   I would report the venue as a finding about the code. P2C interleaves the
#   arms A/B/A/B so any load trend is shared by both, not aligned with one.
#
# These are additions to the record, NOT substitutes for the gate. The gate's
# own verdict is whatever phase 1 measured.
set -u

ROOT=/home/rocketfish/fleet/artemis-diff
LOGS="$ROOT/logs"
BASE_SHA=c55ac360eb573148ef868ce7eb7d03babf05523a
LAND_SHA=e1cb306052aa8519c49fc5941940614cd131ed45
PROGRESS="$ROOT/PROGRESS-P2.txt"

note() { echo "[$(date -u +%H:%M:%SZ)] $*" | tee -a "$PROGRESS"; }
: > "$PROGRESS"

note "=== INSTRUMENT: accessor-diff-p2.sh ==="
note "utc_open=$(date -u +%Y-%m-%dT%H:%M:%SZ) host=$(hostname)"

assert_arm() {
  local arm="$1"
  local want="$2"
  local got
  got="$(git -C "$ROOT/$arm" rev-parse HEAD)"
  if [ "$got" != "$want" ]; then
    note "FATAL: arm $arm at $got, expected $want — ABORT"
    return 1
  fi
  return 0
}

assert_arm base "$BASE_SHA" || exit 90
assert_arm land "$LAND_SHA" || exit 91

# ------------------------------------------------- P2A/P2B: untruncated tests --
full_tests() {
  local arm="$1"
  local tag="$2"
  local featflag="$3"
  local dir="$ROOT/$arm"
  local rc t0 t1
  note "  [$arm/$tag] START load='$(cat /proc/loadavg)'"
  t0=$(date +%s)
  ( cd "$dir" && eval "cargo test --workspace $featflag --no-fail-fast" ) \
      > "$LOGS/$arm.$tag.log" 2>&1
  rc=$?
  t1=$(date +%s)
  note "  [$arm/$tag] rc=$rc in $((t1-t0))s  load='$(cat /proc/loadavg)'"
  echo "$arm	$tag	$rc	$((t1-t0))" >> "$LOGS/rc-table-p2.tsv"
}

note "########## P2A — tests, --no-fail-fast, both arms ##########"
full_tests base p2a-tests-nff "--features beamr/encode"
full_tests land p2a-tests-nff "--features beamr/encode"

note "########## P2B — tests-all-features, --no-fail-fast, both arms ##########"
full_tests base p2b-testsall-nff "--all-features"
full_tests land p2b-testsall-nff "--all-features"

# ------------------------------------ P2C: interleaved A/B on the sensitive test --
# 5 alternating pairs. Alternation is the whole point: it makes any load drift a
# COMMON-MODE term instead of a per-arm one.
note "########## P2C — interleaved A/B, thread_inventory_distribution x5 ##########"
: > "$LOGS/p2c-interleaved.tsv"
for round in 1 2 3 4 5; do
  for arm in base land; do
    dir="$ROOT/$arm"
    load_at="$(awk '{print $1}' /proc/loadavg)"
    ( cd "$dir" && cargo test -p beamr --test thread_inventory_distribution ) \
        > "$LOGS/$arm.p2c.round$round.log" 2>&1
    rc=$?
    verdict=FAIL
    if [ "$rc" -eq 0 ]; then verdict=PASS; fi
    echo "$round	$arm	$rc	$verdict	$load_at" >> "$LOGS/p2c-interleaved.tsv"
    note "  [p2c r$round $arm] $verdict (rc=$rc, load1=$load_at)"
  done
done

note "=== P2 COMPLETE $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
note "--- p2 rc table ---"
cat "$LOGS/rc-table-p2.tsv" | tee -a "$PROGRESS"
note "--- p2c interleaved (round arm rc verdict load1) ---"
cat "$LOGS/p2c-interleaved.tsv" | tee -a "$PROGRESS"
note "DONE"
