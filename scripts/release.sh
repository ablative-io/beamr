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

is_published() {   # 0 if <crate>@<version> already exists on crates.io
  local name="$1" ver="$2" code
  code="$(curl -s -o /dev/null -w '%{http_code}' "https://crates.io/api/v1/crates/$name/$ver")"
  [[ "$code" == "200" ]]
}

wait_for_index() { # block until <crate>@<version> is queryable on crates.io
  local name="$1" ver="$2"
  printf '    waiting for crates.io index to show %s %s' "$name" "$ver"
  until is_published "$name" "$ver"; do printf '.'; sleep 5; done
  printf ' live.\n'
}

echo "==> beamr workspace release  (repo: $REPO)${DRY_RUN:+  [DRY-RUN]}"
for crate in "${CRATES[@]}"; do
  ver="$(crate_version "$crate")"
  if [[ -z "$DRY_RUN" ]] && is_published "$crate" "$ver"; then
    echo "==> $crate $ver — already on crates.io, skipping."
    continue
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
