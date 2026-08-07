#!/bin/bash
# beamr 0.17.0 release battery — legs read FROM gates.json at run time (no transcription).
# Reference form: per-leg log by REDIRECT (never tee), rc into per-leg .rc files,
# boundary-df before EVERY leg. No producer-side silence redirection anywhere.
set -u

WT="/Users/tom/Developer/ablative/stack/beamr"
EV="/private/tmp/claude-501/-Users-tom-Developer-ablative-stack-beamr/b337ce2b-336a-4856-a9d8-54c90496c9fa/scratchpad/battery-0170"
mkdir -p "$EV"
cd "$WT" || exit 90

COMMIT="$(git rev-parse HEAD)"
TREE="$(git rev-parse HEAD^{tree})"
printf 'commit=%s\ntree=%s\n' "$COMMIT" "$TREE" > "$EV/pin.txt"
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

# Enumerate legs from gates.json itself — the denominator comes from the file.
N="$(python3 -c "import json;print(len(json.load(open('gates.json'))['legs']))")"
printf 'legs_declared=%s\n' "$N" > "$EV/denominator.txt"

i=0
while [ "$i" -lt "$N" ]; do
  idx=$i
  i=$((i+1))
  name="$(python3 -c "import json;print(json.load(open('gates.json'))['legs'][$idx].get('name',''))")"
  cmd="$(python3 -c "import json;print(json.load(open('gates.json'))['legs'][$idx].get('cmd',''))")"
  if [ -z "$cmd" ]; then
    printf 'leg %s (%s) HAS EMPTY COMMAND — refusing to score it\n' "$i" "$name" >> "$EV/EMPTY-COMMAND.txt"
    printf '91\n' > "$EV/leg-$i.rc"
    continue
  fi
  printf '%s\n' "$cmd" > "$EV/leg-$i.cmd"
  if ! boundary "pre-leg-$i-$name"; then printf '92\n' > "$EV/leg-$i.rc"; continue; fi
  bash -c "$cmd" > "$EV/leg-$i-$name.log" 2>&1
  printf '%s\n' "$?" > "$EV/leg-$i.rc"
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
printf 'DONE\n' > "$EV/COMPLETE.marker"
