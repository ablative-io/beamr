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
WALL='^(crates/|conformance/|probes/|\.fleet/)'

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
