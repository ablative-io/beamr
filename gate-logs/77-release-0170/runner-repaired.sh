#!/bin/bash
# beamr 0.17.0 release battery — RUN 2. Run 1 was VOID: it scored zero legs and
# wrote COMPLETE.marker anyway.
#
# Five repairs over run 1:
#   R1  the interpreter is EXEC-PROBED, not looked up. `command -v` attests
#       resolvability, never executability — run 1's python3 resolved fine and
#       was a Linux ELF that cannot exec on this box (rc 126).
#   R2  the denominator must be non-empty AND numeric, and its stderr must be
#       EMPTY. A working binary that prints a warning passes a numeric check
#       and is still the wrong instrument.
#   R3  the denominator command's stderr is CAPTURED to a file. Run 1 threw it
#       away, which is why the diagnosis took a second pass.
#   R4  every artifact is asserted to EXIST before it is scored.
#   R5  COMPLETE.marker is DERIVED, not constant: it is written only when
#       legs_scored == legs_declared, and it carries BOTH numbers. legs_scored
#       is counted from the .rc files ON DISK, not from the loop variable, so
#       a loop that never entered cannot report its own success.
# No producer-side silence redirection anywhere. Per-leg log by REDIRECT.
set -u

# PY and EV are overridable ONLY so the abort paths can be exercised as
# controls (see CONTROLS.md beside this script). The defaults are the run.
PY="${BATTERY_PY:-/usr/bin/python3}"
WT="/Users/tom/Developer/ablative/stack/beamr"
EV="${BATTERY_EV:-/private/tmp/claude-501/-Users-tom-Developer-ablative-stack-beamr/b337ce2b-336a-4856-a9d8-54c90496c9fa/scratchpad/battery-0170-r2}"
mkdir -p "$EV"
cd "$WT" || exit 90

# --- R1: exec-probe the interpreter before trusting it with the denominator ---
"$PY" --version > "$EV/interpreter.out" 2> "$EV/interpreter.err"
pyrc=$?
printf '%s\n' "$pyrc" > "$EV/interpreter.rc"
printf 'path=%s\n' "$PY" >> "$EV/interpreter.out"
if [ "$pyrc" -ne 0 ]; then
  printf 'INTERPRETER FAILED TO EXEC: %s rc=%s — refusing to run a battery I cannot count.\n' \
    "$PY" "$pyrc" > "$EV/ABORT.txt"
  exit 93
fi

COMMIT="$(git rev-parse HEAD)"
TREE="$(git rev-parse HEAD^{tree})"
printf 'commit=%s\ntree=%s\n' "$COMMIT" "$TREE" > "$EV/pin.txt"
git status --porcelain > "$EV/tree-state.txt"
printf '25000000\n' > "$EV/THRESHOLD.txt"   # KiB floor (25 GiB), band with no live lanes

boundary () {  # $1 = label; fails LOUDLY if under threshold
  local free thr
  free="$(df -k /System/Volumes/Data | tail -1 | awk '{print $4}')"
  thr="$(cat "$EV/THRESHOLD.txt")"
  printf '%s free=%s threshold=%s\n' "$1" "$free" "$thr" >> "$EV/boundary-df.txt"
  if [ "$free" -lt "$thr" ]; then
    printf 'BOUNDARY VIOLATION at %s: free=%s < threshold=%s\n' "$1" "$free" "$thr" >> "$EV/boundary-df.txt"
    return 1
  fi
  return 0
}

# --- R2 + R3: the denominator comes from gates.json, with stderr captured ---
N="$("$PY" -c "import json;print(len(json.load(open('gates.json'))['legs']))" 2> "$EV/denominator.err")"
nrc=$?
printf 'legs_declared=%s\n' "$N" > "$EV/denominator.txt"
printf 'denominator_rc=%s\n' "$nrc" >> "$EV/denominator.txt"
if [ "$nrc" -ne 0 ]; then
  printf 'DENOMINATOR COMMAND FAILED rc=%s\n' "$nrc" > "$EV/ABORT.txt"; exit 94
fi
if [ -z "$N" ]; then
  printf 'DENOMINATOR EMPTY — this is exactly how run 1 went void.\n' > "$EV/ABORT.txt"; exit 95
