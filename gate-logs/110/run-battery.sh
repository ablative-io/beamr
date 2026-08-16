#!/usr/bin/env bash
# AR-1 row 4 — the canon battery, read FROM gates.json at run time so this
# runner cannot drift from the canon by transcription.
#
# Writes <dir>/declared.count, <dir>/<leg>.rc and <dir>/<leg>.log, which is the
# contract scripts/ci-verdict.sh grades. ⛔ NEVER `2>/dev/null` here: a leg's
# stderr is part of its evidence, and the blocking-call gate's positive control
# prints an expected "N error(s) found" notice to stderr on a PASSING run.
set -uo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 2
DIR="${1:-gate-logs/110/battery}"
mkdir -p "$DIR"

n=$(jq '.legs | length' gates.json)
printf '%s\n' "$n" > "$DIR/declared.count"
echo "declared legs: $n"

for i in $(seq 0 $((n - 1))); do
  name=$(jq -r ".legs[$i].name" gates.json)
  cmd=$(jq -r ".legs[$i].cmd" gates.json)
  echo "=== leg $((i + 1))/$n  $name"
  echo "    $cmd"
  bash -c "$cmd" > "$DIR/${name}.log" 2>&1
  rc=$?
  printf '%s\n' "$rc" > "$DIR/${name}.rc"
  echo "    rc=$rc"
done

echo
echo "=== VERDICT"
bash scripts/ci-verdict.sh "$DIR" gates.json
echo "verdict rc=$?"
