"""AR-1 row 1 -- the ordering-sensitive carrier detector.

THE RULE (gate row 1, verbatim intent): a carrier is FLAGGED when a rooting
call occurs AFTER the first collecting call on that carrier's live range --
never merely because "a rooting call is absent from this function." The
terminal `alloc_list`/`alloc_tuple` at each of the original crossings DOES
root its elements; it roots values that are ALREADY STALE. So every one of
those sites contains a rooting call, and an instrument that measures
presence passes all of them while they are still broken. This instrument
measures ORDER.

## What it grades

The 17 named carriers in the lane's single disposition ledger
(dispositions.json -- rows 1..17; the CONTROL-FIXTURE rows belong to
shape_hunt.py and are not carriers), each RE-DERIVED at the current bytes:
the ledger contributes only (file, function, carrier); every line number in
this instrument's output is measured fresh. That makes it a differently-
shaped instrument from r6_stage3.py (which produced the bind/collect/use
triples) rather than a replay of the ledger -- and it is robust to the line
drift the tree has accumulated since the ledger's ground sha.

## Event classes, per carrier C (syntactic, per line, declared spellings)

ROOT  -- a line where C is named in rooting position:
         `with_rooted(` .. C | `rooted_push(` .. C |
         `alloc_{list,tuple,map,cons}(` .. C
         (C after the call token: C is being handed TO the rooting call.
         The terminal-alloc arm is deliberate: it is how "roots it, but
         too late" becomes measurable as an ordering fact.)
COLLECT -- a line that allocates while C is live and is NOT a ROOT line
         for C: `.alloc_*(` or a `*_to_term(` / `*_to_list(` callee.
         (A collecting call that ROOTS C -- e.g. `alloc_cons(x, C)` --
         is safe FOR C at that call: the real allocator roots its
         arguments before reserving. It still counts as nothing for C's
         earlier exposure, which the ordering comparison captures.)

The live range runs from C's `let` bind (exclusive -- the bind's own RHS
allocates INTO C before C exists) to the end of the enclosing function
body. End-of-body is deliberately conservative: last-use detection is
fragile, and the conservative range errs toward FLAG, the safe direction.

## Verdicts

COLLECT-FIRST  first COLLECT precedes first ROOT          -> FLAG
NO-ROOTING     COLLECTs exist, no ROOT names C            -> FLAG
ROOTED-FIRST   a ROOT naming C precedes every COLLECT     -> CLEAN
NO-COLLECT     nothing allocates on C's live range        -> CLEAN

Pre-fix expectation: every in-tree carrier FLAGS. At fix time `--sign-off`
refuses (rc 5) while any carrier still FLAGS; a carrier whose function or
bind no longer exists is reported UNGRADEABLE and the run exits 4 -- at fix
time such a site must carry a STRUCTURALLY-ELIMINATED disposition in the
ledger (verified by ledger_check.py, not by this instrument) before its
absence here stops being an error.

## Controls (gate row 1's break-it arm and control, committed)

crates/beamr/src/ar1_ordering_control.rs holds one BAD-ORDER fixture
(rooting present but after the collect -- MUST FLAG as COLLECT-FIRST) and
one GOOD-ORDER fixture (rooting scope opens first -- MUST read
ROOTED-FIRST CLEAN). Markers are the binding names `ord_bad_tail` /
`ord_good_tail`; each must let-bind EXACTLY once in the fixture file (0 or
>=2 is UNUSABLE, never PASS). Any control failure exits 2 and every other
number this run printed is uninterpretable.

## Declared limits (the instrument's honest fault domain)

Per-line reading: a rooting call reflowed across lines by fmt stops being
visible, which turns a real CLEAN into NO-ROOTING/FLAG -- the fail-safe
direction -- and kills the GOOD control loudly (exit 2) rather than
silently. Line order within a loop body is a proxy for temporal order; for
every shape in this population the proxy errs toward FLAG (a root spelled
inside the loop after the collect line covers later iterations, never the
first). Closure-parameter shadowing of a carrier name is not modelled.
No `2>/dev/null` anywhere; parse failures exit 3 with the file named.
"""

import json
import pathlib
import re
import sys

LEDGER = pathlib.Path(
    "docs/design/beamr/briefs/evidence/accumulator-rooting/dispositions.json"
)
FIXTURE_FILE = "crates/beamr/src/ar1_ordering_control.rs"
CONTROLS = [
    {
        "id": "ORD-BAD",
        "file": FIXTURE_FILE,
        "function": "ord_bad_root_after_collect",
        "carrier": "ord_bad_tail",
        "expect": "FLAG",
        "why": "rooting present but AFTER the collect -- presence-sensitive "
               "instruments clear it, ordering-sensitive ones flag it",
    },
    {
        "id": "ORD-GOOD",
        "file": FIXTURE_FILE,
        "function": "ord_good_rooted_before_collect",
        "carrier": "ord_good_tail",
        "expect": "CLEAN",
        "why": "rooting scope opens before any collect -- a detector that "
               "flags everything is not a detector",
    },
]

