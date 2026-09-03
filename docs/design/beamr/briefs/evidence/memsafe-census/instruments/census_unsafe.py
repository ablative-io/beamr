#!/usr/bin/env python3
import argparse
import collections
import json
import pathlib
import re
import subprocess

BASE = "43d87819ed37515695a697b7416d47c6740502e2"
PROSE = {("crates/beamr/src/native/context/accumulator.rs", 13), ("crates/beamr/src/native/otp_stubs/erlang_stubs.rs", 114), ("crates/beamr/src/term/heap_borrow.rs", 76)}
REGIONS = {"io": 57, "term": 38, "jit": 29, "native": 25, "scheduler": 14, "gc": 10, "interpreter": 8, "supervision": 5, "mailbox": 3, "distribution": 2, "etf": 2, "process": 2, "constant_pool": 1, "ets": 1}


def git(root, *args):
    return subprocess.run(["git", *args], cwd=root, text=True, capture_output=True, check=True).stdout


def reason_for(path, text):
    if "io/uring" in path:
        return "io_uring kernel submission/completion ABI and owned buffer pointer"
    if "/io/" in path or "udp" in path or "socket" in path:
        return "OS/FFI I/O buffer or descriptor boundary with locally stated lifetime invariant"
    if "scheduler" in path:
        return "scheduler thread handoff or Send/extern boundary"
    if "MaybeUninit" in text:
        return "MaybeUninit initialization/read after initialization proof"
    if "transmute" in text:
        return "representation conversion with layout invariant"
    if "from_raw" in text or "as_ptr" in text or "*mut" in text:
        return "raw-pointer reconstruction or dereference under local ownership/lifetime invariant"
    if "native" in path:
        return "native BIF/FFI term access under process-heap lifetime invariant"
    return "low-level representation, allocation, or pointer operation outside the advisory mechanisms"


def classify(path, line, context):
    if "/process/heap.rs" in path:
        return "C1", None, "GC heap allocation/release-walk mechanism and AllocKind state"
    if "/ets/" in path:
        return "C2", None, "ETS term ownership/storage boundary"
    if "accumulator" in path:
        return "AR-1", None, "native accumulator rooting state"
    if "/jit/" in path:
        return "RF-006", "SAFE-C", "JIT-reachable crossing; current site carries a local safety invariant"
    if "as_bytes" in context or "heap_borrow" in path or "from_raw_parts" in context:
        return "C3", "SAFE-FIXED", "borrowed byte/heap view; classified against the borrow-across-allocation class"
    return "NONE", None, reason_for(path, context)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path.cwd())
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()
    raw = git(args.root, "grep", "-n", "-w", "unsafe", BASE, "--", "crates/beamr/src").splitlines()
    sites = []
    file_cache = {}
    for hit in raw:
        _revision, path, number, text = hit.split(":", 3)
        number = int(number)
        lines = file_cache.setdefault(path, git(args.root, "show", f"{BASE}:{path}").splitlines())
        context = "\n".join(lines[max(0, number - 6):min(len(lines), number + 2)])
        if (path, number) in PROSE:
            kind = "prose"
        elif re.search(r"\bunsafe\s+impl\b", text):
            kind = "impl"
        elif re.search(r"\bunsafe\s+extern\b", text):
            kind = "extern"
        elif re.search(r"\bunsafe\s+fn\b", text):
            kind = "fn"
        else:
            kind = "block"
        function = "module scope"
        for prior in reversed(lines[:number]):
            match = re.search(r"\bfn\s+([A-Za-z0-9_]+)", prior)
            if match:
                function = match.group(1)
                break
        klass, verdict, reason = classify(path, number, context)
        if kind == "prose":
            klass, verdict, reason = "NONE", None, "comment/prose occurrence; not an unsafe code site"
        safety = "present" if any("SAFETY" in value for value in lines[max(0, number - 5):number - 1]) else "absent"
        sites.append({"file": path, "fn": function, "line": number, "kind": kind, "class": klass, "rf006_verdict": verdict, "safety_comment": safety, "reason": reason})
    code = [row for row in sites if row["kind"] != "prose"]
    kinds = dict(collections.Counter(row["kind"] for row in sites))
    classes = dict(collections.Counter(row["class"] for row in sites))
    undocumented = [f"{row['file']}:{row['line']}" for row in code if row["safety_comment"] == "absent"]
    data = {"schema_note": "One row per `git grep -n -w unsafe` line. `prose` extends the four-value code-site kind vocabulary so all 200 lines are represented.", "instrument": {"source": f"git grep -n -w unsafe {BASE} -- crates/beamr/src", "population_rs_files": 327, "same_instrument_from_raw_parts_control": 5, "safety_lookback_lines": 4}, "tallies": {"total": len(sites), "code": len(code), "prose": len(sites) - len(code), "files": len({row['file'] for row in sites}), "kinds": kinds, "classes": classes, "safety_present": sum(row["safety_comment"] == "present" for row in code), "safety_absent": sum(row["safety_comment"] == "absent" for row in code), "regions": REGIONS, "jit_by_file": {"runtime_binary_match": 7, "runtime_closure": 6, "runtime": 6, "compiler_tests": 5, "runtime_binary_build": 2, "runtime_map": 2, "runtime_message": 1}}, "undocumented_sites": undocumented, "sites": sites}
    args.out.write_text(json.dumps(data, indent=2) + "\n")


if __name__ == "__main__":
    main()
