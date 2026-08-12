#!/bin/zsh
# #103 two-arm falsifier. A pin is worth nothing until shown it CAN fail on the
# same bytes.
#
#   arm FIXED   : shipped bytes, new() seats COMMON_ATOMS  -> pins MUST pass
#   arm UNFIXED : new() reverted to the pre-fix empty body -> pins MUST fail
#
# Aborts if the target text is absent or matches the wrong number of times: a
# mutation that silently mutates nothing is a green that means nothing.
set -e
cd /Users/tom/Developer/ablative/stack/beamr

F=crates/beamr/src/atom/table.rs
BEFORE=$(shasum -a 256 "$F" | cut -d' ' -f1)
echo "table.rs before: $BEFORE"

restore() { git checkout -- "$F"; }
trap restore EXIT

run_pins () { cargo test -p beamr --all-features atom::table::tests; }

echo "=== ARM FIXED (shipped bytes) ==="
set +e
run_pins
FIXED=$?
set -e
echo "ARM FIXED rc=$FIXED"

/usr/bin/python3 - "$F" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
target = """        let table = Self {
            by_name: DashMap::new(),
            by_index: DashMap::new(),
            next_index: AtomicU32::new(0),
        };

        for &(name, atom) in COMMON_ATOMS {
            table.by_name.insert(name.to_owned(), atom);
            table.by_index.insert(atom.index(), name);
        }

        table
            .next_index
            .store(COMMON_ATOMS.len() as u32, Ordering::Relaxed);
        table
"""
assert s.count(target) == 1, f"ABORT: new() body not found verbatim exactly once (found {s.count(target)})"
mutated = """        Self {
            by_name: DashMap::new(),
            by_index: DashMap::new(),
            next_index: AtomicU32::new(0),
        }
"""
open(p, "w").write(s.replace(target, mutated, 1))
print("mutated: new() no longer seats COMMON_ATOMS (the pre-fix behaviour)")
PY

echo "=== ARM UNFIXED (pre-fix behaviour restored) ==="
set +e
run_pins
UNFIXED=$?
set -e
echo "ARM UNFIXED rc=$UNFIXED"

restore
trap - EXIT
AFTER=$(shasum -a 256 "$F" | cut -d' ' -f1)
[ "$BEFORE" = "$AFTER" ] || { echo "ABORT: file not restored"; exit 3; }
echo "table.rs after : $AFTER  (restored)"

echo
echo "=== RESULT ==="
echo "arm FIXED    rc=$FIXED    (MUST be 0)"
echo "arm UNFIXED  rc=$UNFIXED  (MUST be nonzero)"
if [ "$FIXED" -eq 0 ] && [ "$UNFIXED" -ne 0 ]; then
  echo "VERDICT: pins are load-bearing."
else
  echo "VERDICT: PINS NOT LOAD-BEARING — do not ship them as coverage."
fi
