#!/usr/bin/env python3
"""Run the RF-006 narrowing at main, apply explicit hand rulings, and emit R4 evidence."""

import argparse
import collections
import json
import pathlib
import re
import subprocess
import sys
import tempfile

KEY_FIELDS = ("file", "fn", "var", "call_name")
REAL_VERDICTS = {"REAL", "REAL-OSIRIS"}


def row_key(row):
    return tuple(row[field] for field in KEY_FIELDS)


def index_functions(root):
    found = {}
    fn_re = re.compile(r"\bfn\s+(\w+)\s*\(")
    call_re = re.compile(r"\b([A-Za-z_]\w*)\s*\(")
    for path in sorted((root / "crates/beamr/src").rglob("*.rs")):
        lines = path.read_text().splitlines()
        for index, line in enumerate(lines):
            match = fn_re.search(line)
            if not match:
                continue
            name = match.group(1)
            depth = 0
            started = False
            body = []
            end = index
            for cursor in range(index, len(lines)):
                body.append(lines[cursor])
                depth += lines[cursor].count("{") - lines[cursor].count("}")
                started = started or "{" in lines[cursor]
                end = cursor
                if started and depth <= 0:
                    break
            calls = sorted(set(call_re.findall("\n".join(body))) - {name})
            found.setdefault(name, []).append(
                {
                    "file": str(path.relative_to(root)),
                    "line": index + 1,
                    "end": end + 1,
                    "calls": calls,
                }
            )
    return found


def build_walk(root):
    dispatch = (root / "crates/beamr/src/jit/compiler/dispatch.rs").read_text()
    roots = list(
        dict.fromkeys(
            re.findall(r",\s*(jit_[A-Za-z0-9_]+)\s+as\s+\*const\s+u8", dispatch, re.S)
        )
    )
    index = index_functions(root)
    edges = []
    reached = set(roots)
    queue = collections.deque((name, 0) for name in roots)
    seen_depth = {name: 0 for name in roots}
    while queue:
        source, depth = queue.popleft()
        if depth >= 4:
            continue
        for definition in index.get(source, [])[:1]:
            for callee in definition["calls"]:
                if callee not in index:
                    continue
                next_depth = depth + 1
                target = index[callee][0]
                stopped = "interpreter" in target["file"] or "scheduler" in target["file"]
                edge = {
                    "from": source,
                    "to": callee,
                    "depth": next_depth,
                    "file": definition["file"],
                    "line": definition["line"],
                }
                if stopped:
                    edge.update(
                        {
                            "terminated": True,
                            "stopped_at": "interpreter/scheduler deopt boundary",
                        }
                    )
                edges.append(edge)
                reached.add(callee)
                if (
                    not stopped
                    and next_depth < 4
                    and (callee not in seen_depth or next_depth < seen_depth[callee])
                ):
                    seen_depth[callee] = next_depth
                    queue.append((callee, next_depth))
    return {
        "instrument": "44 dispatch registrations; syntactic Rust call graph across crates/beamr/src; depth <= 4; stop at interpreter/scheduler",
        "roots": roots,
        "naive_string_tokens": len(re.findall(r"beamr_jit_[a-z0-9_]*", dispatch)),
        "reached_functions": len(reached),
        "reached": sorted(reached),
        "edges": edges,
    }


def run_stage(command, root):
    result = subprocess.run(command, cwd=root, text=True, capture_output=True)
    if result.returncode:
        sys.stderr.write(result.stdout)
        sys.stderr.write(result.stderr)
        raise SystemExit(result.returncode)
    return result


def load_rows(path, key):
    data = json.loads(path.read_text())
    if isinstance(data, dict):
        return data[key]
    return data


def apply_rulings(rows, rulings, prior_by_key):
    ruled = []
    for candidate in rows:
        candidate_key = row_key(candidate)
        ruling = rulings.get(candidate_key)
        if ruling is None:
            print(f"missing ruling: {candidate_key}", file=sys.stderr)
            raise SystemExit(2)
        row = dict(candidate)
        prior = prior_by_key.get(candidate_key)
        row["prior_verdict"] = prior.get("verdict") if prior else None
        row["verdict"] = ruling["verdict"]
        row["reason"] = ruling["reason"]
        row["lines_read"] = ruling["lines_read"]
        ruled.append(row)
    return ruled


