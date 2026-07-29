#!/bin/zsh
# EXIT-001 mutation-run harness — Diana Plum (…c9fb), Annabel's box.
# Usage: run-mutation.zsh <mutation-name> <out-file-basename> <reps> <filter...>
# Applies the diff, runs the filtered walls, captures exit per rep, reverts.
# Preflight per CLAIM CONVENTION v2 (entry 4b8b38e1, ablative/stack): ordinary
# compiles yield to a live battery claim — claim live => HOLD (exit 42), never race.
set -u
WT=/Users/annabel/Developer/ablative/stack/beamr/.wt-exit001-7a1242eb
EV=$WT/docs/design/beamr/briefs/evidence/exit001-7a1242eb
CLAIM=/tmp/ablative-gate-battery.claim
MUT=$1; OUTBASE=$2; REPS=$3; shift 3

if [[ -e $CLAIM ]]; then
  echo "HOLD: battery claim live — not launching. Claim body:"
  cat "$CLAIM"
  exit 42
fi

cd "$WT" || exit 3
if [[ -n "$(git status --porcelain | grep -v '^??')" ]]; then
  echo "TREE NOT CLEAN before apply — refusing"; git status --porcelain; exit 3
fi
git apply "$EV/mutations/$MUT.diff" || { echo "APPLY FAILED"; exit 3; }

OUT=$EV/runs/$OUTBASE
{
  echo "# EXIT-001 mutation run — task 7a1242eb-742b-48be-ba7e-d961f2c49f04"
  echo "# machine: Annabel's box; operator: Diana Plum (b337ce2b-…-c9fb)"
  echo "# date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# tree: $(git rev-parse --short HEAD) + $MUT.diff APPLIED (working tree only, never committed)"
  echo "# preflight: /tmp/ablative-gate-battery.claim ABSENT at launch (claim convention v2, entry 4b8b38e1)"
  echo "# command: cargo test -p beamr --lib -- ${*} (reps: $REPS)"
  echo
} > "$OUT"

OVERALL=0
for i in $(seq 1 "$REPS"); do
  if [[ -e $CLAIM ]]; then
    echo "## rep $i SKIPPED — battery claim appeared mid-sequence; holding" >> "$OUT"
    OVERALL=42
    break
  fi
  cargo test -p beamr --lib -- "$@" >> "$OUT" 2>&1
  RC=$?
  echo "## rep $i exit=$RC" >> "$OUT"
  [[ $RC -ne 0 ]] && OVERALL=$RC
done

git checkout -- crates/
if [[ -n "$(git status --porcelain | grep -v '^??')" ]]; then
  echo "WARNING: tree not clean after revert:"; git status --porcelain
else
  echo "tree clean after revert"
fi
echo "overall=$OVERALL out=$OUT"
tail -n 3 "$OUT"
exit 0
