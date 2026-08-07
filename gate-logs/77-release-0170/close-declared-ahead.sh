#!/bin/zsh
# The §6 close, re-run at my own hands after the publish.
# Metadata endpoint, never the download endpoint (it 302s blindly).
# User-Agent mandatory: crates.io answers 403 to a request without one,
# which parses as "nothing is published" and closes perfectly while false.
set -u
UA='beamr-release-script (+https://github.com/ablative-io/beamr)'
probe() {
  printf '%-28s %-8s HTTP %s\n' "${1}" "${2}" \
    "$(curl -s -A "${UA}" -o /dev/null -w '%{http_code}' \
        "https://crates.io/api/v1/crates/${1}/${2}")"
}
echo '--- declared in the tree ---'
/usr/bin/grep -m1 '^version' crates/*/Cargo.toml
echo
echo '--- declared-ahead probe ---'
probe gleam-types 0.4.3
probe beamr       0.17.0
probe beamr-cli   0.5.0
probe beamr-wasm  0.8.0
echo
echo '--- controls, same invocation ---'
probe beamr 0.16.3                      # positive: must be 200
probe beamr 0.99.0                      # negative, existing crate: must be 404
probe beamr-nonexistent-xyzzy 1.0.0     # negative, absent crate: must be 404
