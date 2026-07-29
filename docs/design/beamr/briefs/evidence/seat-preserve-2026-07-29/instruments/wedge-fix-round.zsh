#!/bin/zsh
# EXIT-001 wedge-fix compile round — Diana Plum (…c9fb), Annabel's box.
# Preflight per claim convention v2; reverts mutation with git apply -R ONLY
# (tree carries uncommitted wall fixes — checkout would destroy them).
set -u
WT=/Users/annabel/Developer/ablative/stack/beamr/.wt-exit001-7a1242eb
EV=$WT/docs/design/beamr/briefs/evidence/exit001-7a1242eb
CLAIM=/tmp/ablative-gate-battery.claim
SP=/private/tmp/claude-501/-Users-annabel-Developer-ablative-stack-beamr/b337ce2b-336a-4856-a9d8-54c90496c9fa/scratchpad
cd "$WT" || exit 3

pf() { if [[ -e $CLAIM ]]; then echo "HOLD: claim live before $1:"; cat "$CLAIM"; exit 42; fi }

# Step 1: clean-tree green verification (walls fixed + wall1b + neighbours)
pf "clean-run"
cargo test -p beamr --lib -- wall1 publication_order w2_watch_registered w3_many w4_registry w5_exclusive watch_ characterization > "$SP/clean-green-verify.txt" 2>&1
G1=$?
echo "clean-tree run exit=$G1"
grep -E "test result" "$SP/clean-green-verify.txt" | tail -1
if [[ $G1 -ne 0 ]]; then echo "CLEAN TREE NOT GREEN — stopping before mutation"; exit 5; fi

# Step 2: mutation red-proof for the FIXED walls + wall1b (bounded wall-clock)
pf "mutation-run"
git apply "$EV/mutations/m-wall1-publish-before-install.diff" || exit 3
OUT=$EV/runs/red-m-wall1-fixed-walls-clean-red.txt
{
  echo "# EXIT-001 post-wedge-fix red proof — task 7a1242eb"
  echo "# machine: Annabel's box; operator: Diana Plum (b337ce2b-…-c9fb)"
  echo "# date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# tree: $(git rev-parse --short HEAD) + uncommitted wedge-law wall fixes + wall1b + m-wall1-publish-before-install.diff APPLIED (working tree only)"
  echo "# preflight: claim ABSENT at launch"
  echo "# purpose: the wedge ruling's evidence demand (1) — the FIXED publication_order wall and wall1b"
  echo "#          must red CLEANLY (terminate, exit 101, bounded wall-clock) under the same mutation"
  echo "#          whose kill previously manifested as a wedge (see wedge-…-discovery.txt)."
  echo "# start: $(date -u +%H:%M:%S)Z"
} > "$OUT"
cargo test -p beamr --lib -- wall1b_ publication_order >> "$OUT" 2>&1
RC=$?
echo "# end: $(date -u +%H:%M:%S)Z — bounded wall-clock, run TERMINATED on its own" >> "$OUT"
echo "## exit=$RC" >> "$OUT"
git apply -R "$EV/mutations/m-wall1-publish-before-install.diff" || { echo "REVERT FAILED — STOP"; exit 4; }
echo "mutation red-proof exit=$RC (expect 101)"
grep -E "test .*(ok|FAILED)|test result" "$OUT" | tail -6
echo "tracked-modified after revert (expect only the 3 known):"
git status --porcelain | grep -v '^??'
