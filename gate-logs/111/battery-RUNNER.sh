#!/bin/bash
#
# The canon battery runner. Legs are read from gates.json AT RUN TIME so this
# script cannot drift from canon by transcription (#75).
#
# ── WHICH ARTEFACTS ARE KEPT, AND WHY (ruled #116, measured not preferred) ────
#
# COMMIT, every lane:  BATTERY.log · <pin>.tsv · <pin>.leg4/5/8.log
# BIN at lane close:   <pin>.leg1/2/3/6/7/9.log
#
# ⚠️ THE RULE IS "COMMIT THE LEGS THAT CARRY AXES", NOT "COMMIT LEGS 4, 5 AND 8".
# Leg numbers are POSITIONS IN gates.json and they move when a leg is added.
# Legs 4/5/8 are the cargo-test legs; every other leg is a check whose only
# verdict is its rc, and the rc is already in the .tsv. #219 added leg 9
# (nostd-ratchet, a check) -- hence the 9 above. Re-derive this list from
# `kind` when the leg set changes; do not transcribe it.
#
# The .tsv records the rc of ALL EIGHT legs, so binning five of the logs loses
# no verdict -- only the transcript of legs that carry no axes. Verified before
# the first binning: every one of the five affected pins had a committed .tsv
# with 8 rows.
#
# ⛔ THE REASON IS NOT SIZE, IT IS CONTENT. Legs 2 and 7 are clippy under
# --message-format=json, i.e. machine-event streams full of ABSOLUTE OPERATOR
# PATHS (/Users/<name>/Developer/..., /Users/<name>/.cargo/registry/...). They
# are ~235 KB each and 99% of a lane's log volume; legs 1/3/6 are ~200 bytes
# each and only ride along because the rule is simpler than an exception list.
# Committing them bakes one machine's environment into the repo permanently and
# makes the artefacts non-reproducible at another seat.
#
# ⛔ AND THE FIX IS NOT .gitignore. Hiding files from `git status` to make the
# tree-count read clean would corrupt the very instrument the count belongs to.
# They are deleted, so there is nothing to hide.
#
# ⚠️ BIN AT LANE CLOSE, IMMEDIATELY AFTER COMMITTING THE KEPT ARTEFACTS. The
# residue is invisible until then: `git status --porcelain` collapses a wholly
# untracked directory to ONE line, so a fresh battery dir reads as 1 no matter
# what is in it. The moment you commit leg4/5/8 the directory becomes tracked
# and its five siblings start listing individually -- which is why the count
# grew by exactly five per lane (21 at #114, 26 at #115) and looked like drift.
#
# `tree pre` / `tree post` assert PRE == POST WITHIN A RUN. That is all they
# assert. The absolute number was never comparable across lanes while residue
# accumulated; with the residue binned it is, and should be kept that way.
#
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
  ( eval "$CMD" ) > "$OUT.leg$((i+1)).log" 2>&1
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
