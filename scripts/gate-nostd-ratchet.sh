#!/usr/bin/env bash
# Gate: the no-std error count may go DOWN, never up.
#
# ⛔ WHY THIS GATE EXISTS (#219, measured — not a hypothetical)
#
# `cargo check -p beamr --no-default-features` has been RED for a long time and
# the redness is DELIBERATELY WAIVED: CHANGELOG.md and
# docs/DIST-CONTROL-WIRE-SPEC.md both record it as "waived, not green.
# Restoring no-std is a separate work item." That waiver was honest.
#
# What nothing recorded is that the number MOVED:
#
#     ec5d7f8  (2026-07-07)   1019 errors
#     d4a82e5  (2026-08-16)   1039 errors      +20, five minor versions, unremarked
#
# Both measured with rustc's OWN tally at both pins, same error composition
# (E0433/E0425-dominated) at both ends -- so it is drift, and not an artifact of
# building an old commit under a newer toolchain.
#
# ⚠️ THE COUNTING RULE, because two seats already tripped on it. Use rustc's own
# "due to N previous errors" line. Do NOT count `^error` lines: the trailing
# `error: could not compile ...` SUMMARY LINE is itself an `error` line, so a
# naive grep reads exactly one too many. The record's "1020" and my own first
# reading of "1040" are both that same off-by-one against rustc's 1019 / 1039.
# Neither was wrong about the world; they counted the summary line. Whenever a
# figure from this gate is quoted beside a historical one, STATE THIS, or the
# next reader concludes a seat miscounted and the accusation outlives it.
#
# ⛔ WHY A WAIVER WITHOUT A RATCHET WAS THE REAL DEFECT
#
# Two structural halves hid the drift, and NEITHER HIDES IT ALONE:
#   1. the waiver lived in PROSE -- honest, explicit, and unwatchable;
#   2. `--no-default-features` is INERT under `cargo test`, because
#      crates/beamr/Cargo.toml's dev-dependency on beamr ITSELF carries default
#      features; the flag binds the package named on the command line while the
#      dev edge is a separate edge asking for `default = true`, and cargo
#      unifies features across edges. So no test-shaped gate could ever have
#      selected the configuration that would show the number.
# The debt grew in the SEAM between them. This leg closes the first half; the
# manifest edge is its own lane (it changes what every `cargo test` builds).
#
# ⚠️ THIS GATE IS DELIBERATELY STRICT IN BOTH DIRECTIONS -- my call, stated so
# it can be overruled rather than discovered. An IMPROVEMENT also fails, and
# demands the ceiling be lowered in the same commit. A ratchet that tolerates
# slack is not a ratchet: leave the ceiling above the true count and the number
# is free to drift back up to it silently, which is precisely the failure this
# gate exists to prevent.
#
# ⛔⛔ THE ONE TIME THIS CEILING WAS RAISED, AND WHY THAT IS NOT DRIFT
#
#     1019  ec5d7f8  (2026-07-07)
#     1039  d4a82e5  (2026-08-16)   +20 drift -- the reason this gate exists
#     1075  B-144 R1 (2026-08-17)   +36 POPULATION CHANGE -- ruled, not drift
#
# B-144 R1 removed `#[cfg(any(threads, cooperative))]` from `pub mod timer` and
# `pub mod replay`. Those gates had been keeping both modules OUT OF THE COMPILE
# UNIT entirely under `--no-default-features`, so their std dependencies were
# never counted by this gate. Removing the gates did not add a single std
# dependency; it made ~56 existing ones VISIBLE, while the same change fixed 20
# others. GROSS UP +56, GROSS DOWN -20, NET +36.
#
# ⭐ A CFG GATE IS A HOLE IN EVERY MEASUREMENT TAKEN THROUGH IT. The 1039 was
# never a count of beamr's no_std debt -- it was a count of the part the gates
# let the compiler see.
#
# PROVEN, NOT ASSERTED (gate-logs/B-144-R1/SPOT-PROOF.md, ruled term 3): every
# one of the +56 lives in timer.rs or replay/, files the fix does not touch and
# which are therefore byte-identical across it; three sampled errors are plain
# `use std::...` lines recovered from the committed git object at the pre-fix
# pin; and at that pin 12 errors named the modules NONEXISTENT while ZERO errors
# were located inside them, because the compiler never looked.
#
# ⚠️ THE NEVER-RAISE RULE BENDS ONLY FOR A MEASURED POPULATION CHANGE, AND ONLY
# WITH THE REASON WRITTEN WHERE THE NUMBER LIVES. A raise justified by anything
# less -- "it got harder", "that code is new" -- is the drift this gate exists to
# catch, wearing a better excuse.
#
# Run `./scripts/gate-nostd-ratchet.sh --self-test` to prove the arms fire.
set -uo pipefail

CEILING=1072           # rustc's own tally. LOWER THIS, NEVER RAISE IT --
                       # except for a MEASURED POPULATION CHANGE, of which
                       # B-144 R1 (1039 -> 1075) is the first and only
                       # instance. See below. 1075 -> 1072 at the 0.19.0 cut:
                       # commit 4e8ccf6 (site-4 trio deletion + #104 scaffold
                       # demotion) removed three errors from the no_std tally.
                       # Which of its two changes carried which error is NOT
                       # attributed -- the delta is commit-level, measured by
                       # this gate itself. A tightening, exactly the direction
                       # this ratchet exists to bank.
