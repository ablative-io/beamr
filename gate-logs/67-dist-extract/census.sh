#!/bin/bash
# MECHANICAL CENSUS — lane #67 DIST-EXTRACT.
#
# The claim this instrument COUNTS (it is not a prose claim):
#
#   full-diff 227267e..HEAD  ==  SUM(byte-intact block moves)
#                              + SUM(enumerated edits)
#   remainder EMPTY
#
# Its own exit code is the verdict:
#   rc 0  -> every block diffed clean, BOTH coverage partitions are exact
#            (no gap, no overlap), the two deictic consumers changed exactly
#            the enumerated lines, and no other file moved.
#   rc 1  -> a remainder exists; every REMAINDER line in the log names it.
#
# Inputs, checked in beside this script:
#   BLOCKS.txt  id  file  OLDa OLDb  NEWa NEWb  transform
#               (transform: `verbatim`, or `dedent4` = exactly four leading
#                spaces stripped, the forced consequence of lifting the
#                `mod tests { ... }` body to file top level)
#   EDITS.txt   category \t side \t file \t line \t text
#               category E-a | E-b | E-c | E-f  (see README.md)
#               side OLD = a BASE connection.rs line consumed by an edit
#               side NEW = a landed line not produced by any block move
#
# Outputs: block-NN.diff + block-NN.rc per block; this log; census.rc.
#
# Portability: macOS ships bash 3.2 (no associative arrays), so every set
# operation here is file-based (sort/comm), which is also what makes the
# remainder literally a file you can look at.
#
# Run from the worktree root:  bash gate-logs/67-dist-extract/census.sh
set -uo pipefail

BASE=227267e1b9d69ab78a3bd4dee935497f5e2421dd
SRC=crates/beamr/src/distribution/connection.rs
DIR=crates/beamr/src/distribution/connection
EV=gate-logs/67-dist-extract
BASE_LINES=3449
NEWFILES="dial.rs frame.rs lifecycle.rs link.rs mod.rs residency.rs tests.rs"

FAIL=0
note() { echo "$*"; }
fail() { echo "REMAINDER: $*"; FAIL=1; }

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

git show "$BASE:$SRC" > "$TMP/base.rs"
n=$(wc -l < "$TMP/base.rs" | tr -d ' ')
note "BASE $BASE:$SRC = $n lines"
[ "$n" = "$BASE_LINES" ] || fail "base $SRC is $n lines, expected $BASE_LINES"

############################################################
# PART A — per-block byte-intactness
############################################################
note ""
note "== PART A: per-block byte-intact verification =="
blocks=0
clean=0
: > "$TMP/old_cov.txt"
for f in $NEWFILES; do : > "$TMP/newcov.$f"; done

while read -r id file olda oldb newa newb transform; do
  case "$id" in ''|'#'*) continue ;; esac
  blocks=$((blocks + 1))
  sed -n "${olda},${oldb}p" "$TMP/base.rs" > "$TMP/old.txt"
  if [ "$transform" = "dedent4" ]; then
    sed 's/^    //' "$TMP/old.txt" > "$TMP/old.d" && mv "$TMP/old.d" "$TMP/old.txt"
  fi
  sed -n "${newa},${newb}p" "$DIR/$file" > "$TMP/new.txt"
  diff "$TMP/old.txt" "$TMP/new.txt" > "$EV/block-$id.diff" 2>&1
  rc=$?
  echo "$rc" > "$EV/block-$id.rc"
  if [ "$rc" = "0" ]; then
    clean=$((clean + 1))
  else
    fail "block-$id ($file OLD $olda-$oldb -> NEW $newa-$newb, $transform) rc=$rc"
  fi
  seq "$olda" "$oldb" >> "$TMP/old_cov.txt"
  seq "$newa" "$newb" >> "$TMP/newcov.$file"
done < "$EV/BLOCKS.txt"

note "blocks declared: $blocks"
note "block diffs rc 0: $clean"
[ "$blocks" = "$clean" ] || fail "$((blocks - clean)) block(s) are not byte-intact"

############################################################
# PART B — OLD-side coverage partition over BASE 1..3449
############################################################
note ""
note "== PART B: OLD-side coverage (base $SRC, 1..$BASE_LINES) =="
sort -n "$TMP/old_cov.txt" > "$TMP/old_cov.s"
if [ -n "$(sort -n "$TMP/old_cov.txt" | uniq -d)" ]; then
  fail "base lines claimed by more than one block: $(sort -n "$TMP/old_cov.txt" | uniq -d | tr '\n' ' ')"
fi
awk -F'\t' '$2=="OLD" && $3=="connection.rs" {print $4}' "$EV/EDITS.txt" | sort -n > "$TMP/old_ed.s"
if [ -n "$(uniq -d "$TMP/old_ed.s")" ]; then
  fail "base lines listed twice as OLD-side edits: $(uniq -d "$TMP/old_ed.s" | tr '\n' ' ')"
