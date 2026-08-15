#!/usr/bin/env bash
#
# ci-verdict.sh — the CI verdict, extracted from .github/workflows/ci.yml so it
# is ONE copy of the truth that can be run and falsified locally. The workflow
# calls this script; it must never restate this logic.
#
# WHAT THIS FIXES (task #84, follow-up to the #75 landing):
#
# 1. PER-LEG EXIT CONTRACTS ARE KEPT, NOT COLLAPSED. The old inline loop set
#    verdict=1 on any `rc -ne 0`, so a leg with a documented multi-code
#    contract lost it at the host: "I measured and it failed" (typically rc 1)
#    and "I COULD NOT MEASURE" (rc 2, or rc 127 for a script that is not on
#    this machine) became the same red. A lumped nonzero cannot distinguish
#    "the estate has a defect" from "the instrument is not installed" — the
#    mirror of `exit 0` failing to distinguish "not applicable" from "fired
#    and permitted". A leg may now declare `cannot_measure_rcs: [..]` in
#    gates.json; those codes classify CANNOT-MEASURE — still a red (a gate
#    that cannot measure must never look green), but reported as a verdict on
#    the INSTRUMENT, not on the tree, and written to gate-rc/<leg>.class for
#    artifact consumers. Undeclared legs keep the old contract: 0 pass,
#    anything else FAIL.
#
# 2. A MALFORMED rc FILE IS A RED, NOT A PASS. Measured before this script
#    existed: an EMPTY rc file made `[ "" -ne 0 ]` error out, the condition
#    read false, and the leg SILENTLY PASSED. The rc must be a plain number;
#    anything else is its own loud failure.
#
# Self-test: every invocation first proves, against embedded fixtures, that
# each verdict class is reachable — pass, FAIL, CANNOT-MEASURE, MALFORMED,
# LEG COUNT MISMATCH, TEST-COUNT — and exits 3 (instrument broken) if any
# fixture grades wrong. A verdict instrument that has never been seen red is
# a regression test, not a wall.
#
# Usage: ci-verdict.sh [GATE_RC_DIR] [GATES_JSON]   (defaults: gate-rc, gates.json)
# Exit: 0 all legs pass · 1 verdict red (any class) · 3 self-test failed

set -uo pipefail

