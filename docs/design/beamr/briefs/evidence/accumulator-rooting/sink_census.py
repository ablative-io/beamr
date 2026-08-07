"""Sink census for the unrooted-accumulator lane -- r3 (callee-resolved).

Three defects of my own, each found by disagreeing with a prior instrument:
 r1: matcher required a '.'/'::' receiver -> structurally blind to FREE
     functions. Reported list_from_vec = 0; truth is 11.
 r2: over-corrected -- counted every free call of a matching NAME, conflating
     FIVE different functions that share these names (ProcessContext methods,
     distribution/etf.rs's heap-based alloc_list, ets/match_{arena,spec}.rs
     arena allocators, and gc test helpers taking &mut process).
 r3: resolves the callee. ProcessContext sinks are METHOD form; list_from_vec
     is the one genuine FREE sink. Definitions and `use` lines excluded, and
     the exclusion is COUNTED so a silent zero cannot masquerade as a filter.
"""
import re, pathlib, collections, json, sys

METHOD_SINKS = ["alloc_list_with_tail", "alloc_list", "alloc_tuple", "alloc_map"]
FREE_SINKS   = ["list_from_vec"]
THREADING    = ["alloc_cons"]
LIT = re.compile(r'^&?\s*\[')
VAR = re.compile(r'^&?\s*(?:mut\s+)?[A-Za-z_]\w*\??(?:\s*\[\s*\.\.\s*\])?$')

def is_test(p):
    n = p.name
    return n == "tests.rs" or n.endswith("_tests.rs") or "/tests/" in str(p)

def balanced(t, i):
    d = 0
    for j in range(i, len(t)):
        if t[j] == '(': d += 1
        elif t[j] == ')':
            d -= 1
            if d == 0: return t[i+1:j]
    return None

def split_top(s):
    out, d, cur = [], 0, []
    for c in s:
        if c in '([{': d += 1
        elif c in ')]}': d -= 1
        if c == ',' and d == 0: out.append(''.join(cur)); cur = []
        else: cur.append(c)
    if ''.join(cur).strip(): out.append(''.join(cur))
    return [a.strip() for a in out]

rows, excl = [], collections.Counter()
for p in sorted(pathlib.Path("crates").rglob("*.rs")):
    if "src/native/context/" in str(p): continue
    text = p.read_text(); lines = text.splitlines()
    for kind, names in (("collection", METHOD_SINKS), ("collection", FREE_SINKS),
                        ("threading", THREADING)):
        for sink in names:
            want_free = sink in FREE_SINKS
            for m in re.finditer(r'(?<![\w])' + sink + r'\s*\(', text):
                ln = text[:m.start()].count('\n') + 1
                src = lines[ln-1]
                if re.search(r'\bfn\s+' + sink + r'\s*\(', src): excl['definition'] += 1; continue
                if src.lstrip().startswith('use '): excl['use-import'] += 1; continue
                c = text[m.start()-1] if m.start() else ''
                is_method = c == '.'
                if want_free and is_method: excl['wrong-form'] += 1; continue
                if not want_free and not is_method: excl['not-ProcessContext-method'] += 1; continue
                a = balanced(text, text.index('(', m.end()-1))
                if a is None: excl['unbalanced'] += 1; continue
                parts = split_top(a); coll = parts[0] if parts else ''
                rows.append(dict(file=str(p), line=ln, sink=sink, kind=kind,
                                 test=is_test(p), arg=coll[:60],
                                 shape=("literal" if LIT.match(coll) else
                                        "variable" if VAR.match(coll) else "other")))
prod = [r for r in rows if not r['test']]
coll = [r for r in prod if r['kind'] == 'collection']
thr  = [r for r in prod if r['kind'] == 'threading']
print(f"excluded (counted, not silent): {dict(excl)}\n")
print("COLLECTION SINKS -- take a Term collection (the type-change surface)")
print(f"{'sink':22} {'prod':>5} {'literal':>8} {'variable':>9} {'other':>6}")
for s in METHOD_SINKS + FREE_SINKS:
    g = [r for r in coll if r['sink'] == s]
    c = collections.Counter(r['shape'] for r in g)
    print(f"{s:22} {len(g):5} {c['literal']:8} {c['variable']:9} {c['other']:6}")
c = collections.Counter(r['shape'] for r in coll)
print(f"{'TOTAL':22} {len(coll):5} {c['literal']:8} {c['variable']:9} {c['other']:6}")
print(f"\nTAIL-THREADING (alloc_cons, no collection arg): {len(thr)} production sites")
print(f"\n⇒ TYPE-CHANGE BLAST RADIUS  = {len(coll)} collection call sites")
print(f"⇒ ACCUMULATOR-CAPABLE       = {c['variable'] + c['other']} "
      f"(variable {c['variable']} + other {c['other']}) -- the rest pass a fixed literal array")
json.dump(rows, open(sys.argv[1], "w"), indent=2) if len(sys.argv) > 1 else None
