#!/bin/zsh
# RF-006 mutation evidence driver.
#
# A wall whose regression-catching is unproven is an untested test. Each
# mutation below is a minimal SEMANTIC change to production code that
# reintroduces the exact defect its wall exists to catch. None of them is ever
# committed: this script applies one, measures, and reverts.
#
# Run from the repository root. Requires a clean working tree.
#
# Controls, because "the suite went red" is worthless if the mutation never
# landed and worthless again if it stayed behind:
#   C1  tree is clean BEFORE each apply
#   C2  the patch APPLIED — changed-line count matches the recorded expectation
#   C3  tree is clean AFTER each revert (no contamination of the next leg)
# C2 is the one that matters. An unapplied patch and a passing suite are
# indistinguishable from a killed mutation if you only read the exit code.

set -uo pipefail

MUT_DIR="docs/design/beamr/briefs/evidence/review-23-07/rf-006/mutations"
RUN_DIR="docs/design/beamr/briefs/evidence/review-23-07/rf-006/runs"

# name | file | expected changed lines | the wall that must catch it
MUTATIONS=(
  "M1-start-match-drops-forwarded-source|crates/beamr/src/jit/runtime_binary_match.rs|1|jit::runtime_binary_match::gc_hazard_tests::start_match_source_survives_forced_collection"
  "M2-map-roots-a-copy|crates/beamr/src/jit/runtime_map.rs|2|jit::runtime_map::gc_hazard_tests::map_update_boxed_values_survive_forced_collection"
  "M4-sharing-arm-drops-forwarded-parent|crates/beamr/src/jit/runtime_binary_match.rs|1|jit::runtime_binary_match::gc_hazard_tests::bs_get_binary_procbin_extraction_shares_forwarded_parent"
)

for entry in "${MUTATIONS[@]}"; do
  name="${entry%%|*}"; rest="${entry#*|}"
  file="${rest%%|*}"; rest="${rest#*|}"
  expected="${rest%%|*}"; wall="${rest#*|}"

  print -- "=== ${name} ==="
  df -k /System/Volumes/Data | tail -1

  # C1
  git diff --no-ext-diff --quiet
  if [[ $? -ne 0 ]]; then
    print -- "C1 FAILED: working tree dirty before applying ${name}"
    exit 2
  fi

  git apply "${MUT_DIR}/${name}.diff"
  if [[ $? -ne 0 ]]; then
    print -- "APPLY FAILED: ${name}"
    exit 2
  fi

  # C2 — the patch actually changed the tree, by the expected amount.
  applied=$(git diff --no-ext-diff -U0 -- "${file}" | grep -cE '^[+-][^+-]')
  print -- "C2 applied changed-lines: ${applied} (expected ${expected})"
  if [[ "${applied}" -ne "${expected}" ]]; then
    print -- "C2 FAILED: ${name} did not apply as recorded"
    git checkout -- "${file}"
    exit 2
  fi

  cargo test -p beamr --lib > "${RUN_DIR}/mut-${name}.txt" 2>&1
  print -- $? > "${RUN_DIR}/mut-${name}.rc"
  print -- "rc: $(cat "${RUN_DIR}/mut-${name}.rc")"
  grep -E '^test result:' "${RUN_DIR}/mut-${name}.txt"
  print -- "wall in failure list:"
  grep -cF "    ${wall}" "${RUN_DIR}/mut-${name}.txt"

  git checkout -- "${file}"

  # C3
  git diff --no-ext-diff --quiet
  if [[ $? -ne 0 ]]; then
    print -- "C3 FAILED: tree still dirty after reverting ${name}"
    exit 2
  fi
  print -- "C3 ok: tree reverted"
done

print -- "=== all mutations measured and reverted ==="
git status --short --untracked-files=no
