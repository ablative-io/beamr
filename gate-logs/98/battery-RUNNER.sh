#!/bin/bash
set -u
REPO=/Users/tom/Developer/ablative/stack/beamr
OUT="$1"
cd "$REPO" || exit 99
PIN=$(git rev-parse HEAD)
echo "pin:      $PIN"
echo "tree pre: $(git status --porcelain -- . ':!.claude' | wc -l | tr -d ' ') modified"
echo "opened:   $(date -u +%Y-%m-%dT%H:%M:%SZ)"
DECLARED=$(/usr/bin/python3 -c "import json;print(len(json.load(open('gates.json'))['legs']))")
echo "legs declared: $DECLARED"
SCORED=0
: > "$OUT.tsv"
for i in $(seq 0 $((DECLARED-1))); do
  NAME=$(/usr/bin/python3 -c "import json;print(json.load(open('gates.json'))['legs'][$i]['name'])")
  CMD=$(/usr/bin/python3 -c "import json;print(json.load(open('gates.json'))['legs'][$i]['cmd'])")
  if [ -z "$CMD" ]; then echo "=== leg $((i+1))/$DECLARED : $NAME === EMPTY COMMAND — ABORT"; exit 98; fi
  echo "=== leg $((i+1))/$DECLARED : $NAME ==="
  ( eval "$CMD" ) >/dev/null 2>&1
  RC=$?
  echo "    rc=$RC"
  printf '%s\t%s\t%s\n' "$((i+1))" "$NAME" "$RC" >> "$OUT.tsv"
  SCORED=$((SCORED+1))
done
echo "--- scored: $SCORED / declared: $DECLARED ---"
echo "--- tree post: $(git status --porcelain -- . ':!.claude' | wc -l | tr -d ' ') ---"
POST=$(git rev-parse HEAD)
echo "--- pin post:  $POST ---"
if [ "$SCORED" -eq "$DECLARED" ] && [ "$PIN" = "$POST" ]; then echo "COMPLETE (derived: $SCORED/$DECLARED, pin stable)"; else echo "INCOMPLETE"; fi
echo "CLOSE $(date -u +%Y-%m-%dT%H:%M:%SZ)"
