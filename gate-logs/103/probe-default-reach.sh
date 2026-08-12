#!/bin/zsh
# #103 instrument: is `impl Default for AtomTable` REACHED by anything?
#
# Arm A  : delete the impl, compile everything. rc 0 => nothing reaches Default.
# Control: delete the impl AND add one non-test `AtomTable::default()` caller.
#          MUST fail, otherwise arm A's rc 0 means nothing.
#
# Aborts if a target string is absent or matches the wrong number of times: a
# mutation that silently mutates nothing is a green that means nothing.
set -e
cd /Users/tom/Developer/ablative/stack/beamr

F=crates/beamr/src/atom/table.rs
BEFORE=$(shasum -a 256 "$F" | cut -d' ' -f1)
echo "table.rs before: $BEFORE"

IMPL='impl Default for AtomTable {
    fn default() -> Self {
        Self::new()
    }
}'

n=$(grep -c '^impl Default for AtomTable {$' "$F")
[ "$n" -eq 1 ] || { echo "ABORT: expected 1 impl Default header, found $n"; exit 2; }

restore() { git checkout -- "$F"; }
trap restore EXIT

run_check () {
  echo "--- host: cargo check --workspace --all-targets --all-features"
  cargo check --workspace --all-targets --all-features
}

# ---------------- Arm A: impl removed ----------------
/usr/bin/python3 - "$F" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
impl = "impl Default for AtomTable {\n    fn default() -> Self {\n        Self::new()\n    }\n}\n\n"
assert s.count(impl) == 1, "ABORT: impl block not found verbatim exactly once"
open(p, "w").write(s.replace(impl, "", 1))
PY
echo "=== ARM A (impl Default deleted) ==="
set +e
run_check
ARM_A=$?
set -e
echo "ARM A rc=$ARM_A"

# ---------------- Control: impl removed + one caller added ----------------
cat >> "$F" <<'RS'

#[allow(dead_code)]
fn control_probe_103_default_caller() -> AtomTable {
    AtomTable::default()
}
RS
echo "=== CONTROL (impl deleted + one non-test AtomTable::default() caller) ==="
set +e
run_check
CONTROL=$?
set -e
echo "CONTROL rc=$CONTROL"

restore
trap - EXIT
AFTER=$(shasum -a 256 "$F" | cut -d' ' -f1)
echo "table.rs after : $AFTER"
[ "$BEFORE" = "$AFTER" ] || { echo "ABORT: file not restored"; exit 3; }

echo
echo "=== RESULT ==="
echo "arm A (impl deleted)      rc=$ARM_A   (0 => NOTHING reaches Default)"
echo "control (deleted+caller)  rc=$CONTROL (nonzero => instrument CAN fire)"