fi
case "$N" in
  ''|*[!0-9]*) printf 'DENOMINATOR NOT NUMERIC: %s\n' "$N" > "$EV/ABORT.txt"; exit 96 ;;
esac
if [ "$N" -lt 1 ]; then
  printf 'DENOMINATOR IS ZERO — a battery with no legs is not a green.\n' > "$EV/ABORT.txt"; exit 97
fi
if [ -s "$EV/denominator.err" ]; then
  printf 'DENOMINATOR WROTE TO STDERR — right number, possibly wrong instrument:\n' > "$EV/ABORT.txt"
  cat "$EV/denominator.err" >> "$EV/ABORT.txt"; exit 98
fi

i=0
while [ "$i" -lt "$N" ]; do
  idx=$i
  i=$((i+1))
  name="$("$PY" -c "import json;print(json.load(open('gates.json'))['legs'][$idx].get('name',''))" 2>> "$EV/extract.err")"
  cmd="$("$PY" -c "import json;print(json.load(open('gates.json'))['legs'][$idx].get('cmd',''))" 2>> "$EV/extract.err")"
  if [ -z "$name" ]; then
    printf 'leg %s HAS NO NAME — extraction is broken, not the leg\n' "$i" >> "$EV/EXTRACT-FAILURE.txt"
    printf '90\n' > "$EV/leg-$i.rc"
    continue
  fi
  if [ -z "$cmd" ]; then
    printf 'leg %s (%s) HAS EMPTY COMMAND — refusing to score it\n' "$i" "$name" >> "$EV/EMPTY-COMMAND.txt"
    printf '91\n' > "$EV/leg-$i.rc"
    continue
  fi
  printf '%s\n' "$name" > "$EV/leg-$i.name"
  printf '%s\n' "$cmd" > "$EV/leg-$i.cmd"
  if ! boundary "pre-leg-$i-$name"; then printf '92\n' > "$EV/leg-$i.rc"; continue; fi
  bash -c "$cmd" > "$EV/leg-$i-$name.log" 2>&1
  legrc=$?
  # R4: the log must exist before the rc is believed to be about anything.
  if [ ! -f "$EV/leg-$i-$name.log" ]; then
    printf 'leg %s (%s) PRODUCED NO LOG — rc %s describes nothing\n' "$i" "$name" "$legrc" >> "$EV/MISSING-ARTIFACT.txt"
    printf '99\n' > "$EV/leg-$i.rc"
    continue
  fi
  printf '%s\n' "$legrc" > "$EV/leg-$i.rc"
done

# §5: the one thing CI uniquely covered that no battery leg does.
boundary "pre-extra-cooperative-json"
cargo check -p beamr --no-default-features --features cooperative,json > "$EV/extra-cooperative-json.log" 2>&1
printf '%s\n' "$?" > "$EV/extra-cooperative-json.rc"

# The lane walls — the memory-safety property the version number claims.
boundary "pre-lane-walls"
cargo test -p beamr --lib gc_rooting > "$EV/walls-gc-rooting.log" 2>&1
printf '%s\n' "$?" > "$EV/walls-gc-rooting.rc"
boundary "pre-lane-walls-jit"
cargo test -p beamr --lib runtime_binary_match > "$EV/walls-binary-match.log" 2>&1
printf '%s\n' "$?" > "$EV/walls-binary-match.rc"

boundary "post-battery"
du -sk "$WT/target" > "$EV/du-final.txt"

# --- R5: the marker is DERIVED. legs_scored is counted from disk. ---
scored=0
for f in "$EV"/leg-*.rc; do
  if [ -f "$f" ]; then scored=$((scored+1)); fi
done
printf 'legs_declared=%s\nlegs_scored=%s\n' "$N" "$scored" > "$EV/tally.txt"
if [ "$scored" -eq "$N" ]; then
  printf 'COMPLETE legs_declared=%s legs_scored=%s commit=%s\n' "$N" "$scored" "$COMMIT" > "$EV/COMPLETE.marker"
  exit 0
else
  printf 'VOID: legs_declared=%s legs_scored=%s commit=%s — this run CANNOT say whether the tree is green.\n' \
    "$N" "$scored" "$COMMIT" > "$EV/VOID.marker"
  exit 89
fi
