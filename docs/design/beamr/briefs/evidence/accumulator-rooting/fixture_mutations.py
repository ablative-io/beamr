"""Break each R10 fixture in turn; require its OWN class control to go red.

A green from a control never observed red is a regression test, not a wall.
Every mutation below is a REALISTIC failure of that fixture -- a fmt reflow, a
refactor, a spelling change -- not a synthetic corruption, because the question
is "would this control notice the ways this fixture actually dies?"

Control-of-the-control: M0 runs unmutated and MUST be all-green. If it is not,
the harness is stuck-red and every "catch" below would be a false credit, so
the run aborts rather than reporting.

Restore is verified by sha256 on every iteration, not assumed.
"""
import hashlib
import pathlib
import re
import subprocess
import sys

# Derived from this file's own location -- an absolute path baked into evidence
# is a claim about one machine, and it silently stops being true on any other.
HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[5]
FIXTURE = REPO / "crates/beamr/src/ar1_shape_control.rs"
HUNT = HERE / "shape_hunt.py"

ORIGINAL = FIXTURE.read_text()
SHA = hashlib.sha256(ORIGINAL.encode()).hexdigest()

MUTATIONS = {
    # S3a: the fmt-reflow failure -- the chain splits and `.map(` leaves the
    # binding line. This is the single most likely real death of this fixture.
    "S3a": (
        "        let s3a_mapped: Vec<_> = src.iter().map(|s| heap.alloc_cons(*s, NIL)).collect();",
        "        let s3a_mapped: Vec<_> = src\n            .iter()\n"
        "            .map(|s| heap.alloc_cons(*s, NIL))\n            .collect();",
    ),
    # S3b: deferred initialisation splits the binding from the expression, so
    # `let X = match` is no longer one line. Brace-balanced on purpose: an
    # unbalanced mutation trips the walk's self-check (exit 3) and would test
    # the self-check rather than the class control.
    "S3b": (
        "        let s3b_chosen = match arms {",
        "        let s3b_chosen;\n        s3b_chosen = match arms {",
    ),
    # S3c: the allocation is hoisted off the literal's line. S3c requires ALLOC
    # on the SAME line, so this kills it while leaving the shape recognisable.
    "S3c": (
        "        let s3c_pair = [heap.alloc_cons(head, NIL), head];",
        "        let hoisted = heap.alloc_cons(head, NIL);\n        let s3c_pair = [hoisted, head];",
    ),
    # S3d: a `let` appears on the line, which is exactly what the class is
    # defined to exclude.
    "S3d": (
        "            s3d_acc = heap.alloc_cons(*item, s3d_acc);",
        "            s3d_acc = { let t = heap.alloc_cons(*item, s3d_acc); t };",
    ),
    # S3e: the constructor spelling changes. TUPVEC reads `Vec::new` /
    # `Vec::with_capacity` only.
    "S3e": (
        "        let mut s3e_pairs: Vec<(FixtureTerm, FixtureTerm)> = Vec::with_capacity(items.len());",
        "        let mut s3e_pairs: Vec<(FixtureTerm, FixtureTerm)> = Vec::default();",
    ),
}


def run_hunt():
    r = subprocess.run([sys.executable, str(HUNT)], cwd=REPO,
                       capture_output=True, text=True)
    if r.returncode == 3:
        return "SELFCHECK", r.stdout, r.stderr
    if r.stderr.strip():
        return "TRACEBACK", r.stdout, r.stderr
    red = sorted(re.findall(r"FAIL  (S3\w)", r.stdout))
    if "POPULATION" in r.stdout:
        red.append("POPULATION")
    return red, r.stdout, r.stderr


def restore():
    FIXTURE.write_text(ORIGINAL)
    got = hashlib.sha256(FIXTURE.read_text().encode()).hexdigest()
    if got != SHA:
        print(f"⛔ RESTORE FAILED -- {got} != {SHA}")
        sys.exit(9)


def main():
    print(f"fixture sha256 {SHA[:16]}…  {len(ORIGINAL)} bytes")

    red, out, _ = run_hunt()
    if red != []:
        print(f"⛔ M0 UNMUTATED IS NOT GREEN ({red}) -- harness stuck red, aborting.")
        print(out[-1500:])
        return 9
    print("✅ M0 unmutated: all 5 class controls green -- harness not stuck-red\n")

    verdicts = {}
    for cls, (old, new) in MUTATIONS.items():
        n = ORIGINAL.count(old)
        if n != 1:
            print(f"  {cls}: UNUSABLE -- anchor matched {n} lines, expected exactly 1")
            verdicts[cls] = None
            continue
        FIXTURE.write_text(ORIGINAL.replace(old, new))
        red, out, err = run_hunt()
        restore()
        if red == "TRACEBACK":
            print(f"  {cls}: TRACEBACK, not a catch\n{err[-400:]}")
            verdicts[cls] = None
            continue
        if red == "SELFCHECK":
            print(f"  {cls}: brace self-check fired (exit 3) -- mutation unbalanced braces, "
                  f"so this is not a clean test of the class control")
            verdicts[cls] = None
            continue
        verdicts[cls] = red
        mark = "✅ ONE-TO-ONE" if red == [cls] else ("⚠️ NOT one-to-one" if red else "🔴 NOT CAUGHT")
        print(f"  {cls}: controls that went red = {red or 'none'}   {mark}")

    restore()
    ok = all(v == [c] for c, v in verdicts.items())
    print(f"\n{'✅' if ok else '🔴'} every fixture's death is caught by its own control "
          f"and no other: {ok}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