grade() {
  # grade GATE_RC_DIR GATES_JSON — prints the report, returns the verdict.
  local dir="$1" gates="$2"

  # DENOMINATOR FIRST. A zero-failure count over a truncated set reads
  # exactly like a clean run, so establish the population before believing
  # any verdict drawn from it.
  if [ ! -f "$dir/declared.count" ]; then
    echo "no declared-leg count: the canon step did not reach its first leg" >&2
    return 1
  fi
  local declared recorded
  declared=$(cat "$dir/declared.count")
  recorded=$(find "$dir" -name '*.rc' | wc -l | tr -d ' ')
  echo "declared legs: ${declared}"
  echo "recorded legs: ${recorded}"
  if [ "$declared" -ne "$recorded" ]; then
    echo "LEG COUNT MISMATCH: ${declared} declared, ${recorded} recorded — a leg is missing, so no verdict is available from this set" >&2
    return 1
  fi
  if [ "$recorded" -eq 0 ]; then
    echo "zero legs recorded: nothing ran, which is not a pass" >&2
    return 1
  fi

  # SECOND DENOMINATOR: a test leg that RAN NOTHING exits 0. Keyed on the
  # leg's declared KIND so a cargo-test leg added to gates.json inherits this
  # assertion with no edit here. Asserted on the leg TOTAL, never per line:
  # a feature-gated test binary legitimately prints `ok. 0 passed`.
  local i kind name log lines passed
  for i in $(seq 0 $((declared - 1))); do
    kind=$(jq -r ".legs[$i].kind // \"\"" "$gates")
    [ "$kind" = "cargo-test" ] || continue
    name=$(jq -r ".legs[$i].name" "$gates")
    log="$dir/${name}.log"
    if [ ! -f "$log" ]; then
      echo "TEST-COUNT: ${name} declares kind cargo-test but produced no log" >&2
      return 1
    fi
    # `| wc -l`, not `grep -c`: grep -c exits 1 on zero matches, so the count
    # and the failure would share one channel. Zero IS the finding here.
    lines=$(grep '^test result:' "$log" | wc -l | tr -d ' ')
    passed=$(awk '/^test result:/ {for (j=1;j<=NF;j++) if ($j=="passed;") p+=$(j-1)} END {print p+0}' "$log")
    echo "  ${name}: ${lines} result line(s), ${passed} passed"
    if [ "$lines" -eq 0 ]; then
      echo "TEST-COUNT: ${name} emitted no 'test result:' line — it ran no test binary, which is not a pass" >&2
      return 1
    fi
    if [ "$passed" -eq 0 ]; then
      echo "TEST-COUNT: ${name} passed 0 tests across ${lines} binaries — an empty population, which is not a pass" >&2
      return 1
    fi
  done

  # CLASSIFICATION. Three classes, each written beside the rc so the artifact
  # carries the distinction the exit code cannot: pass | fail | cannot-measure.
  local verdict=0 rc_file leg rc contract class
  for rc_file in "$dir"/*.rc; do
    leg=$(basename "$rc_file" .rc)
    rc=$(cat "$rc_file")
    case "$rc" in
      ''|*[!0-9]*)
        # The measured defect: an empty rc made `[ -ne ]` error and the leg
        # silently passed. A verdict channel carrying garbage is a red.
        echo "  ${leg}: MALFORMED rc '${rc}' — the verdict channel itself is broken, which is not a pass" >&2
        echo "malformed" > "$dir/${leg}.class"
        verdict=1
        continue
        ;;
    esac
    if [ "$rc" -eq 0 ]; then
      class="pass"
    elif jq -e --arg leg "$leg" --argjson rc "$rc" \
        '.legs[] | select(.name == $leg) | (.cannot_measure_rcs // []) | index($rc) != null' \
        "$gates" > /dev/null; then
      # Documented "I could not measure" code: red, but a verdict on the
      # INSTRUMENT — do not let it read as a defect in the tree, and never
      # let it read as green.
      class="cannot-measure"
    else
      class="fail"
    fi
    echo "$class" > "$dir/${leg}.class"
    case "$class" in
      pass) echo "  ${leg}: rc=${rc} pass" ;;
      fail) echo "  ${leg}: rc=${rc} FAIL — measured red"; verdict=1 ;;
      cannot-measure)
        echo "  ${leg}: rc=${rc} CANNOT-MEASURE — the instrument could not produce a verdict (declared contract); this reds the run but is not a measured defect" >&2
        verdict=1
        ;;
    esac
  done
  return "$verdict"
}

self_test() {
  # Embedded fixtures, one per verdict class the instrument claims to
  # produce. Fixture-based, never keyed on the live gates.json — a control
  # keyed on live state dies when the state changes.
  local tmp want got out
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/ci-verdict-selftest.XXXXXX")
  trap 'rm -rf "$tmp"' RETURN

  cat > "$tmp/gates.json" <<'FIX'
{"legs": [
  {"name": "alpha", "kind": "check"},
  {"name": "beta", "kind": "cargo-test"},
  {"name": "gamma", "kind": "check", "cannot_measure_rcs": [2, 127]}
]}
FIX

  _case() { # _case NAME EXPECT_RC EXPECT_MARKER SETUP_FN
    local name="$1" want_rc="$2" marker="$3" setup="$4" dir out rc
    dir="$tmp/$name" && mkdir -p "$dir"
    "$setup" "$dir"
    out=$(grade "$dir" "$tmp/gates.json" 2>&1)
    rc=$?
    if [ "$rc" -ne "$want_rc" ] || ! grep -q "$marker" <<< "$out"; then
      echo "SELF-TEST FAILED: case ${name} rc=${rc} (want ${want_rc}), marker '${marker}' $(grep -q "$marker" <<< "$out" && echo found || echo MISSING)" >&2
      echo "$out" >&2
      exit 3
    fi
    echo "self-test: ${name} -> ${marker} (expected)"
  }

  _all_green() {
    echo 3 > "$1/declared.count"
    echo 0 > "$1/alpha.rc"; echo 0 > "$1/beta.rc"; echo 0 > "$1/gamma.rc"
    echo "test result: ok. 5 passed; 0 failed; 0 ignored" > "$1/beta.log"
  }
  _one_fail()      { _all_green "$1"; echo 1 > "$1/alpha.rc"; }
  _cannot()        { _all_green "$1"; echo 2 > "$1/gamma.rc"; }
  _uncontracted()  { _all_green "$1"; echo 2 > "$1/alpha.rc"; }
  _malformed()     { _all_green "$1"; printf "" > "$1/alpha.rc"; }
  _truncated()     { _all_green "$1"; rm "$1/gamma.rc"; }
  _empty_tests()   { _all_green "$1"; echo "test result: ok. 0 passed; 0 failed; 0 ignored" > "$1/beta.log"; }

  _case all-green      0 "gamma: rc=0 pass"    _all_green
  _case one-fail       1 "FAIL — measured red" _one_fail
  _case cannot-measure 1 "CANNOT-MEASURE"      _cannot
  _case uncontracted-2 1 "alpha: rc=2 FAIL"    _uncontracted
  _case malformed-rc   1 "MALFORMED rc"        _malformed
  _case truncated-set  1 "LEG COUNT MISMATCH"  _truncated
  _case empty-tests    1 "TEST-COUNT"          _empty_tests
}

self_test
grade "${1:-gate-rc}" "${2:-gates.json}"
exit $?
