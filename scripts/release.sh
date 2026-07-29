#!/usr/bin/env bash
#
# release.sh — publish the beamr workspace to crates.io in dependency order.
#
# The pain this solves: workspace crates must be published in DEPENDENCY ORDER
# (a crate can't be published until everything it depends on is already on
# crates.io), and you must wait for the index to catch up between each. This
# script does that, and is IDEMPOTENT — it checks crates.io first and skips any
# crate whose current Cargo.toml version is already published, so it's safe to
# re-run after a partial/failed release.
#
# Usage:
#   scripts/release.sh            # DRY RUN: package + verify every crate, upload nothing
#   scripts/release.sh --dry-run  # same thing, said explicitly
#   scripts/release.sh --publish  # REAL, IRREVERSIBLE publish to crates.io
#
# The default is the safe one ON PURPOSE. A `cargo publish` cannot be undone —
# there is no unpublish — and this script publishes up to four crates per
# invocation. An irreversible action should require you to say so, not require
# you to remember to add a flag to avoid it. Getting the bare command wrong
# should cost a dry run, never a registry write.
#
# Requires: a crates.io token configured (cargo login) and curl.
#
# After this succeeds, roll the new beamr version out to the CONSUMER repos
# (aion / haematite / liminal) — see the printed reminder at the end.

set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"

# Default to --dry-run; only an explicit --publish arms the real thing. Unknown
# arguments are refused rather than ignored, because an argument that is
# silently dropped is how a caller ends up believing they asked for something
# they did not.
DRY_RUN="--dry-run"
case "${1:-}" in
  ""|--dry-run) DRY_RUN="--dry-run" ;;
  --publish)    DRY_RUN="" ;;
  *)
    echo "release.sh: unknown argument '$1'" >&2
    echo "usage: release.sh [--dry-run|--publish]   (default: --dry-run)" >&2
    exit 2
    ;;
esac

# Publish order = dependency order. gleam-types has no intra-workspace deps;
# beamr depends on gleam-types; beamr-cli and beamr-wasm depend on beamr.
CRATES=(gleam-types beamr beamr-cli beamr-wasm)

crate_version() {  # read `version = "x.y.z"` from a crate's Cargo.toml
  grep -m1 '^version' "$REPO/crates/$1/Cargo.toml" | sed -E 's/.*"([^"]+)".*/\1/'
}

# crates.io answers 403 to a request that sends no User-Agent — for EVERY
# crate, published or not. A two-state "is it published?" therefore CANNOT be
# written safely: an unreachable or unhappy registry is indistinguishable from
# a crate that is simply not there, and both call sites below act on that
# answer. So this reports THREE states and the callers must handle all three.
# "Could not tell" is not "no".
UA="beamr-release-script (+https://github.com/ablative-io/beamr)"

publish_state() {  # echoes: published | absent | unknown-<http-code>
  local name="$1" ver="$2" code
  code="$(curl -s -A "$UA" -o /dev/null -w '%{http_code}' "https://crates.io/api/v1/crates/$name/$ver")"
  case "$code" in
    200) echo published ;;
    404) echo absent ;;
    *)   echo "unknown-$code" ;;
  esac
}

wait_for_index() { # block until <crate>@<version> is queryable on crates.io
  local name="$1" ver="$2" state
  printf '    waiting for crates.io index to show %s %s' "$name" "$ver"
  while true; do
    state="$(publish_state "$name" "$ver")"
    case "$state" in
      published) printf ' live.\n'; return 0 ;;
      absent)    printf '.'; sleep 5 ;;
      *)
        # Never spin here. This loop runs AFTER a successful upload, so a
        # registry we cannot read must stop the run loudly rather than print
        # dots forever while looking like progress.
        printf '\n'
        echo "release.sh: crates.io said '$state' while waiting for $name $ver." >&2
        echo "release.sh: THE PUBLISH ITSELF MAY HAVE SUCCEEDED — check crates.io" >&2
        echo "release.sh: by hand before re-running. Do not assume it failed." >&2
        exit 1
        ;;
    esac
  done
}

echo "==> beamr workspace release  (repo: $REPO)${DRY_RUN:+  [DRY-RUN]}"

# State the whole intent before the first upload. A publish is the estate's
# only unrecoverable act and the registry has no claim discipline of its own:
# the operator holding the credential must be able to see, and announce, the
# full set this run will touch BEFORE any of it happens.
echo "==> intends to publish, in order:"
for crate in "${CRATES[@]}"; do
  echo "      $crate $(crate_version "$crate")"
done

for crate in "${CRATES[@]}"; do
  ver="$(crate_version "$crate")"
  if [[ -z "$DRY_RUN" ]]; then
    state="$(publish_state "$crate" "$ver")"
    case "$state" in
      published)
        echo "==> $crate $ver — already on crates.io, skipping."
        continue
        ;;
      absent) ;;
      *)
        echo "release.sh: crates.io said '$state' for $crate $ver." >&2
        echo "release.sh: refusing to publish — cannot tell published from" >&2
        echo "release.sh: unpublished, and this step cannot be undone." >&2
        exit 1
        ;;
    esac
  fi
  echo "==> $crate $ver — publishing${DRY_RUN:+ (dry-run)}"
  ( cd "$REPO" && cargo publish -p "$crate" $DRY_RUN )
  [[ -z "$DRY_RUN" ]] && wait_for_index "$crate" "$ver"
done

echo "==> beamr workspace publish complete."
if [[ -z "$DRY_RUN" ]]; then
  cat <<'EOF'

NEXT — roll the new beamr version out to the consumer repos (these pin beamr by
version, so the pin must be edited; cargo update alone won't cross a 0.x minor):
  • aion:      Cargo.toml          (workspace dep:  beamr = { version = "X" ... })
  • haematite: crates/haematite/Cargo.toml          (beamr = "X")
  • liminal:   crates/liminal/Cargo.toml
               crates/liminal-server/Cargo.toml     (beamr = { version = "X" ... })
Then in each repo:  cargo update -p beamr && cargo check && cargo test
EOF
fi
