import collections
import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[7]
EVIDENCE = pathlib.Path(__file__).resolve().parents[1]
BASE = "43d87819ed37515695a697b7416d47c6740502e2"


def git(*args, check=True):
    return subprocess.run(["git", *args], cwd=ROOT, text=True, capture_output=True, check=check)


def test_unsafe_population_and_control():
    unsafe = git("grep", "-n", "-w", "unsafe", BASE, "--", "crates/beamr/src").stdout.splitlines()
    control = git("grep", "-n", "-w", "from_raw_parts", BASE, "--", "crates/beamr/src").stdout.splitlines()
    files = list((ROOT / "crates/beamr/src").rglob("*.rs"))
    assert (len(unsafe), len(control), len(files)) == (200, 5, 327)


def test_site_kinds_partition():
    rows = json.loads((EVIDENCE / "unsafe-sites.json").read_text())["sites"]
    counts = {kind: sum(row["kind"] == kind for row in rows) for kind in ("block", "fn", "impl", "extern", "prose")}
    assert len(rows) == 200
    assert counts == {"block": 192, "fn": 2, "impl": 2, "extern": 1, "prose": 3}
    vocabulary = {"C1", "C2", "C3", "RF-006", "AR-1", "NONE"}
    verdicts = {"SAFE-A", "SAFE-C", "SAFE-ROOTED", "SAFE-DETACHED", "SAFE-FIXED", "SAFE-REREAD", "SAFE-OFFHEAP", "FP-D", "FP-E", "FP-F", "REAL", "REAL-OSIRIS", "UNRULED-PRERESERVE"}
    for row in rows:
        assert row["class"] in vocabulary
        if row["class"] in {"RF-006", "C3"}:
            assert row["rf006_verdict"] in verdicts
        else:
            assert row["rf006_verdict"] is None


def test_safety_lookback_counts():
    data = json.loads((EVIDENCE / "unsafe-sites.json").read_text())
    assert data["instrument"]["safety_lookback_lines"] == 4
    rows = [row for row in data["sites"] if row["kind"] != "prose"]
    assert sum(row["safety_comment"] == "present" for row in rows) == 166
    assert sum(row["safety_comment"] == "absent" for row in rows) == 31


def test_region_tally_sums_to_code_sites():
    data = json.loads((EVIDENCE / "unsafe-sites.json").read_text())
    rows = [row for row in data["sites"] if row["kind"] != "prose"]
    prefix = "crates/beamr/src/"
    regions = []
    for row in rows:
        assert row["file"].startswith(prefix)
        relative = row["file"][len(prefix):]
        assert "/" in relative, f"unexpected direct src row: {row['file']}"
        regions.append(relative.split("/", 1)[0])
    recomputed = dict(collections.Counter(regions))
    assert recomputed == data["tallies"]["regions"]
    assert sum(recomputed.values()) == len(rows)


def test_jit_helper_surface_is_44():
    data = json.loads((EVIDENCE / "jit-walk-edges.json").read_text())
    assert len(data["roots"]) == 44
    assert data["naive_string_tokens"] == 45


def test_walk_bound_is_honoured():
    data = json.loads((EVIDENCE / "jit-walk-edges.json").read_text())
    roots = set(data["roots"])
    reached = set(roots)
    for edge in sorted(data["edges"], key=lambda row: row["depth"]):
        assert edge["depth"] <= 4
        assert edge["from"] in reached
        reached.add(edge["to"])
        if edge.get("terminated"):
            assert edge.get("stopped_at")


def test_r4_raw_candidates_are_auditable():
    data = json.loads((EVIDENCE / "jit-candidates-main.json").read_text())
    rows = data["candidates"] if isinstance(data, dict) else data
    assert len(rows) == 74
    assert sum(row["file"].startswith("crates/beamr/src/") for row in rows) == 57


def test_astgrep_pack_tally():
    data = json.loads((EVIDENCE / "reconciliation.json").read_text())
    expected = {"mod-rs-declarations-only": 1054, "no-let-underscore-on-results": 277, "no-lint-bypass-attributes": 16, "no-std-mutex-in-async": 0}
    assert data["pack_rules"] == expected
    assert sum(expected.values()) == 1347
    assert data["unused_suppression"] not in expected.values()


def test_let_underscore_product_count():
    data = json.loads((EVIDENCE / "reconciliation.json").read_text())
    assert (data["let_underscore_raw"], data["let_underscore_fixture"], data["let_underscore_product"]) == (277, 79, 198)


def test_advisory_phrase_cleared():
    result = git("grep", "-c", "had their introduction point", "--", "README.md", "CHANGELOG.md", check=False)
    assert result.returncode == 1 and not result.stdout
