#!/bin/sh
# Keep the README and CHANGELOG advisory facts coherent with the beamr manifest
# and with forward-only Fixed records. All fixtures used by --self-test are
# minted under the run-local TMPDIR; the repository files are never rewritten.
set -u

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
exec python3 - "$ROOT" "$@" <<'PY'
from __future__ import annotations

import re
import shutil
import sys
import tempfile
from pathlib import Path

ROOT = Path(sys.argv[1])
ARGS = sys.argv[2:]
DECLARED = (
    "asbytes-0.16.3",
    "gc-refcount-0.16.2",
    "ets-borrow-0.16.2",
    "rf006-jit-rooting",
    "ar1-accumulator",
    "jit-message-drop",
    "jit-threads-coupling",
)
MARKER_RE = re.compile(
    r"^<!-- class: ([a-z0-9.-]+) status: (fixed|open) fixed_in: "
    r"([0-9]+\.[0-9]+\.[0-9]+|-) -->$"
)
RECORD_RE = re.compile(
    r"^<!-- fixed: ([a-z0-9.-]+) in ([0-9]+\.[0-9]+\.[0-9]+)"
    r"( commit [0-9a-f]{7,40})? -->$"
)
RELEASE_RE = re.compile(r"^<!-- release: ([^ ]+) -->$")
VERSION_RE = re.compile(r"\b0\.[0-9]+\.[0-9]+\b")
LITERAL_FIXED_RE = re.compile(r"(?:FIXED IN|Fixed in) `([0-9]+\.[0-9]+\.[0-9]+)`")


def emit(kind: str, arm: str, message: str) -> None:
    print(f"{kind} arm ({arm}): {message}")


def read_lines(path: Path) -> list[str] | None:
    try:
        return path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeError):
        return None


def advisory_region(lines: list[str], which: str) -> tuple[int, int] | None:
    if which == "README":
        start = next((i for i, line in enumerate(lines) if re.match(r"^## .*[Aa]dvisory", line)), None)
        if start is None:
            return None
        end = next((i for i in range(start + 1, len(lines)) if lines[i].startswith("## ")), len(lines))
        return start, end
    start = next((i for i, line in enumerate(lines) if line.startswith("## Advisory")), None)
    if start is None:
        return None
    end = next(
        (i for i in range(start + 1, len(lines)) if lines[i].startswith("## ") and not lines[i].startswith("## Advisory")),
        len(lines),
    )
    return start, end


def unfenced(lines: list[str], start: int, end: int) -> list[tuple[int, str]]:
    result: list[tuple[int, str]] = []
    fenced = False
    for i in range(start, end):
        line = lines[i]
        if line.startswith("```"):
            fenced = not fenced
            continue
        if not fenced:
            result.append((i, line))
    return result


def version_header_exists(lines: list[str], version: str) -> bool:
    pattern = re.compile(r"^## " + re.escape(version) + r"(?:$|[^0-9.])")
    return any(pattern.match(line) for line in lines)


def fixed_records(lines: list[str]) -> tuple[list[tuple[int, str, str]], list[tuple[int, str]]]:
    records: list[tuple[int, str, str]] = []
    malformed: list[tuple[int, str]] = []
    under_fixed = False
    for i, line in enumerate(lines):
        if line.startswith("## ") or line.startswith("### "):
            under_fixed = line.startswith("### Fixed")
        match = RECORD_RE.match(line)
        if match:
            if under_fixed:
                records.append((i, match.group(1), match.group(2)))
            else:
                malformed.append((i, line))
        elif line.startswith("<!-- fixed:"):
            malformed.append((i, line))
    return records, malformed


def paragraphs(scanned: list[tuple[int, str]]) -> list[list[tuple[int, str]]]:
    blocks: list[list[tuple[int, str]]] = []
    block: list[tuple[int, str]] = []
    for item in scanned:
        if item[1] == "":
            if block:
                blocks.append(block)
                block = []
        else:
            block.append(item)
    if block:
        blocks.append(block)
    return blocks


