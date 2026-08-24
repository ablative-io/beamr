#!/bin/sh
# BEAMR-R8-DEOPT gate — staged by Waffles 2026-08-24. Venue: rocketfish (Linux).
# The verdict half is the repo's OWN canon: every gates.json leg run exactly as
# ci.yml runs it (subshell, redirected-not-piped, no set -e across legs), graded
# by scripts/ci-verdict.sh — one copy of the truth, not restated here.
# Rig arms in front: env bootstrap, floors, tree receipt, census, suppressions.

set -u

# The engine's gate runner is env-scrubbed (PATH survives, HOME does not —
# measured 3x on the dean venue, same worker lineage). Bootstrap the
# instrument env; every CHECK below is unchanged.
HOME="${HOME:-/home/rocketfish}"; export HOME
PATH="${HOME}/.cargo/bin:${HOME}/.local/bin:${HOME}/.bun/bin:${PATH:-/usr/bin:/bin:/usr/sbin:/sbin}"; export PATH
ulimit -n 4096 || echo "  note: fd raise refused, soft stays $(ulimit -n); proceeding"

BASE=272ca9e034b0f78f50e30ec1068ea1eca967e6d6   # beamr main, pinned = the brief's own floor commit

# ⛔⛔ THE ONE NAMED DOOR IN THIS WALL — beamr#31, opened at the fix round of
# beamr-r8-flight1, and it is the seat's to shut.
#
# The wall as drawn (crates/ conformance/ probes/ .fleet/) scopes the R8 DEOPT
# deliverable, and it is correct for that. What it could not anticipate is that
# canon leg 9, `nostd-ratchet`, is BROKEN AT THE PINNED BASE on this venue: the
# leg script builds its cargo log with the BSD `mktemp -t PREFIX` spelling that
# GNU coreutils refuses, so the leg has never once taken a reading on Linux. It
# reds rc=3 CANNOT-MEASURE, structurally, for a cause outside the wall.
#
# That left exactly two reachable outcomes and no third: canon_red (leave a
# known-blind instrument blind) or wall_breach (repair it). The build round
# measured both and escalated the call rather than taking it — report §1b,
# finding F11. The seat's answer to the fix round was "fix the CAUSE the red
# names ... a CANNOT-MEASURE red names the instrument, not the tree ... never by
# skipping the leg." A repair is what that orders, and the repair is one line
# one directory outside the wall.
#
# ⭐ SO THE DOOR IS ONE FILE WIDE, BY EXACT PATH, AND IT IS LOCKED. Not
# `scripts/` — everything else under it, gates.json, ci-verdict.sh and every
# manifest stay walled exactly as before. And because the obvious abuse of a
# door into the ratchet is to walk the CEILING through it, §3b below pins that
# file's JUDGMENT — CEILING, CEILING_PIN, verdict(), parse_tally() — byte-for-
# byte against BASE, unconditionally. The door admits MEASUREMENT REPAIR and
# mechanically refuses anything that changes what the ratchet decides.
#
# Net: this gate now refuses strictly MORE than it did before the door existed.
# A widening that only widens is the move the orders forbid; this one carries
# its own lock, and the lock did not exist until now.
RATCHET='scripts/gate-nostd-ratchet.sh'
WALL='^(crates/|conformance/|probes/|\.fleet/|scripts/gate-nostd-ratchet\.sh$)'

red() { echo "GATE-RED: $1"; exit 1; }

echo "== 1. environment floors =="
DFREE=$(df -BG . | awk 'NR==2 {gsub(/G/,"",$4); print $4}')
echo "  disk free: ${DFREE}G"
[ "${DFREE}" -ge 20 ] || red "only ${DFREE}G free, floor 20G (named cause: disk_floor)"
for TOOL in cargo jq ast-grep; do
  command -v "${TOOL}" >/dev/null || red "${TOOL} absent — the canon cannot run (named cause: toolchain_absent)"
done
command -v wasm-bindgen-test-runner >/dev/null || echo "  note: wasm-bindgen-test-runner absent — the wasm-tests leg will red loudly as its own finding"

echo "== 2. tree receipt =="
HEAD_SHA=$(git rev-parse HEAD) || red "cannot resolve HEAD"
echo "  HEAD: ${HEAD_SHA}"
DIRTY=$(git status --porcelain)
if [ -n "${DIRTY}" ]; then
  echo "  TREE: DIRTY"; echo "${DIRTY}" | sed 's/^/    /'
  red "working tree dirty at gate time (named cause: tree_dirty)"
fi
echo "  TREE: CLEAN"

echo "== 3. paths census =="
CHANGED=$(git diff --no-ext-diff --name-only "${BASE}...HEAD")
[ -n "${CHANGED}" ] || red "no files changed against base ${BASE} (named cause: empty_diff)"
ESCAPEES=$(echo "${CHANGED}" | grep -Ev "${WALL}" || true)
if [ -n "${ESCAPEES}" ]; then
  echo "  outside the wall:"; echo "${ESCAPEES}" | sed 's/^/    /'
  red "files outside the wall (named cause: wall_breach)"
fi
NCRATES=$(echo "${CHANGED}" | grep -cE '^crates/' || true)
[ "${NCRATES}" -gt 0 ] || red "nothing under crates/ — deliverable absent (named cause: deliverable_absent)"
echo "  census OK ($(echo "${CHANGED}" | wc -l | tr -d ' ') files in wall, ${NCRATES} under crates/)"

