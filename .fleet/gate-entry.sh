#!/bin/sh
# fleet-gate-template.sh — the flight gate, AS A TEMPLATE. The rig step
# fills the five placeholders and commits the result at .fleet/gate-entry.sh:
#   7cfa17715682dfe72eaacd6c17f9edd87aa1620b     the clone's own HEAD (the diff base for census and sweeps)
#   ^(scripts/gate-docs-truth\.sh|docs/design/beamr/briefs/evidence/docs-truth/|\.fleet/)     path regex: where edits may land
#   ^(scripts/gate-docs-truth\.sh|docs/design/beamr/briefs/evidence/docs-truth/|\.fleet/) path regex: where the deliverable must land
#   -p gleam-types    cargo-nextest package args for the suite leg
#        nextest args for the e2e binaries hammered 10x (empty = skip)
# Derived byte-for-byte from the proven RACES-1 gate (itself from the
# WORKER-ROTATION-2 / cut-3 landed aion gate). ⛔ NO fmt gate — Tom's
# estate gate ruling (GATE-EXTRACTION-STANDARD.md). Discipline: every rc
# captured DIRECTLY, never through a pipe; --no-ext-diff on every diff a
# grep reads; zero-state redded FIRST; receipt names the tree.
set -u
BASE=7cfa17715682dfe72eaacd6c17f9edd87aa1620b
WALL='^(scripts/gate-docs-truth\.sh|docs/design/beamr/briefs/evidence/docs-truth/|\.fleet/)'
PKG_HINT='^(scripts/gate-docs-truth\.sh|docs/design/beamr/briefs/evidence/docs-truth/|\.fleet/)'
red() { echo "GATE-RED: $1"; exit 1; }

echo "== 0. environment =="
# aion clears a declared body's env to PATH alone; aion_home reads $HOME directly.
if [ -n "${HOME:-}" ]; then echo "  HOME supplied by the caller: ${HOME}"; else
  # getent on Linux; dscl on macOS (no getent there — the mix template and
  # fleet-lib already derive both ways; the cargo template now matches).
  if command -v getent >/dev/null 2>&1; then
    HOME=$(getent passwd "$(id -u)" | cut -d: -f6)
  else
    HOME=$(dscl . -read "/Users/$(id -un)" NFSHomeDirectory 2>/dev/null | awk '{print $2}')
  fi
  [ -n "${HOME}" ] || red "no HOME and no passwd home for uid $(id -u) (named cause: environment_homeless)"
  [ -d "${HOME}" ] || red "passwd home ${HOME} is not a directory (named cause: environment_homeless)"
  export HOME; echo "  HOME supplied from passwd: ${HOME}"
fi
PATH="${HOME}/.cargo/bin:${HOME}/.local/bin:${PATH:-/usr/bin:/bin}"; export PATH
# Every temp file the suite makes lands INSIDE the run tree, never on the
# venue's /tmp: rocketfish's /tmp is a quota'd tmpfs, and on 2026-09-02 the
# v031-converter gate lost 19 server e2e tests to "Disk quota exceeded" on a
# pid-file staging write there — a venue condition read as 19 reds. The run
# tree is on the venue's real disk (measured by the floor below) and is
# reaped with the run. Probed with a real write so an unwritable venue reds
# THIS step by name, before any test asks its question.
RUN_TMP="$(cd .. && pwd)/tmp"
mkdir -p "${RUN_TMP}" 2>/dev/null || red "cannot create the run's temp dir ${RUN_TMP} (named cause: tmp_unwritable)"
if ! ( : > "${RUN_TMP}/.probe" ) 2>/dev/null; then red "cannot write in the run's temp dir ${RUN_TMP} (named cause: tmp_unwritable)"; fi
rm -f "${RUN_TMP}/.probe"
TMPDIR="${RUN_TMP}"; export TMPDIR; echo "  TMPDIR: ${TMPDIR}"
# POSIX df -Pk (df -BG is GNU-only and reds the probe itself on BSD venues).
DFREE=$(df -Pk . | awk 'NR==2 {print int($4/1048576)}')
case "${DFREE}" in ''|*[!0-9]*) red "disk free unmeasurable: df -Pk gave '${DFREE}' (named cause: disk_unmeasurable)" ;; esac
echo "  disk free: ${DFREE}G"
[ "${DFREE}" -ge 20 ] || red "only ${DFREE}G free, floor 20G (named cause: disk_floor)"

