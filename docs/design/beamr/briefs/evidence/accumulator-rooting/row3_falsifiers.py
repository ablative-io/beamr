"""AR-1 row 3 -- subtract-one-word falsifiers on the three reserve arithmetics.

A green never observed red is a regression test, not a wall. Each of the
three exactness walls landed in this row recomputes its promise from the
SAME production arithmetic it guards, so shrinking that arithmetic by one
word moves the promise side while the measured side (the real heap delta)
stays put: the wall MUST go red. Three arms, one per arithmetic:

  F-ets    ets_bifs::info_proplist_heap_words        item_count * 5 -> * 5 - 1
           kills info_one_reserve_covers_every_allocation_exactly
  F-sys    system_info_bifs::info_proplist_heap_words item_count * 5 -> * 5 - 1
           kills memory_zero_reserve_covers_every_allocation_exactly
  F-pi     process_info_bifs::value_heap_words Monitors * 5 -> * 4
           (the parent defect restored verbatim)
           kills process_info_reserve_covers_every_allocation_exactly

Each arm: mutate -> run ONLY the three walls -> require rc != 0 AND the
named test FAILED AND the other two walls still ok -> restore byte-identical
(sha256 checked). A final unmutated run must be green. No 2>/dev/null.
"""

import hashlib
import pathlib
import subprocess
import sys

ROOT = pathlib.Path("/Users/tom/Developer/ablative/stack/beamr")
ETS = ROOT / "crates/beamr/src/native/ets_bifs.rs"
SYS = ROOT / "crates/beamr/src/native/system_info_bifs.rs"
PI = ROOT / "crates/beamr/src/native/process_info_bifs.rs"

WALLS = {
    "ets": "native::ets_bifs::tests::info_one_reserve_covers_every_allocation_exactly",
    "sys": "native::system_info_bifs::tests::memory_zero_reserve_covers_every_allocation_exactly",
    "pi": "native::process_info_bifs::tests::process_info_reserve_covers_every_allocation_exactly",
}

ARMS = [
    ("F-ets", ETS, "    item_count * 5\n", "    item_count * 5 - 1\n", "ets"),
    ("F-sys", SYS, "    item_count * 5\n", "    item_count * 5 - 1\n", "sys"),
    ("F-pi", PI, "                * 5\n", "                * 4\n", "pi"),
]


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def run_walls():
    r = subprocess.run(
        ["cargo", "test", "-p", "beamr", "--features", "encode",
         "reserve_covers_every_allocation_exactly"],
        cwd=ROOT, capture_output=True, text=True)
    return r.returncode, r.stdout + r.stderr


failures = []

print("=== baseline: all three walls green at the fixed bytes ===")
rc, out = run_walls()
ok = rc == 0 and all(f"{w} ... ok" in out for w in WALLS.values())
print(f"{'PASS' if ok else 'WRONG'}  baseline: rc={rc}")
if not ok:
    print(out[-2000:])
    sys.exit(1)

for tag, path, needle, replacement, victim in ARMS:
    print(f"\n=== {tag}: {path.name} {needle.strip()!r} -> {replacement.strip()!r} ===")
    orig = path.read_bytes()
    pre = sha(path)
    src = path.read_text()
    n = src.count(needle)
    if n != 1:
        print(f"ABORT {tag}: mutation target matched {n} times, need exactly 1")
        sys.exit(1)
    try:
        path.write_text(src.replace(needle, replacement))
        rc, out = run_walls()
        died = rc != 0 and f"{WALLS[victim]} ... FAILED" in out
        others_ok = all(f"{WALLS[w]} ... ok" in out for w in WALLS if w != victim)
        ok = died and others_ok
        print(f"{'DIED' if ok else 'WRONG'}  {tag}: rc={rc}; victim "
              f"{'FAILED' if died else 'DID NOT FAIL'}; other walls "
              f"{'ok' if others_ok else 'NOT ok'}")
        for line in out.splitlines():
            if "consumed" in line and "reserved" in line:
                print(f"       {line.strip()}")
        if not ok:
            failures.append(tag)
    finally:
        path.write_bytes(orig)
    if sha(path) != pre:
        print(f"ABORT: {path.name} restore is not byte-identical")
        sys.exit(1)
    print(f"       restore sha-verified {pre[:16]}")

print("\n=== post: unmutated tree green again ===")
rc, out = run_walls()
ok = rc == 0 and all(f"{w} ... ok" in out for w in WALLS.values())
print(f"{'PASS' if ok else 'WRONG'}  post: rc={rc}")
if not ok:
    failures.append("post")

if failures:
    print(f"\nARMS NOT DYING AS EXPECTED: {failures}")
    sys.exit(1)
print("\nALL THREE ARITHMETICS: wall red on subtract-one-word, restores "
      "sha-verified, tree green before and after.")
sys.exit(0)
