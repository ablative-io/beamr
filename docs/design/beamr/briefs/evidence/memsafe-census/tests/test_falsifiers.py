import collections
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[7]
EVIDENCE = pathlib.Path(__file__).resolve().parents[1]
BASE = "43d87819ed37515695a697b7416d47c6740502e2"
RF006 = ROOT / "docs/design/beamr/briefs/evidence/review-23-07/rf-006/sweep"
SHAPE_HUNT = ROOT / "docs/design/beamr/briefs/evidence/accumulator-rooting/shape_hunt.py"


def git(*args, check=True):
    return subprocess.run(["git", *args], cwd=ROOT, text=True, capture_output=True, check=check)


def ancestor(a, b):
    return git("merge-base", "--is-ancestor", a, b, check=False).returncode == 0


def key(row):
    return tuple(row[field] for field in ("file", "fn", "var", "call_name"))


def test_c2_intro_is_first_and_fix_modifies_its_line():
    first = git("log", "--all", "--reverse", "-Sentries: DashMap<EtsKey, Term>", "--format=%H", "--", "crates/beamr/src/ets/set.rs").stdout.splitlines()[0]
    assert first.startswith("487aae5")
    assert "crates/beamr/src/ets/set.rs" in git("show", "--format=", "--name-only", "e9565d6").stdout
    assert "crates/beamr/src/ets/set.rs" not in git("show", "--format=", "--name-only", "4310e4a").stdout


def test_c1_intro_is_first_and_fix_modifies_its_line():
    first = git("log", "--all", "--reverse", "-Sallocations: Vec<(usize, usize)>", "--format=%H", "--", "crates/beamr/src/process/heap.rs").stdout.splitlines()[0]
    assert first.startswith("2c064ed")
    assert "crates/beamr/src/process/heap.rs" in git("show", "--format=", "--name-only", "39a6dcc").stdout


def test_intro_keys_absent_then_present():
    c1 = "allocations: Vec<(usize, usize)>"
    c2 = "entries: DashMap<EtsKey, Term>"
    for tag in ("v0.2.0", "v0.3.15"):
        assert not git("grep", "-F", c1, tag, "--", "crates/beamr/src", check=False).stdout
        assert not git("grep", "-F", c2, tag, "--", "crates/beamr/src", check=False).stdout
    assert git("grep", "-F", c1, "v0.4.0", "--", "crates/beamr/src", check=False).stdout
    assert git("grep", "-F", c2, "v0.4.0", "--", "crates/beamr/src", check=False).stdout
    assert not git("grep", "-F", c1, "v0.16.3", "--", "crates/beamr/src", check=False).stdout
    assert not git("grep", "-F", c2, "v0.16.3", "--", "crates/beamr/src", check=False).stdout


def test_tag_algebra():
    tags = set(git("tag").stdout.splitlines())
    c1 = set(git("tag", "--contains", "2c064ed").stdout.splitlines())
    c2 = set(git("tag", "--contains", "487aae5").stdout.splitlines())
    fixed = set(git("tag", "--contains", "67f89c4").stdout.splitlines())
    assert len((c1 & c2) - fixed) == 33
    assert len(tags - (c1 | c2)) == 18
    assert len(fixed) == 17
    assert len({tag for tag in fixed if tag.startswith("v") or tag.startswith("beamr-v")}) == 12


def test_registry_untagged_split():
    rows = {row["version"]: row for row in json.loads((EVIDENCE / "registry-population.json").read_text())["versions"]}
    for version in ("0.15.3", "0.15.4", "0.16.0", "0.16.1"):
        assert rows[version]["c1"] and rows[version]["c2"] and not rows[version]["fix"] and rows[version]["affected"]
    assert rows["0.16.2"]["fix"] and not rows["0.16.2"]["affected"]


def run_shape_hunt():
    return subprocess.run([sys.executable, str(SHAPE_HUNT)], cwd=ROOT, capture_output=True, text=True)


