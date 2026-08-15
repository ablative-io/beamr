"""AR-1 row 1 -- the ordering detector's break-it arms, each shown ONE-TO-ONE.

A green never observed red is a regression test, not a wall (gate, standing
rule). Four realistic deaths, each expected to be caught by a SPECIFIC
control so no arm can silently stand in for another:

  M0  unmutated baseline               -> rc 0, both controls PASS
      (run FIRST so a stuck-red harness cannot credit a catch)
  A   PRESENCE DEGRADATION: the ordering comparison is disabled, so any
      rooting call anywhere clears the carrier. This is THE row-1 death --
      the instrument every one of the eleven original crossings would
      fool. Caught by ORD-BAD                                  -> rc 2
  B   FLAGS-EVERYTHING: the clean verdict is made a flag. "Flags
      everything" must not read as a pass. Caught by ORD-GOOD  -> rc 2
      (ORD-BAD still passes under B, so the credit is GOOD's alone)
  C   MARKER LOSS: the BAD fixture's binding name is renamed, as a
      refactor or reflow would. Caught by the uniqueness assert -> rc 2
  D   DENOMINATOR: a carrier row is deleted from the ledger. Caught by
      the population check                                     -> rc 4

Every mutation is made on a COPY (detector) or restored BYTE-IDENTICAL
(fixture, ledger -- sha-verified before and after), and M0 is re-run last
to show the tree came back whole. Any arm not dying exactly as expected
fails this script (rc 1). No 2>/dev/null anywhere.
"""

import hashlib
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(".")
DETECTOR = ROOT / ("docs/design/beamr/briefs/evidence/accumulator-rooting/"
                   "ordering_detector.py")
FIXTURE = ROOT / "crates/beamr/src/ar1_ordering_control.rs"
LEDGER = ROOT / ("docs/design/beamr/briefs/evidence/accumulator-rooting/"
                 "dispositions.json")


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def run(script):
    r = subprocess.run([sys.executable, str(script)],
                       capture_output=True, text=True)
    return r.returncode, r.stdout + r.stderr


def mutate_copy(target_line, replacement, tag):
    src = DETECTOR.read_text()
    n = src.count(target_line)
    if n != 1:
        print(f"ABORT {tag}: mutation target matched {n} times, need exactly 1")
        sys.exit(1)
    out = pathlib.Path(tempfile.gettempdir()) / f"ordering_detector_{tag}.py"
    out.write_text(src.replace(target_line, replacement))
    return out


failures = []


def check(tag, rc, out, want_rc, want_in_output):
    ok = rc == want_rc and want_in_output in out
    print(f"{'DIED' if ok and not tag.startswith('M0') else 'PASS' if ok else 'WRONG':>5}  "
          f"{tag}: rc={rc} (want {want_rc}); "
          f"{'found' if want_in_output in out else 'MISSING'} expected marker")
    if not ok:
        failures.append(tag)


print("=== M0: unmutated baseline (stuck-red guard) ===")
rc, out = run(DETECTOR)
check("M0", rc, out, 0, "BOTH CONTROLS PASS")

print("\n=== A: presence degradation (any root anywhere clears) ===")
mut = mutate_copy("    elif first_collect < first_root:",
                  "    elif False:", "A")
rc, out = run(mut)
check("A", rc, out, 2, "ORD-BAD")

print("\n=== B: flags-everything (clean verdict made a flag) ===")
mut = mutate_copy('        verdict = "ROOTED-FIRST/CLEAN"',
                  '        verdict = "ROOTED-FIRST/FLAG"', "B")
rc, out = run(mut)
check("B", rc, out, 2, "ORD-GOOD")
if "FAIL  ORD-BAD" in out:
    print("WRONG  B: ORD-BAD also failed -- the credit must be GOOD's alone")
    failures.append("B-isolation")

print("\n=== C: marker loss (BAD fixture binding renamed) ===")
orig_fixture = FIXTURE.read_bytes()
pre = sha(FIXTURE)
try:
    FIXTURE.write_bytes(orig_fixture.replace(b"ord_bad_tail", b"ord_bad_tail_x"))
    rc, out = run(DETECTOR)
    check("C", rc, out, 2, "UNUSABLE")
finally:
    FIXTURE.write_bytes(orig_fixture)
if sha(FIXTURE) != pre:
    print("ABORT: fixture restore is not byte-identical")
    sys.exit(1)

print("\n=== D: denominator (carrier row deleted from the ledger) ===")
orig_ledger = LEDGER.read_bytes()
pre = sha(LEDGER)
try:
    import json
    doc = json.loads(orig_ledger)
    before = len(doc["sites"])
    doc["sites"] = [s for s in doc["sites"] if s["id"] != 1]
    if len(doc["sites"]) != before - 1:
        print("ABORT D: row deletion removed", before - len(doc["sites"]), "rows")
        sys.exit(1)
    LEDGER.write_text(json.dumps(doc, indent=2))
    rc, out = run(DETECTOR)
    check("D", rc, out, 4, "DENOMINATOR FAILURE")
finally:
    LEDGER.write_bytes(orig_ledger)
if sha(LEDGER) != pre:
    print("ABORT: ledger restore is not byte-identical")
    sys.exit(1)

print("\n=== M0 again: the tree came back whole ===")
rc, out = run(DETECTOR)
check("M0-post", rc, out, 0, "BOTH CONTROLS PASS")

if failures:
    print(f"\nARMS NOT DYING AS EXPECTED: {failures}")
    sys.exit(1)
print("\nALL ARMS DIED ONE-TO-ONE; baseline green before and after; "
      "restores sha-verified.")
sys.exit(0)