ROOT_TOKENS = re.compile(
    r"(with_rooted|rooted_push|alloc_list|alloc_tuple|alloc_map|alloc_cons)"
    r"\s*\("
)
COLLECT_RE = re.compile(r"\.alloc_\w+\s*\(|\b\w+_to_term\s*\(|\b\w+_to_list\s*\(")


def strip_noise(line, state):
    """Blank out string/char literals and comments so brace counting and
    token matching read STRUCTURE. Returns (clean_line, state) where state
    is True inside an unclosed block comment. Same fault-domain choices as
    shape_hunt.py's labeller walk (a deliberately shared shape: both must
    agree on what 'code' is)."""
    out = []
    i = 0
    while i < len(line):
        if state:
            j = line.find("*/", i)
            if j == -1:
                return "".join(out), True
            i = j + 2
            continue
        c = line[i]
        if line.startswith("//", i):
            break
        if line.startswith("/*", i):
            state = True
            i += 2
            continue
        m = re.match(r'r(#*)"', line[i:])
        if m:
            closer = '"' + m.group(1)
            end = line.find(closer, i + len(m.group(0)))
            i = len(line) if end == -1 else end + len(closer)
            out.append(" ")
            continue
        if c == '"':
            i += 1
            while i < len(line):
                if line[i] == "\\":
                    i += 2
                    continue
                if line[i] == '"':
                    i += 1
                    break
                i += 1
            out.append(" ")
            continue
        if c == "'":
            m = re.match(r"'(\\.|[^\\'])'", line[i:])
            if m:
                i += len(m.group(0))
                out.append(" ")
                continue
        out.append(c)
        i += 1
    return "".join(out), state


def clean_lines(path):
    state = False
    out = []
    for raw in path.read_text().splitlines():
        clean, state = strip_noise(raw, state)
        out.append(clean)
    if state:
        print(f"PARSE FAILURE: unclosed block comment in {path}")
        sys.exit(3)
    return out


def function_body(lines, fn_name, carrier):
    """(start, end) 1-based inclusive line span of `fn fn_name`'s body,
    chosen as the candidate whose body let-binds `carrier`. Exactly one
    such candidate is required."""
    sig = re.compile(r"\bfn\s+" + re.escape(fn_name) + r"\b")
    spans = []
    for idx, line in enumerate(lines):
        if not sig.search(line):
            continue
        depth = 0
        opened = False
        for j in range(idx, len(lines)):
            for c in lines[j]:
                if c == "{":
                    depth += 1
                    opened = True
                elif c == "}":
                    depth -= 1
            if opened and depth <= 0:
                spans.append((idx + 1, j + 1))
                break
        else:
            print(f"PARSE FAILURE: fn {fn_name} body never closes")
            sys.exit(3)
    bind_re = re.compile(r"\blet\s+(mut\s+)?" + re.escape(carrier) + r"\b")
    with_carrier = [
        (s, e)
        for s, e in spans
        if any(bind_re.search(lines[k]) for k in range(s - 1, e))
    ]
    return spans, with_carrier, bind_re


def grade(path_str, fn_name, carrier):
    path = pathlib.Path(path_str)
    if not path.exists():
        return {"verdict": "UNGRADEABLE", "detail": "file missing"}
    lines = clean_lines(path)
    spans, with_carrier, bind_re = function_body(lines, fn_name, carrier)
    if not spans:
        return {"verdict": "UNGRADEABLE", "detail": f"fn {fn_name} not found"}
    if len(with_carrier) != 1:
        return {
            "verdict": "UNGRADEABLE",
            "detail": f"carrier let-bind found in {len(with_carrier)} of "
                      f"{len(spans)} fn candidates (need exactly 1)",
        }
    start, end = with_carrier[0]
    bind_line = next(
        k + 1 for k in range(start - 1, end) if bind_re.search(lines[k])
    )
    carrier_re = re.compile(r"\b" + re.escape(carrier) + r"\b")
    first_root = first_collect = None
    for k in range(bind_line, end):  # bind line EXCLUSIVE, body end inclusive
        line = lines[k]
        is_root = any(
            carrier_re.search(line[m.end():]) for m in ROOT_TOKENS.finditer(line)
        )
        if is_root and first_root is None:
            first_root = k + 1
        if not is_root and COLLECT_RE.search(line) and first_collect is None:
            first_collect = k + 1
        if first_root is not None and first_collect is not None:
            break
    if first_collect is None:
        verdict = "NO-COLLECT/CLEAN"
    elif first_root is None:
        verdict = "NO-ROOTING/FLAG"
    elif first_collect < first_root:
        verdict = "COLLECT-FIRST/FLAG"
    else:
        verdict = "ROOTED-FIRST/CLEAN"
    return {
        "verdict": verdict,
        "bind": bind_line,
        "first_collect": first_collect,
        "first_root": first_root,
    }


