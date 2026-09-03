#!/usr/bin/env python3
import argparse
import io
import json
import pathlib
import subprocess
import tarfile
import urllib.request

C1 = "2c064ed"
C2 = "487aae5"
FIX = "67f89c4"


def git(root, *args, check=True):
    return subprocess.run(["git", *args], cwd=root, text=True, capture_output=True, check=check)


def ancestor(root, commit, pin):
    return git(root, "merge-base", "--is-ancestor", commit, pin, check=False).returncode == 0


def version_key(value):
    return tuple(int(part) for part in value.split("."))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path.cwd())
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--cache", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.cache.mkdir(parents=True, exist_ok=True)
    index_url = "https://index.crates.io/be/am/beamr"
    rows = [json.loads(line) for line in urllib.request.urlopen(index_url, timeout=30).read().decode().splitlines()]
    selected = [row for row in rows if version_key(row["vers"]) < (0, 16, 3)]
    versions = []
    for item in selected:
        version = item["vers"]
        crate = args.cache / f"beamr-{version}.crate"
        if not crate.exists():
            url = f"https://static.crates.io/crates/beamr/beamr-{version}.crate"
            crate.write_bytes(urllib.request.urlopen(url, timeout=60).read())
        with tarfile.open(crate, "r:gz") as archive:
            name = f"beamr-{version}/.cargo_vcs_info.json"
            try:
                member = archive.extractfile(name)
            except KeyError:
                member = None
            if member is None:
                versions.append({"version": version, "pinned_sha": None, "c1": None, "c2": None, "fix": None, "affected": None, "yanked": item["yanked"], "source": "pin ABSENT from .crate: UNRESOLVED"})
                continue
            pin = json.load(member)["git"]["sha1"]
        if git(args.root, "cat-file", "-e", f"{pin}^{{commit}}", check=False).returncode != 0:
            raise SystemExit(f"HALT: pin present but absent from clone: {version} {pin}")
        c1 = ancestor(args.root, C1, pin)
        c2 = ancestor(args.root, C2, pin)
        fix = ancestor(args.root, FIX, pin)
        versions.append({"version": version, "pinned_sha": pin, "c1": c1, "c2": c2, "fix": fix, "affected": c1 and c2 and not fix, "yanked": item["yanked"], "source": ".cargo_vcs_info.json"})
    versions.sort(key=lambda row: version_key(row["version"]))
    data = {"instrument": {"index": index_url, "total_registry_versions": len(rows), "population": "every crates.io beamr version below 0.16.3", "commits": {"c1": C1, "c2": C2, "fix": FIX}}, "summary": {"below_0_16_3": len(versions), "affected": sum(row["affected"] is True for row in versions), "affected_live": sum(row["affected"] is True and not row["yanked"] for row in versions), "affected_yanked": sum(row["affected"] is True and row["yanked"] for row in versions), "pre_c1_clean": sum(row["c1"] is False and row["c2"] is False for row in versions), "unresolved": sum(row["pinned_sha"] is None for row in versions)}, "versions": versions}
    args.out.write_text(json.dumps(data, indent=2) + "\n")
    md = args.out.with_suffix(".md")
    lines = ["# Published beamr population below 0.16.3", "", f"Instrument: crates.io sparse index ({len(rows)} total versions), served `.crate` `.cargo_vcs_info.json` pins, and `git merge-base --is-ancestor` against `{C1}`, `{C2}`, `{FIX}`. Population: {len(versions)} versions below 0.16.3.", "", f"Measured: {data['summary']['affected']} affected ({data['summary']['affected_live']} live + {data['summary']['affected_yanked']} yanked), {data['summary']['pre_c1_clean']} pre-C1 clean, {data['summary']['unresolved']} unresolved.", "", "| version | pin | C1 | C2 | fix | affected | yanked |", "| --- | --- | --- | --- | --- | --- | --- |"]
    yn = lambda value: "UNRESOLVED" if value is None else ("Y" if value else "n")
    for row in versions:
        lines.append(f"| {row['version']} | `{row['pinned_sha'] or 'UNRESOLVED'}` | {yn(row['c1'])} | {yn(row['c2'])} | {yn(row['fix'])} | {yn(row['affected'])} | {'YANKED' if row['yanked'] else 'live'} |")
    md.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
