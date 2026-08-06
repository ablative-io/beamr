# No `set -e` here ON PURPOSE: a failing leg must not abort the loop.
# Every leg runs, every rc is recorded, and the verdict is taken
# afterwards from the recorded set. Aborting on the first failure
# would report one defect and hide the rest.
set -uo pipefail
mkdir -p gate-rc

declared=$(jq -r '.legs | length' gates.json)
echo "$declared" > gate-rc/declared.count
echo "gates.json declares ${declared} legs"

for i in $(seq 0 $((declared - 1))); do
  name=$(jq -r ".legs[$i].name" gates.json)
  cmd=$(jq -r ".legs[$i].cmd" gates.json)
  echo "::group::leg $((i + 1)) — ${name}"
  echo "COMMAND: ${cmd}"
  # Not piped: there is no pipefail guarantee for the leg itself and
  # a pipe would report the downstream tool's status instead of the
  # leg's (gates.json says this explicitly of the ast-grep leg).
  #
  # SUBSHELL, and it is load-bearing: `eval` runs in the CURRENT
  # shell, so a leg shaped `cd crates/foo && cargo test` would leave
  # every LATER leg running from the wrong directory — silently,
  # because cargo would still find a manifest there. Same for a leg
  # that exports a variable or assigns one the loop uses ($i, $name,
  # $cmd, $rc, $declared). The subshell makes the legs genuinely
  # independent, so the verdict does not depend on leg ORDER.
  # Where a gate sits relative to its siblings is part of its
  # correctness, not its performance (Hermes, 2026-08-06).
  ( eval "$cmd" )
  rc=$?
  echo "$rc" > "gate-rc/${name}.rc"
  echo "LEG $((i + 1)) (${name}) rc=${rc}"
  echo "::endgroup::"
done
