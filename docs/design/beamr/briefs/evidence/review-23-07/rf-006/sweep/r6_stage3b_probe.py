"""C-vi probe: widen the binder to Term-RETURNING CALLS.

The committed binder recognises a Term carrier only by an explicit `: Term`
annotation or a hard-coded list of constructor spellings. A local bound from a
DOMAIN HELPER that returns a Term -- `ipv4_tuple(..)`, `ok_tuple(..)` -- matches
neither, so it is invisible. This adds one rule: the RHS calls a function whose
signature returns a Term.
"""
import json, pathlib, re, sys
sys.path.insert(0, 'docs/design/beamr/briefs/evidence/review-23-07/rf-006/sweep')
from r6_sweep import index_file, files, test_files

TYPE_HINT = re.compile(r':\s*(?:&\s*)?(?:mut\s+)?(?:Vec<\s*Term|\[\s*Term|Term\b|\*\s*mut\s+u64|\*\s*const\s+u64)')
RHS_HINT = re.compile(r'Term::|\.raw\(\)|as\s+\*mut\s+u64|as\s+\*const\s+u64|\.heap_ptr\(\)'
                      r'|source_term\(\)|from_raw\(|\.x_reg\(|alloc_\w+\(|\.key\(|\.value\(|\.element\(')
BIND = re.compile(r'^\s*let\s+(?:mut\s+)?(?P<name>\w+)\s*(?P<rest>[:=].*)$')
RET_TERM = re.compile(r'->[^{]*\bTerm\b')

stage1 = json.loads(pathlib.Path(sys.argv[1]).read_text())
collecting = set(stage1["collecting"])
call_re = re.compile(r'\b(' + '|'.join(sorted(map(re.escape, collecting))) + r')\s*\(')

# Set of fn names whose signature returns a Term (incl. Result<Term,_>, Option<Term>).
term_returning = set()
whole_test = test_files()
index = {}
for p in files():
    fns, spans = index_file(p)
    index[str(p)] = (fns, spans)
    for fn in fns:
        head = "\n".join(fn["body"].splitlines()[:4])
        if RET_TERM.search(head):
            term_returning.add(fn["name"])
term_call = re.compile(r'\b(' + '|'.join(sorted(map(re.escape, term_returning))) + r')\s*\(')

base, widened = [], []
for p in files():
    if str(p) in whole_test:
        continue
    fns, spans = index[str(p)]
    for fn in fns:
        if any(a <= fn["start"] <= b for a, b in spans):
            continue
        lines = fn["body"].splitlines()
        binds_base, binds_new = {}, {}
        for i, line in enumerate(lines):
            m = BIND.match(line)
            if not m:
                continue
            rest = m.group("rest")
            if TYPE_HINT.search(rest) or RHS_HINT.search(rest):
                binds_base.setdefault(m.group("name"), i)
            elif term_call.search(rest):
                binds_new.setdefault(m.group("name"), i)
        calls = [(i, call_re.search(l).group(1)) for i, l in enumerate(lines) if call_re.search(l)]
        if not calls:
            continue
        for bucket, out in ((binds_base, base), (binds_new, widened)):
            for name, bi in bucket.items():
                w = re.compile(r'\b' + re.escape(name) + r'\b')
                for ci, cname in calls:
                    if ci <= bi:
                        continue
                    if any(w.search(lines[k]) for k in range(ci + 1, len(lines))):
                        out.append({"file": fn["file"], "fn": fn["name"], "var": name,
                                    "bind": fn["start"] + 1 + bi, "call": fn["start"] + 1 + ci,
                                    "call_name": cname})
                        break

print(f"Term-returning fn names        : {len(term_returning)}")
print(f"baseline sites (committed rule): {len(base)}")
print(f"NEWLY VISIBLE sites            : {len(widened)}")
print(f"distinct fns newly implicated  : {len({(c['file'],c['fn']) for c in widened})}")
pathlib.Path(sys.argv[2]).write_text(json.dumps(widened, indent=1))