echo "== 3b. ratchet judgment lock (the named door's lock) =="
# Extract ONLY what the ratchet DECIDES — the ceiling, its pin, and the two
# functions that turn (rc, tally) into a verdict. Everything else in that file
# is comment, self-test and plumbing, which the door exists to let through.
ratchet_judgment() {   # stdin: the ratchet script. stdout: its judgment alone.
  awk '
    /^CEILING=/ || /^CEILING_PIN=/            { print; next }
    /^(verdict|parse_tally)\(\) \{/           { keep = 1 }
    keep                                      { print }
    keep && /^\}$/                            { keep = 0 }
  '
}
# NOT `git show ... | ratchet_judgment` in one shot: a pipeline reports the LAST
# command's status, so awk's 0 would mask a failed `git show` and the guard
# below would be dead code. Take the source first, judge it second.
BASE_SRC=$(git show "${BASE}:${RATCHET}") || red "cannot read ${RATCHET} at ${BASE} (named cause: instrument_broken)"
BASE_J=$(echo "${BASE_SRC}" | ratchet_judgment)
[ -r "${RATCHET}" ] || red "cannot read ${RATCHET} at HEAD (named cause: instrument_broken)"
HEAD_J=$(ratchet_judgment < "${RATCHET}")
# Positive control on the extractor itself: an extractor that returns nothing
# would compare empty-to-empty and "pass" over any edit at all.
BASE_N=$(echo "${BASE_J}" | grep -c '^CEILING=' || true)
[ "${BASE_N}" -eq 1 ] || red "judgment extractor found ${BASE_N} CEILING lines at BASE, wanted 1 — the lock cannot measure (named cause: instrument_broken)"
if [ "${BASE_J}" != "${HEAD_J}" ]; then
  echo "  the ratchet's JUDGMENT changed against ${BASE}:"
  # POSIX sh: no process substitution here, this file is #!/bin/sh.
  LOCKTMP=$(mktemp -d) || red "cannot create lock temp dir (named cause: instrument_broken)"
  echo "${BASE_J}" > "${LOCKTMP}/base"
  echo "${HEAD_J}" > "${LOCKTMP}/head"
  diff "${LOCKTMP}/base" "${LOCKTMP}/head" | sed 's/^/    /'
  rm -rf "${LOCKTMP}"
  red "ratchet judgment altered — the door admits measurement repair ONLY, never a ceiling move (named cause: ratchet_judgment_altered)"
fi
echo "  locked: CEILING, CEILING_PIN, verdict(), parse_tally() byte-identical to ${BASE}"

echo "== 4. suppression sweep =="
SUPP=$(git diff --no-ext-diff -U0 "${BASE}...HEAD" -- '*.rs' | grep -E '^\+' | grep -cE '#\[ignore|#\[allow|#\[expect|#\[cfg\(any\(\)' || true)
[ "${SUPP}" -eq 0 ] || red "${SUPP} suppression(s) added in the diff (named cause: suppression_added)"
echo "  clean"

echo "== 5. the canon (gates.json legs, run as ci.yml runs them) =="
GATE_RC_DIR=$(mktemp -d) || red "cannot create gate-rc temp dir (named cause: instrument_broken)"
# No `set -e` across legs ON PURPOSE: every leg runs, every rc is recorded,
# the verdict is taken afterwards from the recorded set.
declared=$(jq -r '.legs | length' gates.json) || red "cannot read gates.json (named cause: instrument_broken)"
echo "${declared}" > "${GATE_RC_DIR}/declared.count"
echo "  gates.json declares ${declared} legs"
i=0
while [ "${i}" -lt "${declared}" ]; do
  name=$(jq -r ".legs[${i}].name" gates.json)
  cmd=$(jq -r ".legs[${i}].cmd" gates.json)
  echo "  -- leg $((i + 1))/${declared}: ${name}"
  # SUBSHELL (legs stay independent of cwd/vars) and REDIRECTED, NOT PIPED
  # (a pipe would report the downstream tool's status, not the leg's).
  ( eval "${cmd}" ) > "${GATE_RC_DIR}/${name}.log" 2>&1
  rc=$?
  echo "${rc}" > "${GATE_RC_DIR}/${name}.rc"
  if [ "${rc}" -ne 0 ]; then
    echo "  leg ${name} rc=${rc} — last 30 lines:"
    tail -30 "${GATE_RC_DIR}/${name}.log" | sed 's/^/    /'
  fi
  echo "  LEG $((i + 1)) (${name}) rc=${rc}"
  i=$((i + 1))
done

echo "== 6. verdict (scripts/ci-verdict.sh — the one copy of the truth) =="
./scripts/ci-verdict.sh "${GATE_RC_DIR}" gates.json
VERDICT_RC=$?
rm -rf "${GATE_RC_DIR}"
[ "${VERDICT_RC}" -eq 0 ] || red "canon verdict rc=${VERDICT_RC} (named cause: canon_red — read the leg logs above)"

echo "GATE-GREEN: ${DFREE}G free; tree CLEAN at ${HEAD_SHA}; census in wall (${NCRATES} crate files), suppressions 0; all ${declared} canon legs green via ci-verdict.sh against ${BASE}"
