#!/bin/zsh
# #74 SETH-REPLAY — 6-leg canon at the lane tip.
# Reference form: every leg's log by REDIRECT (never tee), rc into leg-N.rc,
# a boundary df read before every leg echoing what it read against THRESHOLD.txt.
# No producer-side silence redirection anywhere: no 2>/dev/null, no || true, no -q.
# Leg commands are VERBATIM from gates.json.

set -u

EV=/private/tmp/claude-501/-Users-tom-Developer-ablative-stack-beamr/b337ce2b-336a-4856-a9d8-54c90496c9fa/scratchpad/EV-74
WT=/private/tmp/claude-501/-Users-tom-Developer-ablative-stack-beamr/b337ce2b-336a-4856-a9d8-54c90496c9fa/scratchpad/wt-74

cd "$WT" || exit 20
THRESH=$(head -1 "$EV/THRESHOLD.txt")

git rev-parse HEAD > "$EV/GREEN-tree.sha"
echo "battery tree: $(cat "$EV/GREEN-tree.sha")"
echo "threshold: $(cat "$EV/THRESHOLD.txt")"

run_leg() {
  n=$1
  name=$2
  cmd=$3

  avail=$(df -k /System/Volumes/Data | awk 'NR==2{printf "%.2f", $4/1048576}')
  echo "$avail" > "$EV/boundary-$n.df"
  echo "LEG $n ($name) BOUNDARY: read ${avail} GiB available (df -k /System/Volumes/Data, avail/1048576) vs THRESHOLD ${THRESH}"
  under=$(awk -v a="$avail" -v t="$THRESH" 'BEGIN{print (a < t) ? 1 : 0}')
  if [ "$under" -eq 1 ]; then
    echo "LEG $n ($name) HALT: boundary ${avail} is UNDER threshold ${THRESH} — battery stops here, no leg run"
    echo "halted-under-threshold" > "$EV/leg-$n.rc"
    exit 9
  fi

  eval "$cmd" > "$EV/leg-$n.log" 2>&1
  rc=$?
  echo "$rc" > "$EV/leg-$n.rc"
  echo "LEG $n ($name) rc=$rc  log=$EV/leg-$n.log"
}

# KNOWN-ANSWER CONTROL — can a leg's RECORDED rc go non-zero through run_leg?
# run_leg above is byte-identical to the battery's (proven by extraction diff).
# Pre-chosen answers: leg c1 MUST record 7, leg c2 MUST record 0.
run_leg c1 control-must-record-7 'sh -c "exit 7"'
run_leg c2 control-must-record-0 'sh -c "exit 0"'
echo "CONTROL-COMPLETE-MARKER 74"
