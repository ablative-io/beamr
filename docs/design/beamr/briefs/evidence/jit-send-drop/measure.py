#!/usr/bin/env python3
"""Per-version census of the beamr jit_send_message self-send-only defect.

Measures the PACKAGED BYTES of each published .crate. Emits JSON rows.
Any measurement failure is recorded in the row, never silently skipped.
"""
import json, hashlib, subprocess, sys, tomllib
from pathlib import Path

BASE = Path("/private/tmp/claude-501/-Users-tom-Developer-ablative-stack-beamr/b337ce2b-336a-4856-a9d8-54c90496c9fa/scratchpad/census-crates")
REPO = "/Users/tom/Developer/ablative/stack/beamr"

versions = [l.strip() for l in (BASE / "version-list.txt").read_text().splitlines() if l.strip()]
yanked = {v["num"]: v["yanked"] for v in json.load(open(BASE / "versions.json"))["versions"]}
assert len(versions) == 58 and set(versions) == set(yanked), "version list / yanked map mismatch"

def extract_fn(src: str, fnname: str):
    """Brace-counted extraction of the full item containing `fn <fnname>`.
    Returns (body_text, error). Starts from beginning of the line holding the
    fn keyword (captures attrs on same line only via signature; attrs above are
    NOT included -- body identity is what we hash)."""
    idx = src.find("fn " + fnname)
    if idx == -1:
        return None, "fn-not-found"
    # back up to start of line
    line_start = src.rfind("\n", 0, idx) + 1
    # find first '{' after signature
    brace = src.find("{", idx)
    if brace == -1:
        return None, "no-open-brace"
    depth = 0
    i = brace
    n = len(src)
    in_str = False
    in_char = False
    in_line_comment = False
    in_block_comment = False
    prev = ""
    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if in_line_comment:
            if c == "\n":
                in_line_comment = False
        elif in_block_comment:
            if c == "*" and nxt == "/":
                in_block_comment = False
                i += 1
        elif in_str:
            if c == "\\":
                i += 1
            elif c == '"':
                in_str = False
        elif in_char:
            if c == "\\":
                i += 1
            elif c == "'":
                in_char = False
        else:
            if c == "/" and nxt == "/":
                in_line_comment = True
                i += 1
            elif c == "/" and nxt == "*":
                in_block_comment = True
                i += 1
            elif c == '"':
                in_str = True
            elif c == "'":
                # lifetime vs char literal: treat as char only if closing ' within 3 chars sans escape shape
                if nxt == "\\" or (i + 2 < n and src[i + 2] == "'"):
                    in_char = True
            elif c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    return src[line_start : i + 1], None
        i += 1
    return None, "unbalanced-braces"

rows = []
for v in versions:
    row = {"version": v, "yanked": yanked[v]}
    pkg = BASE / "extracted" / f"beamr-{v}"

    # (a) vcs sha
    vcs = pkg / ".cargo_vcs_info.json"
    if vcs.is_file():
        try:
            row["vcs_sha"] = json.load(open(vcs))["git"]["sha1"]
        except Exception as e:
            row["vcs_sha"] = None
            row["vcs_error"] = f"{type(e).__name__}: {e}"
    else:
        row["vcs_sha"] = None
        row["vcs_error"] = "no .cargo_vcs_info.json in package"

    # sha exists in local repo?
    if row["vcs_sha"]:
        p = subprocess.run(["git", "-C", REPO, "cat-file", "-t", row["vcs_sha"]],
                           capture_output=True, text=True)
        row["sha_in_repo"] = (p.returncode == 0 and p.stdout.strip() == "commit")
        if p.returncode != 0:
            row["sha_in_repo_stderr"] = p.stderr.strip()
    else:
        row["sha_in_repo"] = None

    # (b) Cargo.toml features
    ct = pkg / "Cargo.toml"
    try:
        toml = tomllib.load(open(ct, "rb"))
        feats = toml.get("features", {})
        row["jit_feature"] = "jit" in feats
        row["jit_in_default"] = "jit" in feats.get("default", [])
        row["default_features"] = feats.get("default", [])
    except Exception as e:
        row["jit_feature"] = None
        row["jit_in_default"] = None
        row["cargo_toml_error"] = f"{type(e).__name__}: {e}"

    # (c) packaged source: which jit file carries the fn (check BOTH paths every version)
    cand = [pkg / "src" / "jit" / "runtime_message.rs", pkg / "src" / "jit" / "runtime.rs"]
    row["jit_dir_exists"] = (pkg / "src" / "jit").is_dir()
    carrier = None
    body = None
    err = None
    hits = []
    for f in cand:
        if f.is_file() and "fn jit_send_message" in f.read_text():
            hits.append(f)
    if len(hits) > 1:
        row["fn_file"] = "AMBIGUOUS:" + ",".join(str(h.relative_to(pkg)) for h in hits)
        row["fn_present"] = True
        row["fn_error"] = "fn appears in more than one candidate file"
    elif len(hits) == 1:
        carrier = hits[0]
        row["fn_file"] = str(carrier.relative_to(pkg))
        row["fn_present"] = True
        body, err = extract_fn(carrier.read_text(), "jit_send_message")
        if err:
            row["fn_error"] = err
    else:
        # not in either candidate; sweep the whole package so a moved file can't hide
        sweep = []
        for f in pkg.rglob("*.rs"):
            try:
                if "fn jit_send_message" in f.read_text():
                    sweep.append(str(f.relative_to(pkg)))
            except Exception as e:
                sweep.append(f"READ-FAIL:{f.relative_to(pkg)}:{type(e).__name__}")
        if sweep:
            row["fn_file"] = "ELSEWHERE:" + ",".join(sweep)
            row["fn_present"] = True
            row["fn_error"] = "fn found outside the two named candidate paths"
        else:
            row["fn_file"] = None
            row["fn_present"] = False

    if body is not None:
        row["body_sha256"] = hashlib.sha256(body.encode()).hexdigest()[:16]
        row["body_lines"] = body.count("\n") + 1
        # store body for manual variant inspection
        vb = BASE / "bodies" / f"{row['body_sha256']}.rs"
        vb.parent.mkdir(exist_ok=True)
        if not vb.exists():
            vb.write_text(f"// first seen in beamr-{v}, file {row['fn_file']}\n" + body + "\n")
    rows.append(row)

out = BASE / "census-rows.json"
out.write_text(json.dumps(rows, indent=1))
print(f"rows: {len(rows)}")
variants = {}
for r in rows:
    variants.setdefault(r.get("body_sha256"), []).append(r["version"])
for h, vs in variants.items():
    print(f"body {h}: {len(vs)} versions: {vs[0]}..{vs[-1]}" if h else f"NO BODY: {len(vs)} versions: {', '.join(vs)}")
