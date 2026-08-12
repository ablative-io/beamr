#!/usr/bin/env bash
# beamr 0.17.1 release battery.
#
# Legs are READ FROM the committed gates.json at run time, never transcribed —
# a transcribed leg can drift from the gate it claims to be.
# Per-leg rc is recorded as its own artifact; stderr is captured, NEVER
# suppressed, because a suppressed stderr turns "I could not measure" into
# "I measured nothing".
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

: > "$HERE/legs.tsv"

# --- tree check, PRE (three-artifact form; counted with wc, never grep -c) ---
git status --porcelain > "$HERE/tree.pre.raw"
grep -v -e '^?? \.claude/skills/' -e '^?? gate-logs/0171-release/' "$HERE/tree.pre.raw" > "$HERE/tree.pre.filtered"
wc -l < "$HERE/tree.pre.filtered" | tr -d ' ' > "$HERE/tree.pre.count"

git rev-parse HEAD > "$HERE/pin.commit"
git rev-parse HEAD^{tree} > "$HERE/pin.tree"

LEGS_DECLARED="$(/usr/bin/python3 -c "import json;print(len(json.load(open('gates.json'))['legs']))")"
# A leg count that did not parse must ABORT, never fall through to a zero-trip
# loop: an empty loop scores nothing and reports every leg green.
case "$LEGS_DECLARED" in
  ''|*[!0-9]*) echo "ABORT: leg count did not parse from gates.json (got '$LEGS_DECLARED')" >&2; exit 2 ;;
esac
[ "$LEGS_DECLARED" -ge 1 ] || { echo "ABORT: gates.json declares $LEGS_DECLARED legs" >&2; exit 2; }
echo "$LEGS_DECLARED" > "$HERE/legs.declared"

SCORED=0
for i in $(seq 0 $((LEGS_DECLARED - 1))); do
  NAME="$(/usr/bin/python3 -c "import json;print(json.load(open('gates.json'))['legs'][$i]['name'])")"
  CMD="$(/usr/bin/python3 -c "import json;print(json.load(open('gates.json'))['legs'][$i]['cmd'])")"
  echo "=== leg $((i+1))/$LEGS_DECLARED : $NAME ===" >&2
  bash -c "$CMD" > "$HERE/leg-$((i+1))-$NAME.log" 2>&1
  RC=$?
  echo "$RC" > "$HERE/leg-$((i+1))-$NAME.rc"
  printf '%s\t%s\t%s\n' "$((i+1))" "$NAME" "$RC" >> "$HERE/legs.tsv"
  SCORED=$((SCORED + 1))
done

echo "$SCORED" > "$HERE/legs.scored"

# --- tree check, POST ---
git status --porcelain > "$HERE/tree.post.raw"
grep -v -e '^?? \.claude/skills/' -e '^?? gate-logs/0171-release/' "$HERE/tree.post.raw" > "$HERE/tree.post.filtered"
wc -l < "$HERE/tree.post.filtered" | tr -d ' ' > "$HERE/tree.post.count"

# --- axes, parsed from the tests leg rather than matched loosely ---
TESTLOG="$HERE/leg-5-tests.log"
if [ ! -f "$TESTLOG" ]; then
  echo "ABORT: tests leg log absent — refusing to report axes from a run that did not produce one" >&2
  exit 3
fi
grep -h '^test result:' "$TESTLOG" > "$HERE/axes.resultlines" || true
wc -l < "$HERE/axes.resultlines" | tr -d ' ' > "$HERE/axes.resultlines.count"
awk '/^test result:/ {for (i=1;i<=NF;i++) {
  if ($i=="passed;") p+=$(i-1);
  if ($i=="failed;") f+=$(i-1);
  if ($i=="ignored;") g+=$(i-1);
}} END {printf "passed=%d failed=%d ignored=%d\n", p, f, g}' "$TESTLOG" > "$HERE/axes.totals"

# --- COMPLETE marker is DERIVED, never asserted ---
if [ "$(cat "$HERE/legs.declared")" = "$(cat "$HERE/legs.scored")" ]; then
  echo "COMPLETE legs_declared=$(cat "$HERE/legs.declared") legs_scored=$(cat "$HERE/legs.scored")" > "$HERE/marker"
else
  echo "INCOMPLETE legs_declared=$(cat "$HERE/legs.declared") legs_scored=$(cat "$HERE/legs.scored")" > "$HERE/marker"
fi

echo "--- legs.tsv ---"; cat "$HERE/legs.tsv"
echo "--- marker ---"; cat "$HERE/marker"
echo "--- axes ---"; cat "$HERE/axes.totals"; echo "result-lines: $(cat "$HERE/axes.resultlines.count")"
echo "--- tree pre/post ---"; cat "$HERE/tree.pre.count"; cat "$HERE/tree.post.count"
