#!/usr/bin/env bash
# accessor-diff.sh — INDEPENDENT differential re-measure of the beamr
# accessor-lifetimes landing, leg for leg.
#
# The claim under test (relayed from the flight, NOT inherited as fact):
#   "the red set at e1cb3060 equals main's leg-for-leg — all five reds pre-existing."
#
# This instrument does not try to confirm that claim. It measures both arms with
# the same battery and emits per-leg ERROR MULTISETS WITH COORDINATES, because the
# claim is about multisets, not counts. A count match with different coordinates
# is a DIFFERENT red set wearing the same number.
#
# Discipline (each line here is a law that has cost me a false finding before):
#   - the leg commands are READ FROM gates.json AT RUNTIME and run verbatim.
#     Nothing is restated. If gates.json differs between arms, that is itself a
#     finding and each arm runs its own.
#   - no 2>/dev/null anywhere. A suppressed stderr is a suppressed measurement.
#   - rc is never taken from the far end of a pipe. Redirect to files, then read $?.
#   - every arm asserts its own HEAD before it measures. A leg run at the wrong
#     tree is worthless and looks identical to a leg run at the right one.
#   - concurrent load is witnessed per leg, because this box has ANOTHER FLEET RUN
#     live on it and a thread-inventory test is exactly what contention perturbs.
#   - the venue names its own drift (wasm-bindgen was downgraded under us today).
set -u

ROOT=/home/rocketfish/fleet/artemis-diff
REPO=/home/rocketfish/Developer/ablative/stack/beamr
BASE_SHA=c55ac360eb573148ef868ce7eb7d03babf05523a
LAND_SHA=e1cb306052aa8519c49fc5941940614cd131ed45
LOGS="$ROOT/logs"
PROGRESS="$ROOT/PROGRESS.txt"

note() { echo "[$(date -u +%H:%M:%SZ)] $*" | tee -a "$PROGRESS"; }

mkdir -p "$LOGS"
: > "$PROGRESS"

note "=== INSTRUMENT: accessor-diff.sh ==="
note "host=$(hostname) user=$(whoami) utc_open=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
note "uname=$(uname -srm) nproc=$(nproc)"

# ---------------------------------------------------------------- P0: venue ---
{
  echo "=== VENUE at open ==="
  echo "utc            : $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "rustc(home)    : $(rustc --version)"
  echo "cargo(home)    : $(cargo --version)"
  echo "wasm-bindgen   : $(wasm-bindgen --version)"
  echo "wb-test-runner : $(wasm-bindgen-test-runner --version)"
  echo "node           : $(node --version)"
  echo "jq             : $(jq --version)"
  echo "ast-grep       : $(ast-grep --version)"
  echo "--- VENUE DRIFT NOTE (measured 2026-08-22) ---"
  echo "wasm-bindgen-cli was REWRITTEN at 09:00:37 +1000 today and .crates.toml"
  echo "now records 0.2.123. The 0.2.127 reading recorded for OB-004 on 2026-08-20"
  echo "was true when taken; the venue has since drifted BACK to the skew that"
  echo "obligation existed to retire. Both arms below see the same 0.2.123, so the"
  echo "DIFFERENTIAL stays well-posed; only the ABSOLUTE wasm-tests result is skewed."
  ls -la --time-style=full-iso /home/rocketfish/.cargo/bin/ | grep wasm
  grep wasm /home/rocketfish/.cargo/.crates.toml
  echo "--- CONCURRENT WORK ON THIS BOX (not mine, NOT to be touched) ---"
  ps -eo pid=,ppid=,etimes=,args= | grep -E "cargo|rustc" | grep -v grep
  echo "--- load/mem/psi ---"
  cat /proc/loadavg
  grep -E "^(MemTotal|MemAvailable|SwapTotal|SwapFree):" /proc/meminfo
  cat /proc/pressure/cpu
  cat /proc/pressure/io
  df -h /home | tail -1
} > "$LOGS/00-venue.txt" 2>&1
note "venue recorded -> logs/00-venue.txt"

# ------------------------------------------------------- P0b: the worktrees ---
setup_arm() {
  # NOT `local a="$1" d="$ROOT/$a"` — bash expands every word of the `local`
  # command BEFORE it assigns any of them, so the second would read an unbound
  # $a and die under `set -u`. Measured: it did, on the first launch.
  local arm="$1"
  local ref="$2"
  local want="$3"
  local dir="$ROOT/$arm"
  if [ -d "$dir/.git" ] || [ -f "$dir/.git" ]; then
    note "arm $arm: worktree already present, reusing"
  else
    note "arm $arm: creating fresh worktree at $dir from $ref"
    git -C "$REPO" worktree add --detach "$dir" "$ref" >> "$LOGS/00-worktrees.txt" 2>&1
    local rc=$?
    if [ "$rc" -ne 0 ]; then note "FATAL: worktree add failed rc=$rc for $arm"; return 1; fi
  fi
  local got
  got="$(git -C "$dir" rev-parse HEAD)"
  if [ "$got" != "$want" ]; then
    note "FATAL: arm $arm is at $got, expected $want — ABORT (wrong tree = worthless leg)"
    return 1
  fi
  note "arm $arm: HEAD=$got VERIFIED, dirty=$(git -C "$dir" status --porcelain | wc -l | tr -d ' ')"
  return 0
}