def test_ar1_controls_refound_by_own_instrument():
    result = run_shape_hunt()
    rows = json.loads((ROOT / "docs/design/beamr/briefs/evidence/accumulator-rooting/dispositions.json").read_text())["sites"]
    controls = [row for row in rows if row.get("disposition") == "CONTROL-FIXTURE"]
    assert result.returncode == 0, result.stdout + result.stderr
    assert len(controls) == 5
    for marker in ("S3a", "S3b", "S3c", "S3d", "S3e"):
        assert f"PASS  {marker}" in result.stdout
    assert "ALL 5 CLASS CONTROLS PASS" in result.stdout


def test_ar1_shape_hunt_receipt_matches_live_tail():
    receipt = (EVIDENCE / "r6c-arm1-shape-hunt.txt").read_text()
    result = run_shape_hunt()
    assert result.returncode == 0
    assert "returncode: 0" in receipt
    assert result.stdout.strip().splitlines()[-1] in receipt


def test_rf006_blind_to_cfg_test_controls():
    data = json.loads((EVIDENCE / "jit-candidates-main.json").read_text())
    rows = data["candidates"] if isinstance(data, dict) else data
    assert not [row for row in rows if row["file"] == "crates/beamr/src/ar1_shape_control.rs"]
    result = run_shape_hunt()
    assert result.returncode == 0
    assert "ALL 5 CLASS CONTROLS PASS" in result.stdout


def test_r6c_arm3_carrier_receipt():
    data = json.loads((EVIDENCE / "r6c-arm3-carrier.json").read_text())
    assert data["with_carrier"]["candidate_count"] - data["without_carrier"]["candidate_count"] == 1
    flagged = data["flagged_row"]
    assert flagged["file"] == data["carrier_file"]
    assert key(flagged) not in {key(row) for row in data["without_carrier"]["candidates"]}
    assert data["with_carrier"]["stage1_returncode"] == data["with_carrier"]["stage3_returncode"] == 0
    assert data["without_carrier"]["stage1_returncode"] == data["without_carrier"]["stage3_returncode"] == 0


def test_no_regrade_in_r4_artefacts():
    crossings_text = (EVIDENCE / "jit-crossings.json").read_text()
    instrument_text = (EVIDENCE / "instruments/jit_sweep.py").read_text()
    assert "regraded_to_SAFE" not in crossings_text
    assert "regraded_to_SAFE" not in instrument_text
    data = json.loads(crossings_text)
    rulings = {key(row) for row in json.loads((EVIDENCE / "jit-rulings.json").read_text())["rulings"]}
    for arm in ("arm_p", "arm_w"):
        for row in data[arm]["crossings"]:
            if row.get("prior_verdict") is not None and row["verdict"] != row["prior_verdict"]:
                assert key(row) in rulings


def test_r4_tally_is_derived_from_rows():
    data = json.loads((EVIDENCE / "jit-crossings.json").read_text())
    for arm in ("arm_p", "arm_w"):
        recomputed = dict(collections.Counter(row["verdict"] for row in data[arm]["crossings"]))
        assert recomputed == data[arm]["verdict_tally"]


def test_r4_every_bounded_row_is_ruled():
    data = json.loads((EVIDENCE / "jit-crossings.json").read_text())
    rulings = {key(row): row for row in json.loads((EVIDENCE / "jit-rulings.json").read_text())["rulings"]}
    for row in data["arm_p"]["crossings"]:
        ruling = rulings[key(row)]
        assert ruling["reason"]
        lines = ruling["lines_read"]
        assert all(name in lines for name in ("bind", "call", "use"))


def test_r4_real_rows_are_findings():
    data = json.loads((EVIDENCE / "jit-crossings.json").read_text())
    real = {
        key(row)
        for arm in ("arm_p", "arm_w")
        for row in data[arm]["crossings"]
        if row["verdict"] in {"REAL", "REAL-OSIRIS"}
    }
    assert real == {key(row) for row in data["findings"]}
