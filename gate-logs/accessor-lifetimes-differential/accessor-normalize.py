#!/usr/bin/env python3
"""accessor-normalize.py — turn two arms of raw battery output into comparable
ERROR MULTISETS WITH COORDINATES, then diff them.

Why this exists, and the one design decision that matters:

    LINE NUMBERS MOVE. The landing adds a `HeapBorrow` parameter and ripples it
    through 83 source files, so a diagnostic that is genuinely THE SAME
    diagnostic will sit on a different line in the two arms. A multiset keyed on
    (lint, file, LINE) would therefore report a difference for every red that
    merely shifted, and I would publish a delta that is pure line drift.

    So the primary key is (level, lint, file, message) and the line is carried
    as an ATTRIBUTE, reported alongside. A red that moved is reported as MOVED,
    which is not the same as CHANGED, and neither is the same as NEW.

The jq extract is taken from gates.json verbatim, not restated here.
"""
import json
import os
import re
import subprocess
import sys
from collections import Counter

ROOT = "/home/rocketfish/fleet/artemis-diff"
LOGS = os.path.join(ROOT, "logs")
ARMS = ("base", "land")


def legs_from_gates(arm):
    with open(os.path.join(ROOT, arm, "gates.json")) as fh:
        return json.load(fh)["legs"]


def json_leg_findings(arm, leg):
    """Apply the leg's OWN jq extract from gates.json to the captured stdout."""
    spec = [l for l in legs_from_gates(arm) if l["name"] == leg][0]
    extract = spec["extract"]
    path = os.path.join(LOGS, f"{arm}.{leg}.json")
    if not os.path.exists(path):
        return None, f"no capture at {path}"
    proc = subprocess.run(
        ["jq", "-c", extract], stdin=open(path), capture_output=True, text=True
    )
    # jq's stderr is a measurement too — a parse failure here is a dead selector,
    # and a dead selector returns an empty set that reads exactly like a clean one.
    out = [json.loads(l) for l in proc.stdout.splitlines() if l.strip()]
    return out, proc.stderr.strip()


def norm_msg(m):
    """Collapse only what is provably positional, nothing semantic."""
    return re.sub(r"\s+", " ", m).strip()


def key_of(f):
    return (f.get("level"), f.get("lint"), f.get("file"), norm_msg(f.get("message") or ""))


TEST_FAIL = re.compile(r"^test\s+(\S+)\s+\.\.\.\s+FAILED", re.M)
TEST_TALLY = re.compile(
    r"test result:\s+(\w+)\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored", re.M
)


def test_leg_findings(arm, leg):
    path = os.path.join(LOGS, f"{arm}.{leg}.log")
    if not os.path.exists(path):
        return None, None, f"no capture at {path}"
    text = open(path, errors="replace").read()
    fails = TEST_FAIL.findall(text)
    tallies = TEST_TALLY.findall(text)
    passed = sum(int(t[1]) for t in tallies)
    failed = sum(int(t[2]) for t in tallies)
    ignored = sum(int(t[3]) for t in tallies)
    # A signal-killed run is NOT a test failure and must never be counted as one.
    signals = re.findall(r"(SIGBUS|SIGSEGV|signal: \d+|core dumped|Killed)", text)
    return fails, {"passed": passed, "failed": failed, "ignored": ignored,
                   "suites": len(tallies), "signals": sorted(set(signals))}, None


def rc_table():
    path = os.path.join(LOGS, "rc-table.tsv")
    out = {}
    if os.path.exists(path):
        for line in open(path):
            p = line.rstrip("\n").split("\t")
            if len(p) == 4:
                out[(p[0], p[1])] = (int(p[2]), int(p[3]))
    return out