CEILING_PIN="0.19.0 cut (site-4 trio deletion)"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 99

# --- the verdict, as a pure function of (rc, tally) -------------------------
# Kept free of cargo so the self-test can exercise every arm with synthetic
# inputs instead of waiting on four real builds. RC_NOSTD=0 means the no-std
# build SUCCEEDED; TALLY empty means the tally line was absent.
#   0 = pass   1 = ratchet breached   2 = improved, lower the ceiling
#   3 = cannot measure (REFUSE, never a pass)
verdict() {
  local rc="$1" tally="$2"

  if [ "$rc" -eq 0 ]; then
    echo "PASS (LOUDLY): no-std COMPILES CLEAN at this pin. The waiver is over."
    echo "  Drop CEILING to 0 and consider retiring this leg for a real gate."
    return 0
  fi

  if [ -z "$tally" ]; then
    echo "REFUSE: cargo exited $rc but no \"due to N previous errors\" line was" >&2
    echo "  found. This gate cannot measure, so it does not get to report." >&2
    echo "  A green here would be worth nothing -- same value whether the tree" >&2
    echo "  is sound or the instrument is dead. Fix the parse; do NOT relax it" >&2
    echo "  to make this pass." >&2
    return 3
  fi

  if [ "$tally" -gt "$CEILING" ]; then
    echo "FAIL: no-std errors $tally EXCEEDS the ratchet ceiling $CEILING" >&2
    echo "  (ceiling set at $CEILING_PIN). New no-std breakage has been added." >&2
    echo "  The waiver permits the debt to STAND, not to GROW." >&2
    return 1
  fi

  if [ "$tally" -lt "$CEILING" ]; then
    echo "FAIL: no-std errors $tally is BELOW the ceiling $CEILING -- this is" >&2
    echo "  an IMPROVEMENT, and the ratchet must be tightened to record it." >&2
    echo "  Set CEILING=$tally in this script, in the same commit." >&2
    echo "  (Strict by design: slack in a ratchet is room to drift back up.)" >&2
    return 2
  fi

  echo "PASS: no-std errors $tally, exactly at the ceiling $CEILING. Debt held."
  return 0
}

parse_tally() {   # stdin: a cargo log. stdout: N, or nothing.
  grep -oE 'due to [0-9]+ previous errors?' | grep -oE '[0-9]+' | tail -1
}

# --- self-test: every arm, both directions, on MINTED inputs ----------------
if [ "${1:-}" = "--self-test" ]; then
  echo "gate-nostd-ratchet self-test (ceiling $CEILING)"
  pass=0; total=0
  arm() {  # arm <name> <expected-rc> <rc> <tally>
    local name="$1" want="$2" rc="$3" tally="$4" got
    verdict "$rc" "$tally" >/dev/null 2>&1; got=$?
    total=$((total+1))
    if [ "$got" -eq "$want" ]; then pass=$((pass+1)); echo "  PASS  $name (rc $got)"
    else echo "  FAIL  $name: wanted rc $want, got $got"; fi
  }

  # The parser must actually parse -- a positive control on the instrument
  # itself, using rustc's real wording rather than a paraphrase of it.
  SPECIMEN='error: could not compile `beamr` (lib) due to 1039 previous errors; 1 warning emitted'
  got=$(printf '%s\n' "$SPECIMEN" | parse_tally)
  total=$((total+1))
  if [ "$got" = "1039" ]; then pass=$((pass+1)); echo "  PASS  parser/real-rustc-wording -> 1039"
  else echo "  FAIL  parser/real-rustc-wording: got '$got'"; fi

  # ...and must NOT invent a number when there is none to find.
  got=$(printf 'error: something else entirely\n' | parse_tally)
  total=$((total+1))
  if [ -z "$got" ]; then pass=$((pass+1)); echo "  PASS  parser/no-anchor -> empty"
  else echo "  FAIL  parser/no-anchor: invented '$got'"; fi

  arm "over-ceiling/must-FAIL"      1 101 $((CEILING+1))
  arm "at-ceiling/must-PASS"        0 101 "$CEILING"
  arm "under-ceiling/must-FAIL"     2 101 $((CEILING-1))
  arm "no-tally/must-REFUSE"        3 101 ""
  arm "compiles-clean/must-PASS"    0 0   ""

  echo "$pass/$total"
  [ "$pass" -eq "$total" ] || { echo "SELF-TEST FAILED"; exit 1; }
  echo "✅ every arm fired"
  exit 0
fi

# --- the real run -----------------------------------------------------------
LOG="$(mktemp -t nostd-ratchet)"
# ⛔ never 2>/dev/null: rustc's tally goes to stderr and IS the measurement.
cargo check -p beamr --no-default-features > "$LOG" 2>&1
RC=$?
TALLY="$(parse_tally < "$LOG")"
echo "no-std ratchet: cargo rc=$RC, rustc tally=${TALLY:-<absent>}, ceiling=$CEILING"
verdict "$RC" "$TALLY"
V=$?
rm -f "$LOG"
exit $V
