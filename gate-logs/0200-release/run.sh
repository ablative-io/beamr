#!/usr/bin/env bash
# beamr 0.20.0 release battery — nine legs VERBATIM from gates.json.
#
# Discipline:
#  - leg commands come verbatim from gates.json; no flags added to the gate of record
#  - NEVER take $? from the far end of a pipe: out="$(cmd 2>&1)"; rc=$? is the form
#  - no 2>/dev/null anywhere; stderr is captured to a file, never suppressed
#  - JSON legs keep stdout and stderr SEPARATE (message-format=json puts machine
#    events on stdout and human diagnostics on stderr; merging them destroys both)
set -u

W="$(cd "$(dirname "$0")/../wt-rel" && pwd)"
OUT="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$OUT/logs"
PROG="$OUT/PROGRESS.txt"
TSV="$OUT/rc-table.tsv"

{
  echo "battery start : $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "worktree      : $W"
  echo "commit        : $(git -C "$W" rev-parse HEAD)"
  echo "tree          : $(git -C "$W" rev-parse HEAD^{tree})"
  echo "dirty         : $(git -C "$W" status --porcelain | wc -l | tr -d ' ')"
  echo "rustc         : $(rustc --version)"
  echo "cargo         : $(cargo --version)"
  echo "---"
} > "$PROG"

printf 'leg\trc\tseconds\n' > "$TSV"

n=$(jq -r '.legs | length' "$W/gates.json")
for i in $(seq 0 $((n-1))); do
  name=$(jq -r ".legs[$i].name" "$W/gates.json")
  cmd=$(jq -r ".legs[$i].command // .legs[$i].cmd" "$W/gates.json")

  echo "[$(date -u +%H:%M:%SZ)] START $name" >> "$PROG"
  start=$(date +%s)

  # stdout and stderr to SEPARATE files, both kept.
  ( cd "$W" && eval "$cmd" ) > "$OUT/logs/$name.out" 2> "$OUT/logs/$name.err"
  rc=$?

  end=$(date +%s)
  secs=$((end-start))
  echo "$rc" > "$OUT/logs/$name.rc"
  printf '%s\t%s\t%s\n' "$name" "$rc" "$secs" >> "$TSV"
  echo "[$(date -u +%H:%M:%SZ)] DONE  $name rc=$rc ${secs}s" >> "$PROG"
done

echo "---" >> "$PROG"
echo "battery end   : $(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$PROG"
echo "COMPLETE" >> "$PROG"
