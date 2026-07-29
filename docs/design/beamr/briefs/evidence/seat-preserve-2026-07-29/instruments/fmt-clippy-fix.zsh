#!/bin/zsh
set -u
WT=/Users/annabel/Developer/ablative/stack/beamr/.wt-exit001-7a1242eb
CLAIM=/tmp/ablative-gate-battery.claim
SP=/private/tmp/claude-501/-Users-annabel-Developer-ablative-stack-beamr/b337ce2b-336a-4856-a9d8-54c90496c9fa/scratchpad
cd "$WT" || exit 3
pf() { if [[ -e $CLAIM ]]; then echo "HOLD before $1:"; cat "$CLAIM"; exit 42; fi }

pf fmt
cargo fmt --all || exit 4
echo "fmt applied; changed files:"; git status --porcelain | grep -v '^??'

pf clippy
cargo clippy --workspace --all-targets -- -D warnings > "$SP/clippy-verify.txt" 2>&1
C=$?
echo "clippy exit=$C"
[[ $C -ne 0 ]] && { grep -E "^error" "$SP/clippy-verify.txt" | head -5; exit 5; }

pf fmt-check
cargo fmt --all --check > "$SP/fmt-verify.txt" 2>&1
F=$?
echo "fmt-check exit=$F"
[[ $F -ne 0 ]] && exit 5

pf test-sanity
cargo test -p beamr --lib -- wall1 publication_order w2_watch_registered watch_ > "$SP/test-sanity.txt" 2>&1
T=$?
echo "test-sanity exit=$T"
grep "test result" "$SP/test-sanity.txt" | tail -1
[[ $T -ne 0 ]] && exit 5
echo "ALL CLEAN"