def run_gate(root: Path) -> int:
    readme_path = root / "README.md"
    changelog_path = root / "CHANGELOG.md"
    manifest_path = root / "crates/beamr/Cargo.toml"
    readme = read_lines(readme_path)
    changelog = read_lines(changelog_path)
    manifest = read_lines(manifest_path)
    if readme is None or changelog is None or manifest is None:
        emit("REFUSE", "e", "README.md, CHANGELOG.md, or crates/beamr/Cargo.toml is unreadable")
        return 3

    readme_bounds = advisory_region(readme, "README")
    changelog_bounds = advisory_region(changelog, "CHANGELOG")
    if readme_bounds is None or changelog_bounds is None:
        emit("REFUSE", "e", "an advisory region was not found")
        return 3

    readme_scan = unfenced(readme, *readme_bounds)
    changelog_scan = unfenced(changelog, *changelog_bounds)
    scans = {"README.md": readme_scan, "CHANGELOG.md": changelog_scan}
    markers: dict[str, list[tuple[str, int, str, str, str]]] = {name: [] for name in scans}
    malformed_markers: list[tuple[str, int, str]] = []
    for name, scan in scans.items():
        for i, line in scan:
            match = MARKER_RE.match(line)
            if match:
                markers[name].append((name, i, match.group(1), match.group(2), match.group(3)))
            elif line.startswith("<!-- class:"):
                malformed_markers.append((name, i, line))
    for name in scans:
        if not markers[name]:
            emit("REFUSE", "e", f"{name} advisory region holds zero markers")
            return 3

    failures = 0

    package = False
    manifest_version = None
    for line in manifest:
        if line == "[package]":
            package = True
            continue
        if package and line.startswith("["):
            break
        if package:
            match = re.match(r'^version\s*=\s*"([^"]+)"\s*$', line)
            if match:
                manifest_version = match.group(1)
                break
    releases = [(i, m.group(1), line) for i, line in enumerate(readme) if (m := RELEASE_RE.match(line))]
    if manifest_version is None:
        emit("RED", "a", "crates/beamr/Cargo.toml [package] has no readable version")
        failures += 1
    if len(releases) != 1:
        emit("RED", "a", f"README.md carries {len(releases)} whole-line release markers, expected exactly 1")
        failures += 1
    elif manifest_version is not None and releases[0][1] != manifest_version:
        i, value, line = releases[0]
        emit("RED", "a", f"README.md:{i + 1} read {line!r}; manifest version is {manifest_version!r}")
        failures += 1

    records, malformed_records = fixed_records(changelog)
    record_pairs = {(class_id, version) for _, class_id, version in records}
    all_record_ids = {class_id for _, class_id, _ in records}
    for name, i, line in malformed_markers:
        emit("RED", "b", f"{name}:{i + 1} malformed marker {line!r}")
        failures += 1
    for i, line in malformed_records:
        emit("RED", "b", f"CHANGELOG.md:{i + 1} fixed-record is malformed or not under a Fixed header: {line!r}")
        failures += 1

    all_markers = markers["README.md"] + markers["CHANGELOG.md"]
    by_id: dict[str, list[tuple[str, int, str, str, str]]] = {}
    for marker in all_markers:
        name, i, class_id, status, fixed_in = marker
        by_id.setdefault(class_id, []).append(marker)
        lines = readme if name == "README.md" else changelog
        above_blank = i > 0 and lines[i - 1] == ""
        below_blank = i + 1 < len(lines) and lines[i + 1] == ""
        if not above_blank or not below_blank:
            emit("RED", "b", f"{name}:{i + 1} marker lacks a blank line above and below: {lines[i]!r}")
            failures += 1
        if class_id not in DECLARED:
            emit("RED", "b", f"{name}:{i + 1} undeclared class id {class_id!r}")
            failures += 1
        if status == "open" and fixed_in != "-":
            emit("RED", "b", f"{name}:{i + 1} open marker carries fixed_in {fixed_in!r}: {lines[i]!r}")
            failures += 1
        if status == "fixed" and fixed_in == "-":
            emit("RED", "b", f"{name}:{i + 1} fixed marker carries fixed_in '-': {lines[i]!r}")
            failures += 1
        if status == "fixed" and fixed_in != "-":
            if not version_header_exists(changelog, fixed_in):
                emit("RED", "b", f"{name}:{i + 1} fixed_in {fixed_in!r} has no exact CHANGELOG version header")
                failures += 1
            if (class_id, fixed_in) not in record_pairs:
                emit("RED", "b", f"{name}:{i + 1} fixed marker has no matching Fixed record: {lines[i]!r}")
                failures += 1
        if status == "open" and class_id in all_record_ids:
            emit("RED", "b", f"{name}:{i + 1} open marker has a Fixed record: {lines[i]!r}")
            failures += 1

    changelog_ids = {marker[2] for marker in markers["CHANGELOG.md"]}
    for class_id in DECLARED:
        if class_id not in changelog_ids:
            emit("RED", "b", f"CHANGELOG advisory region is missing declared class id {class_id!r}")
            failures += 1
    for class_id, same_id in by_id.items():
        states = {(marker[3], marker[4]) for marker in same_id}
        if len(states) != 1:
            where = ", ".join(f"{m[0]}:{m[1] + 1}={m[3]}/{m[4]}" for m in same_id)
            emit("RED", "b", f"class {class_id!r} disagrees across markers: {where}")
            failures += 1

    for name, scan in scans.items():
        for i, line in scan:
            for match in LITERAL_FIXED_RE.finditer(line):
                version = match.group(1)
                if not version_header_exists(changelog, version):
                    emit("RED", "c", f"{name}:{i + 1} read {match.group(0)!r} without an exact CHANGELOG version header")
                    failures += 1

    for name, scan in scans.items():
        active = False
        for block in paragraphs(scan):
            first_i, first_line = block[0]
            if len(block) == 1 and MARKER_RE.match(first_line):
                active = True
                continue
            if first_line.startswith("#"):
                active = False
                continue
            if len(block) == 1 and RECORD_RE.match(first_line):
                continue
            text = " ".join(line for _, line in block)
            if VERSION_RE.search(text) and not active:
                quote = text.encode("utf-8")[:80].decode("utf-8", "ignore")
                emit("RED", "d", f"{name}:{first_i + 1} unmarked version-bearing paragraph {quote!r}")
                failures += 1

    if failures:
        print(f"docs-truth: {failures} measured red(s)")
        return 1
    print("docs-truth: PASS — release, class markers, Fixed records, and version-bearing paragraph scopes agree")
    return 0


