#!/usr/bin/env python3
"""RF-006 R6 stage 3 — capture-side narrowing.

Stage 2's collecting closure is NAME-based, so it over-approximates badly
(4039 of 6304 production fns). That number is a ceiling on reachability, not
a measurement of it, and it is useless as a discriminator on its own.

So the sweep runs from the CAPTURE side instead: a site only matters if a
Term / Vec<Term> / [Term] / *mut u64 / *const u64 local is
  bound  -> a collecting call happens -> the SAME local is read again.
The over-approximate collecting set only makes this list over-inclusive,
which is the safe direction for a candidate list that is then ruled by hand.
"""
import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from r6_sweep import index_file, files, test_files  # noqa: E402

# A local is a candidate carrier if its declaration names a Term/pointer type
# or its initialiser is a known Term/pointer producer.
TYPE_HINT = re.compile(
    r':\s*(?:&\s*)?(?:mut\s+)?(?:Vec<\s*Term|\[\s*Term|Term\b|\*\s*mut\s+u64|\*\s*const\s+u64)'
)
RHS_HINT = re.compile(
    r'Term::|\.raw\(\)|as\s+\*mut\s+u64|as\s+\*const\s+u64|\.heap_ptr\(\)'
    r'|source_term\(\)|from_raw\(|\.x_reg\(|alloc_\w+\(|\.key\(|\.value\(|\.element\('
)
BIND = re.compile(r'^\s*let\s+(?:mut\s+)?(?P<name>\w+)\s*(?P<rest>[:=].*)$')
DESTRUCT = re.compile(r'^\s*let\s+(?:mut\s+)?[\[(](?P<names>[^\])]+)[\])]\s*=')

# --- C-vi: the two carrier shapes the hints above CANNOT see -----------------
#
# The rules above recognise a carrier by an explicit `: Term` annotation or by
# a hard-coded list of constructor spellings. Two real shapes match neither:
#
#   (w1) a local bound from a DOMAIN HELPER that returns a Term --
#        `let ip = ipv4_tuple(..)?;`. Found by `finish_udp_recv`, where the
#        narrow binder flagged `port` (a try_small_int IMMEDIATE, never at
#        risk) and stayed silent on `ip`, boxed, two lines above it.
#
#   (w2) an unannotated Vec<Term> accumulator -- `let mut terms =
#        Vec::with_capacity(..)` -- pushed with terms and handed to an
#        allocator at the end, by which point the elements are already stale.
#
# C-i, C-ii and C-iii all made the population SMALLER, and a narrowing that
# only ever removes false positives is unfalsified in the direction that
# hurts. `narrow` is kept so the historical 36 stays re-derivable.
RET_TERM = re.compile(r'->[^{]*\bTerm\b')
VEC_BIND = re.compile(r'=\s*(?:Vec::(?:new|with_capacity)\s*\(|vec!\s*[\[\(])')
PUSH = r'\.push\s*\('


def term_returning_names(indexed):
    """Names of functions whose signature returns a Term (w1)."""
    out = set()
    for fns, _ in indexed.values():
        for fn in fns:
            if RET_TERM.search("\n".join(fn["body"].splitlines()[:4])):
                out.add(fn["name"])
    return out


def main():
    stage1 = json.loads(pathlib.Path(sys.argv[1]).read_text())
    collecting = set(stage1["collecting"])
    call_re = re.compile(r'\b(' + '|'.join(sorted(map(re.escape, collecting))) + r')\s*\(')

    binder = sys.argv[3] if len(sys.argv) > 3 else "wide"
    if binder not in ("narrow", "wide"):
        print(f"unknown binder mode: {binder}", file=sys.stderr)
        sys.exit(2)

    # The whole-file cfg(test) spelling must be honoured HERE TOO. Patching
    # the indexer's other caller left this one reading only inline spans.
    whole_test = test_files()
    indexed = {str(p): index_file(p) for p in files()}
    term_call = None
    if binder == "wide":
        names = term_returning_names(indexed)
        term_call = re.compile(r'\b(' + '|'.join(sorted(map(re.escape, names))) + r')\s*\(')
    candidates = []
    for p in files():
        if str(p) in whole_test:
            continue
        fns, spans = indexed[str(p)]
        for fn in fns:
            if any(a <= fn["start"] <= b for a, b in spans):
                continue
            lines = fn["body"].splitlines()
            binds = {}          # name -> line index within fn
            for i, line in enumerate(lines):
                m = BIND.match(line)
                if m and (TYPE_HINT.search(m.group("rest")) or RHS_HINT.search(m.group("rest"))):
                    binds.setdefault(m.group("name"), i)
                elif m and binder == "wide":
                    name, rest = m.group("name"), m.group("rest")
                    # (w1) bound from a Term-returning domain helper.
                    if term_call.search(rest):
                        binds.setdefault(name, i)
                    # (w2) a Vec accumulator that is pushed a term somewhere in
                    # this function. The push may sit many lines below the
                    # bind, so this looks at the whole body, not the RHS.
                    elif VEC_BIND.search(rest):
                        pushes = re.compile(r'\b' + re.escape(name) + PUSH)
                        if any(pushes.search(l)
                               and (RHS_HINT.search(l) or term_call.search(l))
                               for l in lines):
                            binds.setdefault(name, i)
                m = DESTRUCT.match(line)
                # A destructuring bind must ALSO look like it carries terms.
                # Without this, `let (fail, source, destination) = match (op,
                # operands) { .. }` registers three &Operand references as Term
                # carriers -- and instruction operands are module-resident, so
                # they neither move nor need rooting. That single unchecked arm
                # accounted for every site in binary/matching.rs.
                if m and (TYPE_HINT.search(line) or RHS_HINT.search(line)):
                    for nm in re.findall(r'\w+', m.group("names")):
                        binds.setdefault(nm, i)
            if not binds:
                continue
            calls = [(i, call_re.search(line).group(1))
                     for i, line in enumerate(lines) if call_re.search(line)]
            if not calls:
                continue
            for name, bi in binds.items():
                word = re.compile(r'\b' + re.escape(name) + r'\b')
                for ci, cname in calls:
                    if ci <= bi:
                        continue
                    # The collecting call must not itself be the line that
                    # rebinds the local (that is a fresh value, not a carry).
                    if ci == bi:
                        continue
                    uses = [k for k in range(ci + 1, len(lines)) if word.search(lines[k])]
                    if uses:
                        candidates.append({
                            "file": fn["file"], "fn": fn["name"],
                            "fn_line": fn["start"] + 1,
                            "var": name,
                            "bind": fn["start"] + 1 + bi,
                            "call": fn["start"] + 1 + ci,
                            "call_name": cname,
                            "use": fn["start"] + 1 + uses[0],
                        })
                        break

    out = pathlib.Path(sys.argv[2])
    out.write_text(json.dumps(candidates, indent=1))
    by_fn = {(c["file"], c["fn"]) for c in candidates}
    by_file = {c["file"] for c in candidates}
    print(f"binder mode                 : {binder}")
    print(f"candidate (var, call) sites : {len(candidates)}")
    print(f"distinct functions          : {len(by_fn)}")
    print(f"distinct files              : {len(by_file)}")
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
