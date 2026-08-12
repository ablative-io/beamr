#!/usr/bin/env bash
# Gate: no blocking construct inside crates/beamr/src/native/.
#
# ⛔ WHY THIS IS A SCRIPT AND NOT A BARE `ast-grep scan`
#
# The bare command's PASSING verdict is "no findings, exit 0". MEASURED, that
# same verdict is returned when:
#
#   * the rule's pattern list is EMPTY          -> `[]`, rc 0
#   * the SCAN PATH DOES NOT EXIST              -> `[]`, rc 0, and the
#     "No such file or directory" notice goes to STDERR while the exit code
#     stays 0, so even capturing stderr does not fail the leg
#
# (A missing rule FILE does exit nonzero — rc 6 — so that one arm was already
# safe. The other two were not.)
#
# beamr has restructured `src/` directories twice recently. A renamed or split
# `native/` would leave this gate scanning nothing and reporting clean forever.
# A zero from an absent check and a zero from a passing check are the same
# number, so the gate must establish that its checker CAN fire before its zero
# means anything.
#
# Three preconditions, each of which RAISES rather than skips:
#   1. the rule fires on a positive-control fixture, once per declared pattern
#   2. the scan path exists and holds at least one .rs file
#   3. only then is the real scan's emptiness believed
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

RULE=".ast-grep/rules/blocking-call-in-native-bif.yml"
FIXTURES=".ast-grep/fixtures"
SCAN_PATH="crates/beamr/src/native/"
PY=/usr/bin/python3

fail() { echo "GATE FAIL: $*" >&2; exit 1; }

[ -f "$RULE" ] || fail "rule file absent: $RULE"
[ -d "$FIXTURES" ] || fail "positive-control fixtures absent: $FIXTURES"
"$PY" -c 'import sys; sys.exit(0)' || fail "$PY is not a runnable interpreter"

# --- precondition 1: the checker fires, once per declared pattern ------------
# Requiring one finding per pattern means adding a pattern to the rule without
# adding its specimen FAILS the gate. An untested pattern is a pattern whose
# zero means nothing.
PATTERNS="$("$PY" - "$RULE" <<'EOF'
import re, sys
text = open(sys.argv[1]).read()
body = text[text.index('rule:'):]
print(len(re.findall(r'^\s*-\s*pattern:', body, re.M)))
EOF
)"
case "$PATTERNS" in
  ''|*[!0-9]*) fail "could not count patterns in $RULE (got '$PATTERNS')" ;;
esac
[ "$PATTERNS" -ge 1 ] || fail "$RULE declares $PATTERNS patterns — the checker is empty"

# ast-grep writes "N error(s) found in code" to stderr for the control's
# deliberate hits. That notice is EXPECTED here and is left visible rather than
# suppressed — a gate that silences its tool's stderr cannot tell "no findings"
# from "could not look".
echo "checking the checker: the next 'error(s) found' notice is the positive control firing, as required"
CONTROL_JSON="$(ast-grep scan -r "$RULE" "$FIXTURES" --json)"
CONTROL_HITS="$(printf '%s' "$CONTROL_JSON" | "$PY" -c 'import json,sys; print(len(json.load(sys.stdin)))')"
case "$CONTROL_HITS" in
  ''|*[!0-9]*) fail "positive control produced unparseable output" ;;
esac
[ "$CONTROL_HITS" -ge "$PATTERNS" ] || fail \
  "positive control matched $CONTROL_HITS of $PATTERNS declared patterns — the checker cannot fire, so a clean scan proves nothing"

# --- precondition 2: there is something to scan ------------------------------
[ -d "$SCAN_PATH" ] || fail "scan path does not exist: $SCAN_PATH (a renamed directory would otherwise report clean)"
RS_COUNT="$(find "$SCAN_PATH" -name '*.rs' -type f | wc -l | tr -d ' ')"
[ "$RS_COUNT" -ge 1 ] || fail "scan path $SCAN_PATH holds no .rs files"

# --- the actual gate ---------------------------------------------------------
SCAN_JSON="$(ast-grep scan -r "$RULE" "$SCAN_PATH" --json)"
HITS="$(printf '%s' "$SCAN_JSON" | "$PY" -c 'import json,sys; print(len(json.load(sys.stdin)))')"
case "$HITS" in
  ''|*[!0-9]*) fail "scan produced unparseable output" ;;
esac

echo "checker verified: $CONTROL_HITS control hits / $PATTERNS patterns; scanned $RS_COUNT .rs files under $SCAN_PATH"
if [ "$HITS" -ne 0 ]; then
  printf '%s\n' "$SCAN_JSON"
  fail "$HITS blocking construct(s) in $SCAN_PATH"
fi
echo "OK: 0 findings, and the zero is trustworthy"