echo "== 1. toolchain =="
command -v rustc >/dev/null 2>&1 || red "rustc absent (named cause: toolchain_absent)"
command -v cargo >/dev/null 2>&1 || red "cargo absent (named cause: toolchain_absent)"
command -v rustup >/dev/null 2>&1 || red "rustup absent — the pin file would be silently ignored (named cause: toolchain_absent)"
command -v cargo-nextest >/dev/null 2>&1 || red "cargo-nextest absent (named cause: toolchain_absent)"
[ -f rust-toolchain.toml ] || red "rust-toolchain.toml absent (named cause: toolchain_unpinned)"
PIN=$(grep -E '^channel *= *"' rust-toolchain.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
[ -n "${PIN}" ] || red "rust-toolchain.toml carries no channel (named cause: toolchain_unpinned)"
ACTIVE=$(rustc --version 2>&1) || red "rustc --version failed: ${ACTIVE} (named cause: toolchain_absent)"
ACTIVE_V=$(echo "${ACTIVE}" | awk '{print $2}'); echo "  pinned: ${PIN}  active: ${ACTIVE}"
[ "${ACTIVE_V}" = "${PIN}" ] || red "active rustc ${ACTIVE_V} but pin is ${PIN} (named cause: toolchain_mismatch)"
SRC=$(rustup show active-toolchain 2>&1) || red "rustup show active-toolchain failed: ${SRC} (named cause: toolchain_absent)"
case "${SRC}" in *rust-toolchain.toml*) echo "  pin HONORED: ${SRC}" ;; *) red "toolchain matches by version but is not sourced from rust-toolchain.toml (${SRC}) (named cause: pin_unhonored)" ;; esac

echo "== 2. tree receipt =="
HEAD_SHA=$(git rev-parse HEAD) || red "cannot resolve HEAD"
echo "  HEAD: ${HEAD_SHA}"
DIRTY=$(git status --porcelain)
if [ -n "${DIRTY}" ]; then echo "  TREE: DIRTY"; echo "${DIRTY}" | sed 's/^/    /'; red "working tree dirty at gate time (named cause: tree_dirty)"; fi
echo "  TREE: CLEAN"

echo "== 3. paths census =="
CHANGED=$(git diff --no-ext-diff --name-only "${BASE}...HEAD")
[ -n "${CHANGED}" ] || red "no files changed against base ${BASE} (named cause: empty_diff)"
ESCAPEES=$(echo "${CHANGED}" | grep -Ev "${WALL}" || true)
if [ -n "${ESCAPEES}" ]; then echo "  outside the wall:"; echo "${ESCAPEES}" | sed 's/^/    /'; red "files changed outside the wall (named cause: wall_breach)"; fi
DELIV=$(echo "${CHANGED}" | grep -cE "${PKG_HINT}" || true)
[ "${DELIV}" -gt 0 ] || red "no changed file matching the deliverable hint — census would pass vacuously (named cause: deliverable_absent)"
echo "  census OK ($(echo "${CHANGED}" | wc -l | tr -d ' ') files within the wall, ${DELIV} matching the deliverable hint)"