fi
note "block-covered base lines: $(wc -l < "$TMP/old_cov.s" | tr -d ' ')"
note "enumerated OLD-side edit lines: $(wc -l < "$TMP/old_ed.s" | tr -d ' ')"

both=$(comm -12 "$TMP/old_cov.s" "$TMP/old_ed.s")
[ -n "$both" ] && fail "base lines in BOTH a block and an edit: $(echo "$both" | tr '\n' ' ')"

sort -n -u "$TMP/old_cov.s" "$TMP/old_ed.s" > "$TMP/old_acct.s"
seq 1 "$BASE_LINES" | sort -n > "$TMP/old_all.s"
missing=$(comm -23 "$TMP/old_all.s" "$TMP/old_acct.s")
extra=$(comm -13 "$TMP/old_all.s" "$TMP/old_acct.s")
[ -n "$missing" ] && fail "base lines accounted by NOTHING: $(echo "$missing" | tr '\n' ' ')"
[ -n "$extra" ] && fail "accounted lines outside 1..$BASE_LINES: $(echo "$extra" | tr '\n' ' ')"
[ -z "$missing$extra$both" ] && note "OLD-side partition EXACT: blocks + edits = 1..$BASE_LINES, no gap, no overlap"

############################################################
# PART C — NEW-side coverage partition over every landed file
############################################################
note ""
note "== PART C: NEW-side coverage ($DIR/) =="
for f in $NEWFILES; do
  len=$(wc -l < "$DIR/$f" | tr -d ' ')
  sort -n "$TMP/newcov.$f" > "$TMP/nc.s"
  if [ -n "$(uniq -d "$TMP/nc.s")" ]; then
    fail "$f lines claimed by more than one block: $(uniq -d "$TMP/nc.s" | tr '\n' ' ')"
  fi
  awk -F'\t' -v f="$f" '$2=="NEW" && $3==f {print $4}' "$EV/EDITS.txt" | sort -n > "$TMP/ne.s"
  both=$(comm -12 "$TMP/nc.s" "$TMP/ne.s")
  [ -n "$both" ] && fail "$f lines in BOTH a block and an edit: $(echo "$both" | tr '\n' ' ')"
  sort -n -u "$TMP/nc.s" "$TMP/ne.s" > "$TMP/na.s"
  seq 1 "$len" | sort -n > "$TMP/nall.s"
  missing=$(comm -23 "$TMP/nall.s" "$TMP/na.s")
  extra=$(comm -13 "$TMP/nall.s" "$TMP/na.s")
  note "  $f: $len lines = $(wc -l < "$TMP/nc.s" | tr -d ' ') block-moved + $(wc -l < "$TMP/ne.s" | tr -d ' ') enumerated"
  [ -n "$missing" ] && fail "$f lines accounted by NOTHING: $(echo "$missing" | tr '\n' ' ')"
  [ -n "$extra" ] && fail "$f accounted lines past EOF: $(echo "$extra" | tr '\n' ' ')"
done

############################################################
# PART D — the two deictic consumers
############################################################
note ""
note "== PART D: deictic consumers (etf.rs, sender.rs) =="
for f in etf.rs sender.rs; do
  p="crates/beamr/src/distribution/$f"
  git diff --no-ext-diff -U0 "$BASE" -- "$p" | grep -E '^-[^-]|^\+[^+]' | sort > "$TMP/cgot.s"
  awk -F'\t' -v f="$f" '$1=="E-c" && $3==f {print ($2=="OLD" ? "-" : "+") $5}' "$EV/EDITS.txt" | sort > "$TMP/cexp.s"
  note "  $f: $(wc -l < "$TMP/cgot.s" | tr -d ' ') changed lines in the working diff, $(wc -l < "$TMP/cexp.s" | tr -d ' ') enumerated"
  if ! diff "$TMP/cexp.s" "$TMP/cgot.s" > "$TMP/cres.txt"; then
    fail "$f working diff is not exactly the enumerated E-c set:"
    cat "$TMP/cres.txt"
  fi
done

############################################################
# PART E — nothing else moved
############################################################
note ""
note "== PART E: no other path changed vs $BASE =="
git diff --no-ext-diff --name-status "$BASE" -- . > "$TMP/names.txt"
cat "$TMP/names.txt"
unexpected=$(grep -v -E "	($SRC$|$DIR/|crates/beamr/src/distribution/(etf|sender)\.rs$|$EV/)" "$TMP/names.txt")
if [ -n "$unexpected" ]; then
  fail "paths changed outside the declared set:"
  echo "$unexpected"
else
  note "  confined to: $SRC (deleted), $DIR/, etf.rs, sender.rs, $EV/"
fi

############################################################
note ""
if [ "$FAIL" = "0" ]; then
  note "CENSUS: REMAINDER EMPTY"
  note "  full-diff $BASE..HEAD"
  note "    = $blocks byte-intact block moves (all rc 0)"
  note "    + $(grep -c . "$EV/EDITS.txt") enumerated edit lines"
  note "    + nothing else"
else
  note "CENSUS: REMAINDER NON-EMPTY — see the REMAINDER lines above"
fi
exit "$FAIL"