def green_files(root: Path) -> None:
    (root / "crates/beamr").mkdir(parents=True)
    (root / "crates/beamr/Cargo.toml").write_text('[package]\nname = "beamr"\nversion = "1.2.3"\n', encoding="utf-8")
    (root / "README.md").write_text(
        "# fixture\n\n<!-- release: 1.2.3 -->\n\n## Security advisory\n\n"
        "<!-- class: jit-message-drop status: fixed fixed_in: 0.18.2 -->\n\n"
        "The fixed line points to `0.18.2`.\n\n## End\n",
        encoding="utf-8",
    )
    marker_rows = [
        ("asbytes-0.16.3", "fixed", "0.16.3"),
        ("gc-refcount-0.16.2", "fixed", "0.16.2"),
        ("ets-borrow-0.16.2", "fixed", "0.16.2"),
        ("rf006-jit-rooting", "fixed", "0.18.1"),
        ("ar1-accumulator", "fixed", "0.19.0"),
        ("jit-message-drop", "fixed", "0.18.2"),
        ("jit-threads-coupling", "open", "-"),
    ]
    advisory = ["# Changelog", "", "## Advisory fixture", ""]
    for class_id, status, version in marker_rows:
        advisory += [f"<!-- class: {class_id} status: {status} fixed_in: {version} -->", ""]
        if version == "-":
            advisory += ["The coupling is open.", ""]
        else:
            advisory += [f"The class is Fixed in `{version}`.", ""]
    advisory += ["## Unreleased", "", "### Fixed (record) — fixture", ""]
    records = [
        "<!-- fixed: asbytes-0.16.3 in 0.16.3 -->",
        "<!-- fixed: gc-refcount-0.16.2 in 0.16.2 commit 67f89c4 -->",
        "<!-- fixed: ets-borrow-0.16.2 in 0.16.2 commit 67f89c4 -->",
        "<!-- fixed: rf006-jit-rooting in 0.18.1 -->",
        "<!-- fixed: ar1-accumulator in 0.19.0 commit 1a70068 -->",
        "<!-- fixed: jit-message-drop in 0.18.2 -->",
    ]
    advisory += sum(([record, "fixture prose", ""] for record in records), [])
    for version in ("0.16.2", "0.16.3", "0.18.1", "0.18.2", "0.19.0"):
        advisory += [f"## {version} — fixture", ""]
    (root / "CHANGELOG.md").write_text("\n".join(advisory) + "\n", encoding="utf-8")