setup_arm base bundle-base "$BASE_SHA" || exit 90
setup_arm land bundle-land "$LAND_SHA" || exit 91

# gates.json identity across arms — a differential is only well-posed if the
# battery DEFINITION is the same on both sides. If it is not, say so loudly.
BASE_G="$(sha256sum "$ROOT/base/gates.json" | awk '{print $1}')"
LAND_G="$(sha256sum "$ROOT/land/gates.json" | awk '{print $1}')"
{
  echo "gates.json sha256 base : $BASE_G"
  echo "gates.json sha256 land : $LAND_G"
  if [ "$BASE_G" = "$LAND_G" ]; then
    echo "VERDICT: IDENTICAL — the differential is well-posed, same battery both arms."
  else
    echo "VERDICT: ***DIFFERENT*** — each arm runs its OWN legs and this is a finding."
  fi
} > "$LOGS/00-gates-identity.txt" 2>&1
note "gates.json identity: base=$BASE_G land=$LAND_G"

# ------------------------------------------------------------ P1: the legs ---
run_arm() {
  local arm="$1"
  local want="$2"
  local dir="$ROOT/$arm"
  note "########## ARM $arm — battery opening ##########"

  local head_pre
  head_pre="$(git -C "$dir" rev-parse HEAD)"
  if [ "$head_pre" != "$want" ]; then
    note "FATAL: arm $arm drifted to $head_pre before battery — ABORT"; return 1
  fi

  # leg names, in gates.json order, read from THIS ARM's own tree
  local legs
  legs="$(python3 -c "import json,sys;print(' '.join(l['name'] for l in json.load(open('$dir/gates.json'))['legs']))")"
  note "arm $arm legs: $legs"

  local leg
  for leg in $legs; do
    local cmd fmt
    cmd="$(python3 -c "import json;print([l['cmd'] for l in json.load(open('$dir/gates.json'))['legs'] if l['name']=='$leg'][0])")"
    fmt="$(python3 -c "import json;print([l.get('format','text') for l in json.load(open('$dir/gates.json'))['legs'] if l['name']=='$leg'][0])")"
    if [ -z "$cmd" ]; then note "FATAL: empty leg command for $leg — ABORT"; return 1; fi

    local stamp_open load_open rc t0 t1
    stamp_open="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    load_open="$(cat /proc/loadavg)"
    t0=$(date +%s)
    note "  [$arm/$leg] START $stamp_open  load='$load_open'"

    {
      echo "=== LEG $leg  ARM $arm ==="
      echo "tree     : $dir"
      echo "head     : $(git -C "$dir" rev-parse HEAD)"
      echo "cmd      : $cmd"
      echo "started  : $stamp_open"
      echo "load_open: $load_open"
      echo "concurrent cargo/rustc at open:"
      ps -eo pid=,etimes=,args= | grep -E "cargo|rustc" | grep -v grep | sed 's/^/    /'
    } > "$LOGS/$arm.$leg.meta.txt" 2>&1

    if [ "$fmt" = "json" ]; then
      # message-format=json: machine events on STDOUT, human diagnostics on STDERR.
      # They must not be merged or the extract cannot parse.
      ( cd "$dir" && eval "$cmd" ) > "$LOGS/$arm.$leg.json" 2> "$LOGS/$arm.$leg.err"
      rc=$?
    else
      ( cd "$dir" && eval "$cmd" ) > "$LOGS/$arm.$leg.log" 2>&1
      rc=$?
    fi
    t1=$(date +%s)

    {
      echo "rc        : $rc"
      echo "finished  : $(date -u +%Y-%m-%dT%H:%M:%SZ)"
      echo "seconds   : $((t1-t0))"
      echo "load_close: $(cat /proc/loadavg)"
      echo "head_post : $(git -C "$dir" rev-parse HEAD)"
      echo "dirty_post: $(git -C "$dir" status --porcelain | wc -l | tr -d ' ')"
    } >> "$LOGS/$arm.$leg.meta.txt" 2>&1

    echo "$arm	$leg	$rc	$((t1-t0))" >> "$LOGS/rc-table.tsv"
    note "  [$arm/$leg] rc=$rc in $((t1-t0))s"
  done

  local head_post
  head_post="$(git -C "$dir" rev-parse HEAD)"
  note "arm $arm CLOSED. head_pre=$head_pre head_post=$head_post dirty=$(git -C "$dir" status --porcelain | wc -l | tr -d ' ')"
  return 0
}

# Arms run SEQUENTIALLY on purpose: running them concurrently would have each arm
# contending with the other for the same 16 cores, and one of the five reds under
# test is a THREAD-INVENTORY test. Self-contention would manufacture the very
# failure I am trying to attribute.
run_arm base "$BASE_SHA"
note "base arm returned $?"
run_arm land "$LAND_SHA"
note "land arm returned $?"

note "=== BATTERY COMPLETE — $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
note "rc table:"
cat "$LOGS/rc-table.tsv" | tee -a "$PROGRESS"
note "DONE"
