"""Mutation test for shape_hunt_controls.py.

⭐ A SUITE THAT ONLY EVER PRINTS PASS IS INDISTINGUISHABLE FROM ONE THAT ALWAYS
PRINTS PASS. The controls assert the labeller is right; this asserts the
CONTROLS CAN TELL. Each mutation breaks one mechanism of cfg_test_lines in a
copy of the file; at least one arm must go red, or that mechanism is untested.

Cally Ray ran this shape by hand against `4fcc9a7` and it found a real hole:
removing raw-string handling left the suite at 8/8. Both D and E were VACUOUSLY
GREEN -- written as direct checks, but their inputs could not separate intact
from mutated:

  E as written  r#"{{{ x }}}"#   contains no `"` inside the raw string, so the
                                 PLAIN-string scanner skips the same span. It
                                 tested that *something* skips that text.
  D as written  '{' then '}'     on consecutive lines: without char handling the
                                 walk counts both as structure, +1 then -1, and
                                 lands back on depth 0. ⭐ THE CASE INSTANTIATED
                                 THE EXACT FAILURE MODE ITS OWN COMMENT WARNED
                                 ABOUT -- offsets that cancel.

⭐⭐ "CHECKED DIRECTLY RATHER THAN TRUSTED TO THE DEPTH-0 GREEN" IS A PROPERTY
OF THE INPUT, NOT OF THE INTENTION. Both arms carried the label and neither had
the property. The discriminating inputs now in CASES were derived by asking what
input SEPARATES intact from mutated, which is the only question that settles it.

Two controls on this harness itself:
  * M0, unmutated, must PASS -- otherwise the harness is stuck-red and every
    "caught" verdict below is meaningless.
  * every mutation must actually CHANGE the source text. A no-op mutation that
    "passes" proves nothing, so it is reported as MUTATION-NOOP, never as a
    caught or missed result.

Run:  python3 shape_hunt_mutations.py     (rc 0 = every mutation caught)
"""
import pathlib
import shutil
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).parent
HUNT = HERE / "shape_hunt.py"
CONTROLS = HERE / "shape_hunt_controls.py"

# (name, what it breaks, literal to find, replacement)
#
# LITERALS, not regexes, and each must appear EXACTLY ONCE. The first draft of
# this file used a regex for M1 and it matched the WRONG `gated.add(lineno)` --
# the body-gating one at the top of the loop rather than the opener fix inside
# the `{` branch. It reported a verdict for a mutation it had not made.
# ⭐ A MUTATION THAT LANDS SOMEWHERE ELSE STILL PRODUCES A ROW IN THE TABLE.
# Hence the uniqueness assert below: ambiguous or absent literals are reported,
# never applied silently.
MUTATIONS = [
    ("M1_drop_opener_gate", "the opener-line fix (arm A2's defect)",
     "                    gated.add(lineno)\n", ""),
    ("M2_gate_everything", "the cfg-attribute test -- gate indiscriminately",
     "        if line.strip().startswith('#[cfg(test)]'):", "        if True:"),
    ("M3_no_block_comment", "block-comment skipping",
     "            if line.startswith('/*', i):",
     "            if False and line.startswith('/*', i):"),
    ("M4_no_raw_string", "raw-string skipping",
     "            m = re.match(r'r(#*)\"', line[i:])", "            m = None"),
    ("M5_no_char_literal", "char-literal handling",
     "                m = re.match(r\"'(\\\\.|[^\\\\'])'\", line[i:])",
     "                m = None"),
    ("M6_no_plain_string", "plain-string skipping",
     "            if c == '\"':", "            if False:"),
]


def run_suite(workdir):
    """Return (rc, red_arm_names, crashed).

    `crashed` matters: a mutation that makes the script raise is NOT the suite
    discriminating -- it is the file being broken. Counting a traceback as
    "caught" would credit the controls with a catch they did not make.
    """
    proc = subprocess.run([sys.executable, str(workdir / CONTROLS.name)],
                          capture_output=True, text=True)
    arms = sorted({line.split()[1] for line in proc.stdout.splitlines()
                   if line.startswith("  FAIL  ") and len(line.split()) > 1})
    return proc.returncode, arms, bool(proc.stderr.strip())


def main():
    root = pathlib.Path(tempfile.mkdtemp())
    hunt_src = HUNT.read_text()

    # ---- M0: control of the control -----------------------------------------
    base = root / "m0"
    base.mkdir()
    shutil.copy(HUNT, base / HUNT.name)
    shutil.copy(CONTROLS, base / CONTROLS.name)
    rc0, _, crashed0 = run_suite(base)
    print(f"  {'PASS' if rc0 == 0 else 'FAIL'}  M0_unmutated           "
          f"suite rc={rc0}"
          f"{'' if rc0 == 0 else '  <- HARNESS IS STUCK-RED, results below are void'}")
    if rc0 != 0 or crashed0:
        print("\nAborting: an unmutated copy must pass cleanly before any "
              "mutation result means anything.")
        return 2

    missed, unusable = [], []
    for name, breaks, literal, repl in MUTATIONS:
        occurrences = hunt_src.count(literal)
        if occurrences != 1:
            unusable.append((name, f"literal appears {occurrences}x, need exactly 1"))
            print(f"  ????  {name:22s} MUTATION-UNUSABLE -- literal appears "
                  f"{occurrences}x; this row proves nothing")
            continue

        work = root / name
        work.mkdir()
        (work / HUNT.name).write_text(hunt_src.replace(literal, repl, 1))
        shutil.copy(CONTROLS, work / CONTROLS.name)

        rc, arms, crashed = run_suite(work)
        if crashed:
            unusable.append((name, "mutated file raised; not a discrimination"))
            print(f"  ????  {name:22s} CRASHED -- the mutation broke the file "
                  f"rather than the suite catching it; proves nothing")
            continue

        caught = rc != 0
        if not caught:
            missed.append((name, breaks))
        print(f"  {'PASS' if caught else 'FAIL'}  {name:22s} "
              f"{'caught by ' + ', '.join(arms) if caught else 'NOT CAUGHT'}"
              f"   (breaks: {breaks})")

    print(f"\n{len(MUTATIONS) - len(missed) - len(unusable)}/{len(MUTATIONS)} "
          f"mutations caught; {len(unusable)} unusable.")
    for name, breaks in missed:
        print(f"  MISSED {name}: nothing in the suite depends on {breaks}")
    for name, why in unusable:
        print(f"  UNUSABLE {name}: {why}")
    return 1 if (missed or unusable) else 0


if __name__ == "__main__":
    sys.exit(main())