def mutate_case(root: Path, name: str) -> None:
    readme = root / "README.md"
    changelog = root / "CHANGELOG.md"
    if name == "a-mismatch":
        readme.write_text(readme.read_text().replace("<!-- release: 1.2.3 -->", "<!-- release: 0.17.0 -->"))
    elif name == "b-fixed-without-record":
        changelog.write_text(changelog.read_text().replace("<!-- fixed: rf006-jit-rooting in 0.18.1 -->\nfixture prose\n\n", ""))
    elif name == "b-open-with-record":
        changelog.write_text(changelog.read_text().replace("### Fixed (record) — fixture\n\n", "### Fixed (record) — fixture\n\n<!-- fixed: jit-threads-coupling in 0.20.0 -->\nfixture prose\n\n"))
    elif name == "b-open-with-version":
        changelog.write_text(changelog.read_text().replace("<!-- class: jit-threads-coupling status: open fixed_in: - -->", "<!-- class: jit-threads-coupling status: open fixed_in: 0.20.0 -->"))
    elif name == "b-fixed-with-dash":
        changelog.write_text(changelog.read_text().replace("<!-- class: rf006-jit-rooting status: fixed fixed_in: 0.18.1 -->", "<!-- class: rf006-jit-rooting status: fixed fixed_in: - -->"))
    elif name == "d-unmarked-paragraph":
        readme.write_text(readme.read_text().replace("## Security advisory\n\n", "## Security advisory\n\nUnmarked version `0.1.0`.\n\n"))
    elif name == "e-missing-file":
        readme.unlink()
    elif name == "e-zero-markers":
        readme.write_text(readme.read_text().replace("<!-- class: jit-message-drop status: fixed fixed_in: 0.18.2 -->\n\n", ""))
    elif name != "green":
        raise ValueError(name)


def self_test() -> int:
    cases = [
        ("a-mismatch", 1),
        ("b-fixed-without-record", 1),
        ("b-open-with-record", 1),
        ("b-open-with-version", 1),
        ("b-fixed-with-dash", 1),
        ("d-unmarked-paragraph", 1),
        ("green", 0),
        ("e-missing-file", 3),
        ("e-zero-markers", 3),
    ]
    pass_count = 0
    temp_parent = Path(tempfile.mkdtemp(prefix="docs-truth.", dir=str(Path.cwd().parent / "tmp")))
    try:
        for name, expected in cases:
            case_root = temp_parent / name
            green_files(case_root)
            mutate_case(case_root, name)
            got = run_gate(case_root)
            if got == expected:
                pass_count += 1
                print(f"SELF-TEST PASS {name}: rc {got}")
            else:
                print(f"SELF-TEST FAIL {name}: expected rc {expected}, got {got}")
        print(f"{pass_count}/{len(cases)}")
        return 0 if pass_count == len(cases) else 1
    finally:
        shutil.rmtree(temp_parent)


if ARGS == ["--self-test"]:
    raise SystemExit(self_test())
if len(ARGS) == 2 and ARGS[0] == "--root":
    raise SystemExit(run_gate(Path(ARGS[1])))
if ARGS:
    print("usage: gate-docs-truth.sh [--self-test | --root FIXTURE]", file=sys.stderr)
    raise SystemExit(3)
raise SystemExit(run_gate(ROOT))
PY
