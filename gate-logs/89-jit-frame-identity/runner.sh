#!/bin/bash
# beamr #88 JIT-SEND-DELIVERY release battery — six canon legs read FROM the
# committed gates.json at run time (so the battery cannot drift by transcription).
#
# Inherits every repair from the 0.17.0 run-2 runner (gate-logs/77-release-0170/
# runner-repaired.sh): interpreter exec-probe, denominator non-empty+numeric+
# stderr-clean, artifacts asserted before scoring, COMPLETE.marker DERIVED from
# .rc files ON DISK carrying both numbers.
#
# NEW THIS RUN — the AMENDED TREE CHECK (three artifacts, not one):
#   tree.dirty.raw       `git status --porcelain` verbatim, nothing filtered
#   tree.dirty.filtered  raw MINUS the one known-untracked path (.claude/skills/)
#   tree.dirty.rc        the COUNT of filtered lines, written explicitly
# The count is written with wc, never with `grep -c`: grep exits 1 on a zero
# count, so a `|| fail` on the pipeline would turn a true clean tree into
# "producer failed". The raw file is kept so the filter can be audited — a
# filtered artifact alone cannot show what it removed.
# No producer-side silence redirection anywhere. No 2>/dev/null.
set -u

PY="${BATTERY_PY:-/usr/bin/python3}"
WT="/Users/tom/Developer/ablative/stack/beamr"
EV="${BATTERY_EV:-/private/tmp/claude-501/-Users-tom-Developer-ablative-stack-beamr/b337ce2b-336a-4856-a9d8-54c90496c9fa/scratchpad/battery-88}"
mkdir -p "$EV"
cd "$WT" || exit 90

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
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
printf 'commit=%s\ntree=%s\nbranch=%s\n' "$COMMIT" "$TREE" "$BRANCH" > "$EV/pin.txt"

# --- the amended tree check ---
git status --porcelain > "$EV/tree.dirty.raw"
grep -v '^?? \.claude/skills/$' "$EV/tree.dirty.raw" > "$EV/tree.dirty.filtered"
dirty="$(wc -l < "$EV/tree.dirty.filtered" | tr -d ' ')"
printf '%s\n' "$dirty" > "$EV/tree.dirty.rc"
printf 'filter=drop one line, exactly "?? .claude/skills/" (untracked, never staged)\n' \
  > "$EV/tree.dirty.note"
if [ "$dirty" -ne 0 ]; then
  printf 'TREE DIRTY: %s line(s) survive the filter — the bytes that run are not the bytes committed.\n' \
    "$dirty" > "$EV/ABORT.txt"
  cat "$EV/tree.dirty.filtered" >> "$EV/ABORT.txt"
  exit 88
fi

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

N="$("$PY" -c "import json;print(len(json.load(open('gates.json'))['legs']))" 2> "$EV/denominator.err")"
nrc=$?
printf 'legs_declared=%s\n' "$N" > "$EV/denominator.txt"
printf 'denominator_rc=%s\n' "$nrc" >> "$EV/denominator.txt"
if [ "$nrc" -ne 0 ]; then
  printf 'DENOMINATOR COMMAND FAILED rc=%s\n' "$nrc" > "$EV/ABORT.txt"; exit 94
fi
if [ -z "$N" ]; then
  printf 'DENOMINATOR EMPTY — this is exactly how the 0.17.0 run 1 went void.\n' > "$EV/ABORT.txt"; exit 95
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
  if [ ! -f "$EV/leg-$i-$name.log" ]; then
    printf 'leg %s (%s) PRODUCED NO LOG — rc %s describes nothing\n' "$i" "$name" "$legrc" >> "$EV/MISSING-ARTIFACT.txt"
    printf '99\n' > "$EV/leg-$i.rc"
    continue
  fi
  printf '%s\n' "$legrc" > "$EV/leg-$i.rc"
done

# The lane walls — the delivery property this version number claims.
boundary "pre-lane-walls-send"
cargo test -p beamr --lib jit_send > "$EV/walls-jit-send.log" 2>&1
printf '%s\n' "$?" > "$EV/walls-jit-send.rc"
boundary "pre-lane-gate-integration"
cargo test -p beamr --test jit_send_delivery_gate > "$EV/walls-integration.log" 2>&1
printf '%s\n' "$?" > "$EV/walls-integration.rc"

# §5 carry from the 0.17.0 run: the one thing CI uniquely covered that no leg does.
boundary "pre-extra-cooperative-json"
cargo check -p beamr --no-default-features --features cooperative,json > "$EV/extra-cooperative-json.log" 2>&1
printf '%s\n' "$?" > "$EV/extra-cooperative-json.rc"

boundary "post-battery"
du -sk "$WT/target" > "$EV/du-final.txt"
df -k /System/Volumes/Data > "$EV/df-final.txt"

# the tree is re-checked AFTER the run: a leg that writes into the worktree
# would otherwise ship bytes the pin never covered.
git status --porcelain > "$EV/tree.dirty.raw.post"
grep -v '^?? \.claude/skills/$' "$EV/tree.dirty.raw.post" > "$EV/tree.dirty.filtered.post"
wc -l < "$EV/tree.dirty.filtered.post" | tr -d ' ' > "$EV/tree.dirty.rc.post"

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
