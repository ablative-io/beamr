#!/usr/bin/python3
# The shebang pins the SYSTEM python3 deliberately: on the release box,
# `/usr/bin/env python3` resolves to a broken ~/.local shim (rc 126). That
# failure is at least fail-safe under release.sh's `set -e`, but a gate that
# cannot run is a gate that cannot pass, so pin the interpreter that works.
# Stdlib only; no third-party imports, so the system python suffices.
"""release-obligations-gate.py -- the release-obligations gate (pilot).

Ruled properties (ablative/docs tracking/ruling-release-obligations-20260809.md,
commit 1369278), each load-bearing in this file:

  1. IN THE REPO: grades RELEASE-OBLIGATIONS.json at the repo root.
  2. ON THE PATH: scripts/release.sh runs this before any publish (dry-run
     included); rc != 0 aborts the run under `set -e`.
  3. THE GATE REFUSES -- IT DOES NOT LIST. A due, unresolved obligation is a
     non-zero exit. There is no warn-only or list-only mode, on purpose: a red
     is the only thing that reliably reaches a person; a list gets summarised
     by whoever is reporting.
  4. THE REASON IS REQUIRED. An open obligation with no reason_unresolved is
     MALFORMED and reds -- the failure mode is not forgetting, it is writing a
     good justification somewhere the act cannot read.
  5. ABSENCE RAISES, NEVER SKIPS. A missing, unreadable, or malformed file
     refuses the release; deleting the file must not also delete its alarm.

Instrument discipline (pattern: scripts/gate-blocking-call.sh): before grading
the real file, the gate proves ON EVERY RUN that it can produce each verdict,
against committed fixtures -- a refusal, a malformed red, and a clean pass. A
control failing exits 3 (instrument broken), which also reds the release. No
stderr is suppressed anywhere.

Exit codes:
  0  no obligation is due-and-open; release may proceed
  1  REFUSED: at least one obligation is due at this version and not resolved
  2  RAISED: obligations file absent, unreadable, or malformed
  3  INSTRUMENT BROKEN: a built-in control did not produce its known verdict

Usage: release-obligations-gate.py [--repo DIR]
The version being cut is read from crates/beamr/Cargo.toml (the same source
scripts/release.sh publishes), so the gate and the act cannot disagree.
"""

import argparse
import json
import pathlib
import re
import sys

DUE_RE = re.compile(r"^(?:next-release|release:(\d+)\.(\d+)\.(\d+))$")


def fail(code, msg):
    print(f"release-obligations-gate: {msg}", file=sys.stderr)
    sys.exit(code)


def parse_version(text, source):
    m = re.match(r"^(\d+)\.(\d+)\.(\d+)$", text.strip())
    if not m:
        fail(2, f"cannot parse version {text!r} from {source}")
    return tuple(int(p) for p in m.groups())


def crate_version(repo):
    manifest = repo / "crates/beamr/Cargo.toml"
    if not manifest.is_file():
        fail(2, f"missing {manifest} -- cannot determine the version being cut")
    for line in manifest.read_text().splitlines():
        if line.startswith("version"):
            m = re.search(r'"([^"]+)"', line)
            if m:
                return parse_version(m.group(1), str(manifest))
    fail(2, f"no version line found in {manifest}")


def grade(doc, cutting):
    """Grade a parsed obligations document against the version being cut.

    Returns (verdict, detail): verdict is 'pass', 'refuse', or 'malformed'.
    Pure so the built-in controls can grade fixtures with the SAME code path
    that grades the real file -- a control through a side door proves nothing.
    """
    if not isinstance(doc, dict) or doc.get("schema") != 1:
        return "malformed", "schema field missing or not 1"
    obligations = doc.get("obligations")
    if not isinstance(obligations, list):
        return "malformed", "obligations is not a list"
    seen, due_open = set(), []
    for i, ob in enumerate(obligations):
        if not isinstance(ob, dict):
            return "malformed", f"obligation #{i} is not an object"
        ident = ob.get("id")
        if not ident or ident in seen:
            return "malformed", f"obligation #{i} has a missing or duplicate id"
        seen.add(ident)
        status = ob.get("status")
        if status not in ("open", "resolved"):
            return "malformed", f"{ident}: status {status!r} is not open|resolved"
        due = ob.get("due", "")
        m = DUE_RE.match(due or "")
        if not m:
            return "malformed", f"{ident}: due {due!r} is not next-release|release:X.Y.Z"
        if status == "open":
            reason = ob.get("reason_unresolved")
            if not isinstance(reason, str) or not reason.strip():
                return (
                    "malformed",
                    f"{ident}: OPEN WITH NO reason_unresolved -- the reason is a"
                    " required field (ruled property 4)",
                )
            is_due = due == "next-release" or cutting >= tuple(
                int(p) for p in m.groups()
            )
            if is_due:
                due_open.append((ident, ob.get("title", "<untitled>")))
        else:
            resolution = ob.get("resolution")
            if not isinstance(resolution, str) or not resolution.strip():
                return "malformed", f"{ident}: resolved with no stated resolution"
    if due_open:
        lines = "; ".join(f"{i}: {t}" for i, t in due_open)
        return "refuse", f"due and unresolved at this cut -- {lines}"
    return "pass", "no obligation is due and open"


# Committed fixtures: the gate proves each verdict reachable EVERY run. Keyed
# on the shape, not on any live obligation, so no repair or resolution in the
# real file can blind them (the R10-a lesson: a control keyed on a live defect
# is destroyed by the repair it exists to survive).
CONTROLS = [
    ("due_open.json", "refuse"),
    ("missing_reason.json", "malformed"),
    ("resolved_ok.json", "pass"),
]


def run_controls(repo):
    fixture_dir = repo / "scripts/release-obligations-fixtures"
    for name, want in CONTROLS:
        path = fixture_dir / name
        if not path.is_file():
            fail(3, f"control fixture {path} is missing -- instrument broken")
        try:
            doc = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError) as err:
            fail(3, f"control fixture {name} unreadable: {err}")
        got, detail = grade(doc, (0, 19, 0))
        if got != want:
            fail(3, f"control {name} graded {got!r} (want {want!r}: {detail})"
                    " -- instrument broken, refusing the release")
        print(f"release-obligations-gate: control {name} -> {got} (expected)")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=None, help="repo root (default: script's parent's parent)")
    args = parser.parse_args()
    repo = pathlib.Path(args.repo) if args.repo else pathlib.Path(__file__).resolve().parent.parent

    run_controls(repo)

    obligations_path = repo / "RELEASE-OBLIGATIONS.json"
    if not obligations_path.is_file():
        fail(2, f"{obligations_path} is ABSENT -- absence raises, never skips"
                " (ruled property 5); restore the file or its history")
    try:
        doc = json.loads(obligations_path.read_text())
    except (OSError, json.JSONDecodeError) as err:
        fail(2, f"{obligations_path} unreadable or not JSON: {err}")

    cutting = crate_version(repo)
    verdict, detail = grade(doc, cutting)
    version = ".".join(str(p) for p in cutting)
    if verdict == "malformed":
        fail(2, f"RELEASE-OBLIGATIONS.json MALFORMED: {detail}")
    if verdict == "refuse":
        fail(1, f"REFUSING release of {version}: {detail}. Resolve the"
                " obligation in RELEASE-OBLIGATIONS.json (state the resolution)"
                " or record a lead's ruling as its new reason -- do not delete it.")
    print(f"release-obligations-gate: PASS for {version} -- {detail}")


if __name__ == "__main__":
    main()
