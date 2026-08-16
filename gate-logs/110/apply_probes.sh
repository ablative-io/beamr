#!/usr/bin/env bash
# AR-1 row 4 — apply every banked probe to its target file.
#
# ⛔ THIS SCRIPT EXISTS BECAUSE I GOT AN APPEND OFFSET WRONG TWICE. Once
# harmlessly (a stray comment line dragged into site 3's append) and once
# loudly (site 12/16 lost its `cfg` line and rustc reported `E0432` against a
# line that was written correctly). The harmless instance is the dangerous one:
# it is what trains you to stop checking. ⇒ NO OFFSET IS TRANSCRIBED HERE. Each
# one is DERIVED from the probe's own `cfg` attribute and then CHECKED against
# what actually landed in the target.
#
# Refuses (exit 2) on: a missing probe or target, a probe whose module opener
# cannot be found, a non-blank line where the blank separator must be, a target
# that already carries the module (so a second run cannot silently double it),
# or an appended region whose first two lines are not exactly what was intended.
#
# Usage:  bash gate-logs/110/apply_probes.sh            # apply + verify
#         bash gate-logs/110/apply_probes.sh --check    # verify only, no writes
set -uo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 2

CHECK_ONLY=0
[ "${1:-}" = "--check" ] && CHECK_ONLY=1

# probe                       target
PAIRS=(
  "probe_site_1.rs.txt        crates/beamr/src/distribution/control.rs"
  "probe_site_2.rs.txt        crates/beamr/src/distribution/pg.rs"
  "probe_site_3.rs.txt        crates/beamr/src/native/code_management_bifs.rs"
  "probe_site_5.rs.txt        crates/beamr/src/native/file_meta_bifs.rs"
  "probe_site_7.rs.txt        crates/beamr/src/native/stdlib_stubs/uri_bifs.rs"
  "probe_sites_8_9.rs.txt     crates/beamr/src/native/stdlib_stubs/uri_bifs.rs"
  "probe_site_10.rs.txt       crates/beamr/src/native/udp_bifs.rs"
  "probe_sites_11_15.rs.txt   crates/beamr/src/term/json.rs"
  "probe_sites_12_16.rs.txt   crates/beamr-wasm/src/convert.rs"
  "probe_site_14.rs.txt       crates/beamr/src/native/stdlib_stubs/string_bifs.rs"
)

DIR="gate-logs/110"
rc=0

for pair in "${PAIRS[@]}"; do
  read -r probe target <<<"$pair"
  src="$DIR/$probe"

  if [ ! -f "$src" ];    then echo "REFUSED $probe: probe file absent";  rc=2; continue; fi
  if [ ! -f "$target" ]; then echo "REFUSED $probe: target $target absent"; rc=2; continue; fi

  # The module opener is whatever cfg attribute the probe itself carries —
  # `#[cfg(test)]` for the native probes, `#[cfg(all(test, target_arch = ...))]`
  # for the wasm pair. Read it, never assume it.
  n=$(grep -n '^#\[cfg(' "$src" | head -1 | cut -d: -f1)
  if [ -z "$n" ]; then echo "REFUSED $probe: no module opener"; rc=2; continue; fi
  if [ "$n" -lt 2 ]; then echo "REFUSED $probe: opener at line $n, no separator above it"; rc=2; continue; fi

  sep=$(sed -n "$((n - 1))p" "$src")
  if [ -n "$sep" ]; then
    echo "REFUSED $probe: line $((n - 1)) is not blank — the append would drag a comment in"
    rc=2; continue
  fi

  opener=$(sed -n "${n}p" "$src")
  modline=$(sed -n "$((n + 1))p" "$src")
  modname=$(printf '%s' "$modline" | sed -n 's/^mod \([a-z0-9_]*\) {$/\1/p')
  if [ -z "$modname" ]; then echo "REFUSED $probe: line $((n + 1)) is not a module opener: $modline"; rc=2; continue; fi

  present=$(grep -c "^mod $modname {\$" "$target")

  if [ "$CHECK_ONLY" = 1 ]; then
    if [ "$present" = 1 ]; then echo "PRESENT  $probe -> $target  (mod $modname, offset $((n - 1)))"
    else echo "ABSENT   $probe -> $target  (mod $modname)"; rc=2; fi
    continue
  fi

  if [ "$present" != 0 ]; then
    echo "REFUSED $probe: $target already carries \`mod $modname\` — a second append would double it"
    rc=2; continue
  fi

  before=$(wc -l < "$target")
  tail -n +"$((n - 1))" "$src" >> "$target"
  after=$(wc -l < "$target")

  # ⭐ VERIFY WHAT LANDED, not what was intended. The check reads the target.
  got_sep=$(sed -n "$((before + 1))p" "$target")
  got_opener=$(sed -n "$((before + 2))p" "$target")
  got_mod=$(sed -n "$((before + 3))p" "$target")
  if [ -n "$got_sep" ] || [ "$got_opener" != "$opener" ] || [ "$got_mod" != "$modline" ]; then
    echo "REFUSED $probe: appended region is wrong at $target:$((before + 1))"
    echo "  want: <blank> / $opener / $modline"
    echo "  got : '$got_sep' / '$got_opener' / '$got_mod'"
    rc=2; continue
  fi

  echo "APPLIED  $probe -> $target  offset $((n - 1))  +$((after - before)) lines  (mod $modname)"
done

if [ "$rc" != 0 ]; then echo; echo "⛔ REFUSED — nothing above may be trusted as applied."; fi
exit "$rc"