echo "== 3b. surface census =="
# A bun surface leg exists only where the tree MEASURES one: a package.json
# at the root or directly under surface/. No flag, no assumption — presence
# selects the section-6 legs and widens the 5b control (measured on cambium
# 58dfb78: no root package.json; surface/app + surface/mcp).
SURFACE_PKGS=""
for PKG in package.json surface/*/package.json; do
  [ -f "${PKG}" ] && SURFACE_PKGS="${SURFACE_PKGS} $(dirname "${PKG}")"
done
if [ -n "${SURFACE_PKGS}" ]; then echo "  surface packages:${SURFACE_PKGS}"; else echo "  no package.json at root or under surface/ — no surface legs (measured absence)"; fi

echo "== 4. suppression sweep =="
SUPP=$(git diff --no-ext-diff -U0 "${BASE}...HEAD" -- '*.rs' | grep -E '^\+' | grep -cE '#!?\[(allow|expect|ignore)\b|#!?\[cfg\(any\(\)\)|todo!\(|unimplemented!\(' || true)
[ "${SUPP}" -eq 0 ] || red "${SUPP} suppression(s) added in the diff (named cause: suppression_added)"
echo "  clean"

echo "== 5a. clippy =="
cargo clippy --workspace --all-targets -- -D warnings
RC=$?; [ ${RC} -eq 0 ] || red "cargo clippy --workspace --all-targets -- -D warnings rc=${RC} (named cause: clippy_red)"
echo "  clippy clean"

echo "== 5b. new-test control =="
# The control counts EVERY test grammar a tree can carry, always — a brief's
# tests are in whatever language its wall names, and a counter that knows only
# Rust reads a Swift-only or Python-only diff as "zero new tests" and reds a
# green tree (understudy UB-014 rounds 1 and 2, 2026-09-02). Each grammar is
# counted on its own file extensions from the diff's ADDED lines only, printed
# by name so the receipt says which language the tests were in.
# Added lines only, with the grammar's own comment lines dropped: a test that is
# commented out is not a test, in any of the six.
count_added() { git diff --no-ext-diff "${BASE}...HEAD" -- "$@" | grep -E '^\+' | grep -vE '^\+\+\+' ; }
added_code() { count_added "$@" | grep -vE '^\+\s*(//|/\*|\*\s|\*$)' ; }
added_hash_code() { count_added "$@" | grep -vE '^\+\s*#' ; }
RSTESTS=$(added_code '*.rs' | grep -cE '^\+\s*#\[(test|tokio::test|sqlx::test)\b' || true)
# A declaration position only: `test(` / `it(` (or `Deno.test(`) at the start of
# a statement, so RegExp's `.test(x)` in application code or a committed bundle
# never counts. Two residuals a line read cannot see, named rather than hidden:
# the inner lines of a block comment (or a Python docstring) that carry no
# leading `*` still count as code; Swift Testing's `@Test func name()` without
# the `test` prefix is not counted.
TSTESTS=$(added_code '*.ts' '*.tsx' '*.js' '*.jsx' | grep -cE '^\+\s*(export\s+)?(async\s+)?(Deno\.)?(test|it)(\.[a-z]+)*\s*\(' || true)
SWIFTTESTS=$(added_code '*.swift' | grep -cE '^\+\s*(@[A-Za-z]+\s+)*(override\s+)?func\s+test[A-Za-z0-9_]*\s*\(' || true)
PYTESTS=$(added_hash_code '*.py' | grep -cE '^\+\s*(async\s+)?def\s+test_[A-Za-z0-9_]*\s*\(' || true)
GLEAMTESTS=$(added_code '*.gleam' | grep -cE '^\+\s*pub\s+fn\s+[a-z0-9_]+_test\s*\(' || true)
EXSTESTS=$(added_hash_code '*.exs' | grep -cE '^\+\s*(test|describe)\s+"' || true)
echo "  rust #[test]: ${RSTESTS}; ts/js test/it: ${TSTESTS}; swift func test: ${SWIFTTESTS}; python def test_: ${PYTESTS}; gleam _test: ${GLEAMTESTS}; exunit test/describe: ${EXSTESTS}"
NEWTESTS=$((RSTESTS + TSTESTS + SWIFTTESTS + PYTESTS + GLEAMTESTS + EXSTESTS))
[ "${NEWTESTS}" -gt 0 ] || red "the diff adds ZERO test annotations in any grammar the control knows (rust, ts/js, swift, python, gleam, exunit) (named cause: suite_no_new_tests)"
echo "  new test annotations in diff: ${NEWTESTS}"

echo "== 5c. suite =="
LIST_OUT=$(cargo nextest list --workspace 2>&1); LIST_RC=$?
[ ${LIST_RC} -eq 0 ] || { echo "${LIST_OUT}" | tail -20 | sed 's/^/    /'; red "cargo nextest list failed rc=${LIST_RC} (named cause: suite_unlistable)"; }
LISTED=$(echo "${LIST_OUT}" | grep -c '::' || true)
[ "${LISTED}" -gt 0 ] || red "cargo nextest list names ZERO tests (named cause: suite_empty)"
echo "  suite names ~${LISTED} test(s)"
cargo nextest run -p gleam-types --no-fail-fast
RC=$?; [ ${RC} -eq 0 ] || red "suite nextest run rc=${RC} (named cause: suite_red)"
TENX=''
if [ -n "${TENX}" ]; then
  echo "== 5d. the flight's e2e binaries, 10x each =="
  for i in 1 2 3 4 5 6 7 8 9 10; do
    # shellcheck disable=SC2086 — TENX is the dispatch's own nextest args, split on purpose
    cargo nextest run --no-fail-fast ${TENX} > /tmp/gate-tenx-$$.log 2>&1
    RC=$?; tail -1 /tmp/gate-tenx-$$.log
    [ ${RC} -eq 0 ] || red "e2e 10x run ${i}/10 rc=${RC} (named cause: suite_red)"
  done
  rm -f /tmp/gate-tenx-$$.log
  echo "  e2e binaries 10/10"
else
  echo "== 5d. no e2e hammer declared for this flight =="
fi

if [ -n "${SURFACE_PKGS}" ]; then
  echo "== 6. surface packages (bun) =="
  # Measured discipline, mirrored from the cargo legs: bun install first
  # (--frozen-lockfile iff the package COMMITS a lock — bun installs
  # happily without one and writes nothing under --frozen-lockfile, both
  # measured on bun 1.4.0), then ONLY the scripts the package DECLARES,
  # from a fixed list. check:fix, build, and acceptance are never run —
  # a gate measures, it does not rewrite, produce artifacts, or drive
  # browsers. `bun run` propagates the script's exit code exactly
  # (measured). Every rc captured directly; drift checked against the
  # clean tree receipt after every mutating step.
  command -v bun >/dev/null 2>&1 || red "surface packages present but bun absent (named cause: toolchain_absent)"
  command -v jq >/dev/null 2>&1 || red "surface packages present but jq absent — declared scripts unreadable (named cause: toolchain_absent)"
  BUNV=$(bun --version 2>&1)
  RC=$?; [ ${RC} -eq 0 ] || red "bun --version rc=${RC}: ${BUNV} (named cause: toolchain_absent)"
  echo "  bun ${BUNV} at $(command -v bun)"
  for DIR in ${SURFACE_PKGS}; do
    echo "  -- ${DIR} --"
    PKG_SCRIPTS=$(jq -r '.scripts // {} | keys[]' "${DIR}/package.json" 2>&1)
    RC=$?; [ ${RC} -eq 0 ] || red "package.json unreadable in ${DIR}: ${PKG_SCRIPTS} (named cause: surface_manifest_unreadable)"
    if [ -f "${DIR}/bun.lock" ] || [ -f "${DIR}/bun.lockb" ]; then
      ( cd "${DIR}" && bun install --frozen-lockfile )
      RC=$?; [ ${RC} -eq 0 ] || red "bun install --frozen-lockfile rc=${RC} in ${DIR} (named cause: surface_install_red)"
      echo "  install green (lockfile frozen)"
    else
      ( cd "${DIR}" && bun install )
      RC=$?; [ ${RC} -eq 0 ] || red "bun install rc=${RC} in ${DIR} (named cause: surface_install_red)"
      echo "  install green (no lockfile committed — nothing to freeze)"
    fi
    DRIFT=$(git status --porcelain)
    if [ -n "${DRIFT}" ]; then echo "${DRIFT}" | sed 's/^/    /'; red "bun install left the tree dirty in ${DIR} — a lockfile or artifact the branch does not commit (named cause: surface_install_drift)"; fi
    for SCRIPT in typecheck check; do
      if printf '%s\n' "${PKG_SCRIPTS}" | grep -qx "${SCRIPT}"; then
        ( cd "${DIR}" && bun run "${SCRIPT}" )
        RC=$?; [ ${RC} -eq 0 ] || red "bun run ${SCRIPT} rc=${RC} in ${DIR} (named cause: surface_${SCRIPT}_red)"
        echo "  ${SCRIPT} green"
      else echo "  ${SCRIPT}: not declared"; fi
    done
    if printf '%s\n' "${PKG_SCRIPTS}" | grep -qx generate; then
      ( cd "${DIR}" && bun run generate )
      RC=$?; [ ${RC} -eq 0 ] || red "bun run generate rc=${RC} in ${DIR} (named cause: surface_generate_red)"
      DRIFT=$(git status --porcelain)
      if [ -n "${DRIFT}" ]; then echo "${DRIFT}" | sed 's/^/    /'; red "generate changed the tree in ${DIR} — committed artifacts are stale (named cause: surface_generated_drift)"; fi
      echo "  generate green, no drift"
    else echo "  generate: not declared"; fi
    if printf '%s\n' "${PKG_SCRIPTS}" | grep -qx test; then
      ( cd "${DIR}" && bun run test )
      RC=$?; [ ${RC} -eq 0 ] || red "bun run test rc=${RC} in ${DIR} (named cause: surface_suite_red)"
      echo "  test green"
    else echo "  test: not declared"; fi
    for SCRIPT in check:fix build acceptance; do
      printf '%s\n' "${PKG_SCRIPTS}" | grep -qx "${SCRIPT}" && echo "  ${SCRIPT}: declared, NOT measured — a gate never rewrites, builds artifacts, or drives browsers"
    done
  done
  SURFACE_NOTE="; surface pkgs green:${SURFACE_PKGS} (bun ${BUNV})"
else
  echo "== 6. no surface packages measured =="
  SURFACE_NOTE="; no surface packages"
fi

echo "GATE-GREEN: toolchain ${PIN} pin-honored; tree CLEAN at ${HEAD_SHA}; census within wall (${DELIV} in crates/aion); suppressions 0; clippy clean; ${NEWTESTS} new test annotations; ~${LISTED} tests named and green against base ${BASE}${SURFACE_NOTE}"