def main():
    sign_off = "--sign-off" in sys.argv
    ledger = json.loads(LEDGER.read_text())
    sites = [s for s in ledger["sites"] if isinstance(s["id"], int)]
    print(f"population: {len(sites)} ledger carriers + {len(CONTROLS)} controls")
    if len(sites) != 17:
        print(f"DENOMINATOR FAILURE: ledger carries {len(sites)} carrier rows, "
              f"pre-registered population is 17")
        sys.exit(4)

    ungradeable, flags = [], []
    print(f"\n{'id':>3}  {'verdict':<20} {'bind':>5} {'coll':>5} {'root':>5}  site")
    for s in sites:
        r = grade(s["file"], s["function"], s["carrier"])
        tag = f"{s['file'].split('crates/')[-1]}::{s['function']}::{s['carrier']}"
        if r["verdict"] == "UNGRADEABLE":
            ungradeable.append((s["id"], tag, r["detail"]))
            print(f"{s['id']:>3}  {'UNGRADEABLE':<20} {'-':>5} {'-':>5} {'-':>5}  "
                  f"{tag}  [{r['detail']}]")
            continue
        note = " [OSIRIS -- accounted, not owned]" if s["id"] == 14 else ""
        if r["verdict"].endswith("/FLAG"):
            flags.append(s["id"])
        print(f"{s['id']:>3}  {r['verdict']:<20} {r['bind']:>5} "
              f"{str(r['first_collect']):>5} {str(r['first_root']):>5}  {tag}{note}")

    print(f"\nsummary: {len(flags)} FLAG · "
          f"{len(sites) - len(flags) - len(ungradeable)} CLEAN · "
          f"{len(ungradeable)} UNGRADEABLE")

    # ---- CONTROLS: the committed break-it arm and clean control -------------
    print("\n=== CONTROLS ===")
    failed = []
    fixture = pathlib.Path(FIXTURE_FILE)
    if not fixture.exists():
        failed.append(f"fixture file {FIXTURE_FILE} missing entirely")
    else:
        fx_lines = clean_lines(fixture)
        for ctl in CONTROLS:
            bind_re = re.compile(
                r"\blet\s+(mut\s+)?" + re.escape(ctl["carrier"]) + r"\b"
            )
            n_binds = sum(1 for line in fx_lines if bind_re.search(line))
            if n_binds != 1:
                failed.append(
                    f"{ctl['id']}: marker `{ctl['carrier']}` let-binds "
                    f"{n_binds} times, expected exactly 1 -- UNUSABLE"
                )
                print(f"  FAIL  {ctl['id']}  marker x{n_binds}")
                continue
            r = grade(ctl["file"], ctl["function"], ctl["carrier"])
            got = ("FLAG" if r["verdict"].endswith("/FLAG")
                   else "CLEAN" if r["verdict"].endswith("/CLEAN")
                   else r["verdict"])
            if got != ctl["expect"]:
                failed.append(
                    f"{ctl['id']}: expected {ctl['expect']}, got {r['verdict']} "
                    f"-- {ctl['why']}"
                )
                print(f"  FAIL  {ctl['id']}  expected {ctl['expect']}, "
                      f"got {r['verdict']}")
            else:
                print(f"  PASS  {ctl['id']}  {r['verdict']} as required")

    if failed:
        print("\nCONTROL FAILURE -- every verdict this run printed is "
              "uninterpretable:")
        for f in failed:
            print(f"  {f}")
        sys.exit(2)
    print("\nBOTH CONTROLS PASS -- ordering sensitivity demonstrated THIS RUN "
          "in both directions.")

    if ungradeable:
        print("\nUNGRADEABLE SITES (at fix time these need a "
              "STRUCTURALLY-ELIMINATED ledger disposition, verified by "
              "ledger_check.py -- until then this is an error):")
        for sid, tag, why in ungradeable:
            print(f"  {sid}: {tag} -- {why}")
        sys.exit(4)

    if sign_off:
        if flags:
            print(f"\nSIGN-OFF REFUSED: {len(flags)} carrier(s) still FLAG: "
                  f"{flags}")
            sys.exit(5)
        print("\nSIGN-OFF: no carrier flags; dispositions remain "
              "ledger_check.py's to verify.")
    sys.exit(0)


if __name__ == "__main__":
    main()
