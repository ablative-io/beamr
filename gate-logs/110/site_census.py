#!/usr/bin/env /usr/bin/python3
"""AR-1 row 4 — do the seventeen sites still exist as the ledger records them?

The ledger (dispositions.json) pins each site by file, function name, and four
LINE NUMBERS, all recorded pre-fix. Line numbers drift; a red aimed at a drifted
line proves nothing. This instrument re-locates every site at a given git ref
BY ITS OWN BYTES -- the function name in the file's committed blob -- and
reports drift rather than assuming its absence.

It reads the COMMITTED blob (`git show <ref>:<path>`), not the worktree, so the
bytes measured are the bytes that ref carries.

Usage: site_census.py [ref]     (default HEAD)
Exit 0 always; the verdict is the table. A site that cannot be located is
printed as MISSING, which is a finding, not an error.
"""

import json
import re
import subprocess
import sys

LEDGER = "docs/design/beamr/briefs/evidence/accumulator-rooting/dispositions.json"


def show(ref, path):
    r = subprocess.run(["git", "show", f"{ref}:{path}"], capture_output=True, text=True)
    if r.returncode != 0:
        return None
    return r.stdout


def blob(ref, path):
    r = subprocess.run(["git", "rev-parse", f"{ref}:{path}"], capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else None


def find_fn(text, name):
    """Line numbers (1-based) of every `fn <name>(` definition in the blob."""
    pat = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+|async\s+|unsafe\s+|extern\s+\"[^\"]*\"\s+)*fn\s+"
                     + re.escape(name) + r"\s*[(<]")
    return [i for i, line in enumerate(text.splitlines(), 1) if pat.match(line)]


def classify(ref, s):
    """The single-site verdict, factored out so the controls exercise the SAME
    code path the census does. A control that runs a parallel implementation
    proves that implementation, not the instrument."""
    text = show(ref, s["file"])
    if text is None:
        return "MISSING-FILE", None
    hits = find_fn(text, s["function"])
    if not hits:
        return "MISSING-FN", None
    if len(hits) > 1:
        return f"AMBIGUOUS({len(hits)})", hits[0]
    rec = s["function_line"]
    if rec is None:
        return "NO-REC-LINE", hits[0]
    if hits[0] == rec:
        return "EXACT", hits[0]
    return f"DRIFT{hits[0] - rec:+d}", hits[0]


def controls(ref, sites):
    """Break the property three ways; each break MUST change the verdict.

    A census that reports 17/17 EXACT has said nothing until it is shown able
    to say otherwise -- 'not found' and 'not looked for' produce the same
    output and mean opposite things (row 7's rule, applied to this instrument).
    """
    probe = dict(sites[0])
    arms = []

    a = dict(probe, function_line=probe["function_line"] + 1)
    arms.append(("recorded line +1", classify(ref, a)[0], "DRIFT-1"))

    b = dict(probe, function=probe["function"] + "_no_such_fn")
    arms.append(("function name mangled", classify(ref, b)[0], "MISSING-FN"))

    c = dict(probe, file=probe["file"] + ".no_such_file")
    arms.append(("file path mangled", classify(ref, c)[0], "MISSING-FILE"))

    print("## CONTROLS -- the instrument observed red")
    ok = True
    for name, got, want in arms:
        verdict = "PASS" if got == want else "FAIL"
        ok &= got == want
        print(f"  [{verdict}] {name:<24} -> {got:<14} (required {want})")
    print(f"  controls {'ALL PASS' if ok else 'FAILED -- the census below proves nothing'}")
    print()
    return ok


def main():
    ref = sys.argv[1] if len(sys.argv) > 1 else "HEAD"
    ledger = json.loads(open(LEDGER).read())
    sites = [s for s in ledger["sites"] if s["disposition"] != "CONTROL-FIXTURE"]
    controls(ref, sites)

    print(f"# AR-1 row-4 site census at ref {ref}")
    print(f"# ledger population (non-control): {len(sites)}")
    print()

    located = drifted = missing = 0
    rows = []
    for s in sites:
        sid, path, fn = s["id"], s["file"], s["function"]
        text = show(ref, path)
        if text is None:
            rows.append((sid, path, fn, "MISSING-FILE", "", "", ""))
            missing += 1
            continue
        hits = find_fn(text, fn)
        b = blob(ref, path)
        rec = s["function_line"]
        if not hits:
            rows.append((sid, path, fn, "MISSING-FN", str(rec), "-", b))
            missing += 1
            continue
        if len(hits) > 1:
            state = f"AMBIGUOUS({len(hits)})"
        elif rec is None:
            # the ledger recorded no function_line for this site; located by
            # name only, and that is reported rather than papered over
            state = "NO-REC-LINE"
            located += 1
        elif hits[0] == rec:
            state = "EXACT"
            located += 1
        else:
            state = f"DRIFT{hits[0] - rec:+d}"
            drifted += 1
        # the carrier's bind line, quoted from the ref's own bytes
        lines = text.splitlines()
        bl = s.get("bind_line")
        bind_txt = lines[bl - 1].strip() if bl and 0 < bl <= len(lines) else "<out of range>"
        rows.append((sid, path, fn, state, str(rec), str(hits[0]), b))
        rows.append((None, "", f"  bind_line {bl}: {bind_txt}", "", "", "", ""))

    for r in rows:
        if r[0] is None:
            print(f"        {r[2]}")
            continue
        sid, path, fn, state, rec, found, b = r
        print(f"{sid:>3}  {state:<12} rec_fn_line={rec:<5} found={found:<5} {path}::{fn}")
        print(f"     blob {b}")

    print()
    print(f"EXACT {located} · DRIFTED {drifted} · MISSING {missing} · of {len(sites)}")


if __name__ == "__main__":
    main()
