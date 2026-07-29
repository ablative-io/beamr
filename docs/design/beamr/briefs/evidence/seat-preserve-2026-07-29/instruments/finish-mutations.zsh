#!/bin/zsh
set -u
WT=/Users/annabel/Developer/ablative/stack/beamr/.wt-exit001-7a1242eb
EV=$WT/docs/design/beamr/briefs/evidence/exit001-7a1242eb
CLAIM=/tmp/ablative-gate-battery.claim
SP=/private/tmp/claude-501/-Users-annabel-Developer-ablative-stack-beamr/b337ce2b-336a-4856-a9d8-54c90496c9fa/scratchpad
cd "$WT" || exit 3
pf() { if [[ -e $CLAIM ]]; then echo "HOLD before $1:"; cat "$CLAIM"; exit 42; fi }

# A) 10x isolated greens for the fixed walls + wall1b (clean tree, committed 8f3bf57)
OUT=$EV/runs/green-postfix-walls-10x.txt
{
  echo "# EXIT-001 post-fix 10x isolated greens — task 7a1242eb"
  echo "# machine: Annabel's box; operator: Diana Plum (b337ce2b-…-c9fb)"
  echo "# date: $(date -u +%Y-%m-%dT%H:%M:%SZ); tree: $(git rev-parse --short HEAD), clean"
} > "$OUT"
for f in wall1b_ publication_order w2_watch_registered; do
  echo "=== filter: $f — 10x isolated ===" >> "$OUT"
  for i in {1..10}; do
    pf "green-10x $f rep $i"
    cargo test -p beamr --lib -- "$f" >> "$OUT" 2>&1
    echo "## $f rep $i exit=$?" >> "$OUT"
  done
done
G=$(grep -c "exit=0" "$OUT")
echo "green-10x: $G/30 reps exit 0"
[[ $G -ne 30 ]] && { echo "NOT ALL GREEN — stopping"; exit 5; }

# B) Mutation 8: m-check-then-register — prediction b: NO reds
pf "mutation-8"
zsh "$SP/run-mutation.zsh" m-check-then-register obs-m-check-then-register-full.txt 1 wall1 publication_order w2_watch_registered w3_many w4_registry w5_exclusive watch_ characterization || exit $?
pf "mutation-8b"
zsh "$SP/run-mutation.zsh" m-check-then-register obs-m-check-then-register-w2-10x.txt 10 w2_watch_registered || exit $?

# C) Mutation 9: m-skeleton-both-reverted — W2 red (lost-wake)
pf "mutation-9"
zsh "$SP/run-mutation.zsh" m-skeleton-both-reverted red-m-skeleton-both-reverted-w2.txt 1 w2_watch_registered || exit $?

echo "ALL DONE"
git status --porcelain | grep -v '^??' | wc -l | xargs echo "tracked-modified:"