def arm(rows):
    return {
        "population": len(rows),
        "verdict_tally": dict(sorted(collections.Counter(row["verdict"] for row in rows).items())),
        "crossings": rows,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path.cwd())
    parser.add_argument("--out-dir", type=pathlib.Path, required=True)
    parser.add_argument("--tmp-dir", type=pathlib.Path)
    args = parser.parse_args()
    root = args.root.resolve()
    out_dir = args.out_dir.resolve()
    out_dir.mkdir(parents=True, exist_ok=True)

    walk = build_walk(root)
    (out_dir / "jit-walk-edges.json").write_text(json.dumps(walk, indent=2) + "\n")

    sweep_dir = root / "docs/design/beamr/briefs/evidence/review-23-07/rf-006/sweep"
    if args.tmp_dir:
        tmp_dir = args.tmp_dir.resolve()
        tmp_dir.mkdir(parents=True, exist_ok=True)
        cleanup = None
    else:
        cleanup = tempfile.TemporaryDirectory(prefix="beamr-rf006-")
        tmp_dir = pathlib.Path(cleanup.name)
    stage1_path = tmp_dir / "r6-stage1.json"
    candidates_path = tmp_dir / "r6-candidates.json"
    stage1 = run_stage([sys.executable, str(sweep_dir / "r6_sweep.py"), str(stage1_path)], root)
    stage3 = run_stage(
        [sys.executable, str(sweep_dir / "r6_stage3.py"), str(stage1_path), str(candidates_path), "wide"],
        root,
    )
    candidates = load_rows(candidates_path, "candidates")
    (out_dir / "jit-candidates-main.json").write_text(json.dumps(candidates, indent=2) + "\n")

    prior_rows = load_rows(sweep_dir / "verdicts.json", "verdicts")
    prior_by_key = {row_key(row): row for row in prior_rows}
    ruling_rows = json.loads((out_dir / "jit-rulings.json").read_text())["rulings"]
    rulings = {row_key(row): row for row in ruling_rows}
    beamr_candidates = [row for row in candidates if row["file"].startswith("crates/beamr/src/")]
    candidate_keys = {row_key(row) for row in beamr_candidates}
    inert = sorted(set(rulings) - candidate_keys)
    if inert:
        print(f"inert rulings: {inert}", file=sys.stderr)
        raise SystemExit(2)

    reached = set(walk["reached"])
    arm_p_candidates = [row for row in beamr_candidates if row["fn"] in reached]
    arm_w_candidates = [row for row in beamr_candidates if row["fn"] not in reached]
    arm_p_rows = apply_rulings(arm_p_candidates, rulings, prior_by_key)
    arm_w_rows = apply_rulings(arm_w_candidates, rulings, prior_by_key)

    current_by_key = {row_key(row): row for row in candidates}
    prior_keys = set(prior_by_key)
    current_keys = set(current_by_key)
    gone = [prior_by_key[key] for key in sorted(prior_keys - current_keys)]
    new = [current_by_key[key] for key in sorted(current_keys - prior_keys)]
    persisting = sorted(prior_keys & current_keys)
    formerly_real = [
        row
        for row in arm_p_rows + arm_w_rows
        if row.get("prior_verdict") in REAL_VERDICTS
    ]
    findings = [row for row in arm_p_rows + arm_w_rows if row["verdict"] in REAL_VERDICTS]
    wasm = [row for row in candidates if row["file"].startswith("crates/beamr-wasm/src/")]
    data = {
        "instrument": "RF-006 r6_sweep.py plus r6_stage3.py wide, followed by explicit jit-rulings.json hand rulings; JIT reachability depth <= 4 with interpreter/scheduler stop",
        "run": {
            "stage1_returncode": stage1.returncode,
            "stage1_stdout": stage1.stdout,
            "stage3_returncode": stage3.returncode,
            "stage3_stdout": stage3.stdout,
        },
        "arm_p": arm(arm_p_rows),
        "arm_w": arm(arm_w_rows),
        "out_of_population": {
            "beamr_wasm": len(wasm),
            "reason": "outside R4's crates/beamr/src population",
            "rows": wasm,
        },
        "diff_from_prior": {
            "prior_rows": len(prior_rows),
            "current_rows": len(candidates),
            "persisting": len(persisting),
            "gone": len(gone),
            "new": len(new),
            "gone_rows": gone,
            "new_rows": new,
            "formerly_real_survivors": formerly_real,
        },
        "findings": findings,
        "controls": {
            "ar1_cfg_test_candidates": sum(
                row["file"] == "crates/beamr/src/ar1_shape_control.rs" for row in candidates
            )
        },
    }
    (out_dir / "jit-crossings.json").write_text(json.dumps(data, indent=2) + "\n")
    print(f"RF-006 candidates: {len(candidates)} total; {len(beamr_candidates)} crates/beamr/src; {len(wasm)} beamr-wasm")
    print(f"arm P: {len(arm_p_rows)} {data['arm_p']['verdict_tally']}")
    print(f"arm W: {len(arm_w_rows)} {data['arm_w']['verdict_tally']}")
    print(f"REAL/REAL-OSIRIS findings: {len(findings)}")
    if cleanup:
        cleanup.cleanup()


if __name__ == "__main__":
    main()
