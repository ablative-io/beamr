set -uo pipefail

# DENOMINATOR FIRST. A zero-failure count over a truncated set reads
# exactly like a clean run, so establish the population before
# believing any verdict drawn from it. If the previous step died
# mid-loop, declared and recorded disagree and that IS the finding.
if [ ! -f gate-rc/declared.count ]; then
  echo "no declared-leg count: the canon step did not reach its first leg" >&2
  exit 1
fi
declared=$(cat gate-rc/declared.count)
recorded=$(find gate-rc -name '*.rc' | wc -l | tr -d ' ')

echo "declared legs: ${declared}"
echo "recorded legs: ${recorded}"

if [ "$declared" -ne "$recorded" ]; then
  echo "LEG COUNT MISMATCH: ${declared} declared, ${recorded} recorded — a leg is missing, so no verdict is available from this set" >&2
  exit 1
fi
if [ "$recorded" -eq 0 ]; then
  echo "zero legs recorded: nothing ran, which is not a pass" >&2
  exit 1
fi

verdict=0
for rc_file in gate-rc/*.rc; do
  leg=$(basename "$rc_file" .rc)
  rc=$(cat "$rc_file")
  echo "  ${leg}: rc=${rc}"
  if [ "$rc" -ne 0 ]; then
    verdict=1
  fi
done

# `rc`/`verdict`, never `status`: in zsh `status` is a read-only
# alias for $? and assigning to it is fatal. This file runs under
# bash, but the name is a trap for anyone lifting the idiom.
exit "$verdict"
