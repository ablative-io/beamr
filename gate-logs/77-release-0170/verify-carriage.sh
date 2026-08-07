#!/bin/zsh
# Re-verify the eight forward-port carriage markers at my own bytes.
#
# Instrument notes, because two earlier attempts produced confident zeros:
#   H1. `$VAR:path` in zsh begins a MODIFIER — `$PRE:crates/...` became
#       `67f89c4rates/...`. Every rev variable is braced here.
#   H2. A failed `git show` feeds EMPTY stdin to grep, which then prints 0.
#       So the producer is captured and its rc asserted SEPARATELY, before
#       anything is counted.
#   H3. `grep -c` exits 1 on a ZERO count. An `|| fail` on the pipeline
#       therefore turns a true zero into "producer failed" — a declined
#       measurement masquerading as a negative one. grep's rc is NOT used.
#   H4. zsh does not word-split unquoted parameters; iterate an array.
set -u

PRE="$(git rev-parse 67f89c4^{commit})"
T163="$(git rev-parse v0.16.3^{commit})"
POST="$(git rev-parse v0.17.0^{commit})"
printf 'pre  = %s\n163  = %s\npost = %s\n\n' "${PRE}" "${T163}" "${POST}"

blob() {  # rev path -> file content on stdout; rc 0 iff the object was read
  git show "${1}:crates/beamr/src/${2}"
}

count() {  # rev path pattern -> count, or aborts if the PRODUCER failed
  local out rc
  out="$(blob "${1}" "${2}")"; rc=$?
  if [ "${rc}" -ne 0 ]; then
    printf 'PRODUCER-FAILED rc=%s %s:%s\n' "${rc}" "${1}" "${2}" >&2
    exit 9
  fi
  # grep rc is deliberately discarded: 1 means zero matches, not failure.
  printf '%s\n' "${out}" | /usr/bin/grep -c -F "${3}" || true
}

typeset -a rows
rows=(
  'jit/runtime_binary_match.rs|Own the bytes'
  'jit/runtime_binary_match.rs|let bytes = bytes.to_vec();'
  'jit/runtime_binary_match.rs|Advance the position BEFORE the allocation'
  'native/gate3_bifs/mod.rs|to_vec()'
  'native/stdlib_stubs/misc_bifs.rs|to_vec()'
  'native/stdlib_stubs/uri_bifs.rs|to_vec()'
  'native/stdlib_stubs/string_bifs.rs|to_vec()'
  'native/etf_bifs.rs|to_vec()'
)

echo '=== marker counts ==='
for row in "${rows[@]}"; do
  f="${row%%|*}"; pat="${row#*|}"
  a="$(count "${PRE}"  "${f}" "${pat}")"
  b="$(count "${POST}" "${f}" "${pat}")"
  printf '%-36s %-44s pre=%-3s post=%s\n' "${f}" "${pat}" "${a}" "${b}"
done

echo
echo '=== CONTROLS ==='
# C1 negative: a pattern that cannot be present must count 0, NOT abort.
printf 'C1 absent-pattern (expect 0): %s\n' \
  "$(count "${POST}" native/etf_bifs.rs 'zzzz-never-present-zzzz')"
# C2 positive: a pattern certain to be present must count > 0.
printf 'C2 present-pattern (expect >0): %s\n' \
  "$(count "${POST}" native/etf_bifs.rs 'fn ')"
# C3 the producer assertion must actually fire on a bad path.
# Its stderr is CAPTURED, not silenced: the whole point of the control is that
# a message gets written, so sending it to /dev/null would prove nothing and
# would leave this run's .err file misleadingly empty.
c3err="$(( count "${POST}" native/no_such_file_xyzzy.rs 'x' ) 2>&1 >/dev/null)"
c3rc=$?
if [ "${c3rc}" -eq 0 ]; then
  echo 'C3 FAILED — bad path did not abort'
else
  printf 'C3 producer-assertion fires on a bad path: OK (rc=%s, said: %s)\n' \
    "${c3rc}" "${c3err}"
fi

echo
echo '=== byte-identity v0.16.3 vs v0.17.0 (the stronger claim) ==='
typeset -a files
files=(
  jit/runtime_binary_match.rs
  native/gate3_bifs/mod.rs
  native/stdlib_stubs/misc_bifs.rs
  native/stdlib_stubs/uri_bifs.rs
  native/stdlib_stubs/string_bifs.rs
  native/etf_bifs.rs
)
for f in "${files[@]}"; do
  a="$(git rev-parse "${T163}:crates/beamr/src/${f}")"
  b="$(git rev-parse "${POST}:crates/beamr/src/${f}")"
  if [ "${a}" = "${b}" ]; then
    printf '%-36s IDENTICAL %s\n' "${f}" "${a}"
  else
    printf '%-36s DIFFERS   %s vs %s\n' "${f}" "${a}" "${b}"
  fi
done