def main():
    rcs = rc_table()
    legs = legs_from_gates("base")
    report = []
    report.append("# ACCESSOR-LIFETIMES DIFFERENTIAL — leg for leg")
    report.append("")
    report.append("base = c55ac360 (origin/main)   land = e1cb3060 (landing head)")
    report.append("")
    report.append("| leg | base rc | land rc | base secs | land secs | verdict |")
    report.append("|---|---|---|---|---|---|")
    for spec in legs:
        leg = spec["name"]
        b = rcs.get(("base", leg), ("?", "?"))
        l = rcs.get(("land", leg), ("?", "?"))
        v = "SAME rc" if b[0] == l[0] else "**rc DIFFERS**"
        report.append(f"| {leg} | {b[0]} | {l[0]} | {b[1]} | {l[1]} | {v} |")
    report.append("")

    for spec in legs:
        leg = spec["name"]
        report.append(f"## leg: {leg}")
        report.append("")
        if spec.get("format") == "json":
            sets = {}
            for arm in ARMS:
                found, err = json_leg_findings(arm, leg)
                if found is None:
                    report.append(f"- {arm}: NO CAPTURE ({err})")
                    sets[arm] = None
                    continue
                if err:
                    report.append(f"- {arm}: ⛔ jq stderr NON-EMPTY (a dead selector reads as clean): `{err[:300]}`")
                sets[arm] = found
                report.append(f"- {arm}: {len(found)} diagnostics extracted")
            if sets.get("base") is not None and sets.get("land") is not None:
                bk = Counter(key_of(f) for f in sets["base"])
                lk = Counter(key_of(f) for f in sets["land"])
                only_b = bk - lk
                only_l = lk - bk
                lines_b = {}
                lines_l = {}
                for f in sets["base"]:
                    lines_b.setdefault(key_of(f), []).append(f.get("line"))
                for f in sets["land"]:
                    lines_l.setdefault(key_of(f), []).append(f.get("line"))
                report.append("")
                report.append(f"**multiset (level,lint,file,message): base={sum(bk.values())} land={sum(lk.values())}**")
                if not only_b and not only_l:
                    report.append("")
                    report.append("✅ **MULTISETS EQUAL.** Every diagnostic present in one arm is present")
                    report.append("in the other with the same multiplicity.")
                    moved = [k for k in bk if sorted(lines_b.get(k, [])) != sorted(lines_l.get(k, []))]
                    if moved:
                        report.append("")
                        report.append(f"{len(moved)} of them MOVED (line drift only, expected from the ripple):")
                        for k in sorted(moved, key=str)[:40]:
                            report.append(f"  - `{k[1]}` {k[2]} : base lines {sorted(lines_b[k])} -> land lines {sorted(lines_l[k])}")
                    else:
                        report.append("")
                        report.append("None of them even moved: identical lines in both arms.")
                else:
                    report.append("")
                    report.append("⛔ **MULTISETS DIFFER — this is the delta to name.**")
                    for k, n in sorted(only_b.items(), key=str):
                        report.append(f"  - BASE-ONLY x{n}: `{k[1]}` {k[2]}:{sorted(lines_b.get(k,[]))} — {k[3][:200]}")
                    for k, n in sorted(only_l.items(), key=str):
                        report.append(f"  - **LAND-ONLY x{n}**: `{k[1]}` {k[2]}:{sorted(lines_l.get(k,[]))} — {k[3][:200]}")
        elif spec.get("kind") == "cargo-test":
            sets = {}
            for arm in ARMS:
                fails, tally, err = test_leg_findings(arm, leg)
                if fails is None:
                    report.append(f"- {arm}: NO CAPTURE ({err})")
                    sets[arm] = None
                    continue
                sets[arm] = fails
                report.append(f"- {arm}: {tally['passed']} passed / {tally['failed']} failed / "
                              f"{tally['ignored']} ignored across {tally['suites']} suites"
                              + (f"  ⚠️ SIGNALS: {tally['signals']}" if tally["signals"] else ""))
            if sets.get("base") is not None and sets.get("land") is not None:
                bk, lk = Counter(sets["base"]), Counter(sets["land"])
                only_b, only_l = bk - lk, lk - bk
                report.append("")
                if not only_b and not only_l:
                    report.append(f"✅ **FAILING-TEST MULTISETS EQUAL** ({sum(bk.values())} failing test names, identical both arms).")
                    for name in sorted(bk):
                        report.append(f"  - {name}")
                else:
                    report.append("⛔ **FAILING-TEST MULTISETS DIFFER.**")
                    for k, n in sorted(only_b.items()):
                        report.append(f"  - BASE-ONLY x{n}: {k}")
                    for k, n in sorted(only_l.items()):
                        report.append(f"  - **LAND-ONLY x{n}**: {k}")
        else:
            for arm in ARMS:
                path = os.path.join(LOGS, f"{arm}.{leg}.log")
                if not os.path.exists(path):
                    report.append(f"- {arm}: NO CAPTURE")
                    continue
                text = open(path, errors="replace").read()
                errs = [l for l in text.splitlines() if l.startswith("error") or "REFUSE" in l]
                report.append(f"- {arm}: rc={rcs.get((arm,leg),('?',))[0]}, {len(errs)} error/REFUSE lines")
                for e in errs[:12]:
                    report.append(f"      {e[:220]}")
        report.append("")

    out = "\n".join(report)
    with open(os.path.join(LOGS, "DIFFERENTIAL.md"), "w") as fh:
        fh.write(out + "\n")
    print(out)


if __name__ == "__main__":
    main()
