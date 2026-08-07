"""AR-1 row 7 -- the third-carrier-shape hunt.

⭐ DELIBERATELY A DIFFERENTLY-SHAPED INSTRUMENT, not a widened r6_stage3.py.
A structurally blind instrument cannot be caught by looking at its output;
only a differently-shaped instrument disagreeing with it will do it -- and a
second generation of the SAME binder cannot falsify the first.

r6_stage3.py binds carriers by (a) `: Term` annotation, (b) a constructor
list, (w1) a Term-returning callee, (w2) Vec<Term> + .push. This one ignores
all of that and keys on BIND SYNTAX instead, asking a different question:
"what shapes can hold a Term that this file's binder never looks at?"

S3a collect/map -> collection   (w2 keys on Vec::new + push; .collect() is invisible)
S3b match/if-expression bind    (RHS is an expression, not a call)
S3c array/tuple-literal bind
S3d reassignment with no `let`  (the binder keys on `let`)
S3e Vec OF TUPLES               <-- THE HIT: carrier is an element inside a
                                    tuple inside a Vec, so it is neither a
                                    `Vec<Term>` nor a Term-shaped push arg.

Known-positive control: S3d must surface code_management_bifs.rs's
`list = context.alloc_cons(tuple, list)?` -- RF-006 defect #1. If it does not,
this instrument is broken and its zeroes mean nothing.
"""
import re, pathlib, collections, sys

ALLOC = re.compile(r'\.(alloc_binary|alloc_tuple|alloc_cons|alloc_list|alloc_map|'
                   r'alloc_float|alloc_bigint|alloc_reference|alloc_external_\w+|'
                   r'alloc_fd_resource)\s*\(')
TUPVEC = re.compile(r'let\s+mut\s+(\w+)\s*(?::\s*Vec<\([^)]*Term[^)]*\)>)?\s*=\s*'
                    r'Vec::(?:new|with_capacity)')
ALLOCISH = re.compile(r'\.(alloc_\w+)\s*\(|_to_term\s*\(|_term\s*\(')

def source_files():
    for p in sorted(pathlib.Path("crates").rglob("*.rs")):
        n = p.name
        if n == "tests.rs" or n.endswith("_tests.rs"): continue
        if "src/native/context/" in str(p): continue
        yield p

hits = collections.defaultdict(list)
for p in source_files():
    lines = p.read_text().splitlines()
    for i, l in enumerate(lines):
        blk = "\n".join(lines[i:i+8])
        if re.search(r'let\s+\w+.*=.*\.(map|filter_map)\s*\(', l) and '.collect' in blk and ALLOC.search(blk):
            hits['S3a'].append((p, i+1, l.strip()))
        if re.search(r'let\s+\w+\s*=\s*(match|if)\b', l) and ALLOC.search(blk):
            hits['S3b'].append((p, i+1, l.strip()))
        if re.search(r'let\s*[\(\[]?\w+.*=\s*[\[\(]', l) and ALLOC.search(l):
            hits['S3c'].append((p, i+1, l.strip()))
        if re.match(r'\s*\w+\s*=\s*[^=]', l) and ALLOC.search(l) and 'let ' not in l:
            hits['S3d'].append((p, i+1, l.strip()))
        m = TUPVEC.search(l)
        if m:
            var, blk14 = m.group(1), "\n".join(lines[i:i+14])
            if re.search(r'\b' + var + r'\.push\s*\(\s*\(', blk14) and ALLOCISH.search(blk14):
                hits['S3e'].append((p, i+1, l.strip()))

for k in sorted(hits):
    print(f"\n=== {k}: {len(hits[k])} ===")
    for f, ln, s in hits[k]:
        print(f"  {str(f).split('crates/')[-1]}:{ln}  {s[:86]}")

ctl = [h for h in hits['S3d'] if 'code_management_bifs' in str(h[0])]
print(f"\nKNOWN-POSITIVE CONTROL (RF-006 defect #1 via S3d): "
      f"{'PASS' if ctl else 'FAIL -- instrument is blind, its zeroes are meaningless'}")
sys.exit(0 if ctl else 2)
