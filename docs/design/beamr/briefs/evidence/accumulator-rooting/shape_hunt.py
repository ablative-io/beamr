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

⚠️ THE CONTROL IS KEYED ON A LIVE DEFECT THIS LANE REPAIRS, so it dies with the
repair -- see AR-1-LANDING-GATE R10-a. Re-siting it on a synthetic fixture is a
separate change and is NOT made here.

## cfg(test) LABELLING -- added after a false positive it would have prevented

This hunt is a REGEX over method-name spelling. It cannot see `#[cfg(test)]`,
so a test helper and a production site are INDISTINGUISHABLE in its raw output.
That is not hypothetical: `native/udp_bifs.rs:473` was graded as a live row-6
defeat when it is a test helper inside `#[cfg(test)] mod tests` (opens :382).

The same blindness is DELIBERATELY LOAD-BEARING for the R10 fixture -- a
synthetic positive sited inside `#[cfg(test)]` survives a structural remedy
precisely because this regex does not read cfg.
⭐ A DELIBERATE USE OF A BLIND SPOT CREATES AN OBLIGATION TO LABEL EVERYTHING
THAT BLIND SPOT HIDES: the blindness has been converted from an accident into a
dependency, and every downstream reader inherits it without being told.

⇒ LABEL, NEVER FILTER. Filtering would be wrong twice over: it blinds the hunt
in the direction that hurts, AND it would hide the fixture the control depends
on. ⭐ A CAVEAT IN A DOCSTRING FIRES AT 0%; A COLUMN FIRES AT EVERY LOOKUP.
Counts are UNCHANGED by this addition -- only the labels are new.

The labeller is a brace-depth walk, which is a DIFFERENT SHAPE from the regex
it annotates, and it is orthogonal to source_files() -- so it labels the R10
fixture correctly wherever that fixture is sited.
⚠️ It has its own self-check: every file must close at depth 0. If any does
not, the walk has been confused (raw string, macro, unbalanced cfg_attr) and
this script exits 3 rather than emit labels it cannot stand behind.
"""
import re, pathlib, collections, sys

ALLOC = re.compile(r'\.(alloc_binary|alloc_tuple|alloc_cons|alloc_list|alloc_map|'
                   r'alloc_float|alloc_bigint|alloc_reference|alloc_external_\w+|'
                   r'alloc_fd_resource)\s*\(')
TUPVEC = re.compile(r'let\s+mut\s+(\w+)\s*(?::\s*Vec<\([^)]*Term[^)]*\)>)?\s*=\s*'
                    r'Vec::(?:new|with_capacity)')
ALLOCISH = re.compile(r'\.(alloc_\w+)\s*\(|_to_term\s*\(|_term\s*\(')


def cfg_test_lines(path):
    """Line numbers sitting inside a `#[cfg(test)]` block, plus the closing depth.

    Returns (gated_lines, final_depth). final_depth != 0 means the walk lost
    track and the labels for this file must not be trusted.

    ⚠️ A `#[cfg(test)]` on a BRACELESS item -- `mod tests;`, a `use`, a `const`
    -- has no block of its own. The arming flag must therefore be DISARMED at
    that item's `;`, or it stays live and captures the next brace in the file,
    which can be production code hundreds of lines away. Measured before the
    fix at f0be59a: 81 braceless cfg(test) sites in crates/, 9 files with wrong
    labels, 40 lines wrong, and ⭐ ALL 40 IN THE HIDING DIRECTION -- production
    reported as `cfg(test)`, none the safe way. The hit set was unaffected, so
    the 32/27/5 split published at f0be59a stands; the defect is in the
    instrument, not in that reading. Arm I in shape_hunt_controls.py fires on
    the unfixed walk. Found by Cally Ray while siting the R10 fixture.
    """
    src = path.read_text().splitlines()
    depth = 0
    in_block_comment = False
    pending_cfg = False
    test_depth = None
    gated = set()

    for lineno, line in enumerate(src, 1):
        if test_depth is not None and depth <= test_depth:
            test_depth = None
        if test_depth is not None:
            gated.add(lineno)
        if line.strip().startswith('#[cfg(test)]'):
            pending_cfg = True

        depth_before_line = depth
        i = 0
        while i < len(line):
            c = line[i]
            if in_block_comment:
                if line.startswith('*/', i):
                    in_block_comment = False
                    i += 2
                    continue
                i += 1
                continue
            if line.startswith('//', i):
                break
            if line.startswith('/*', i):
                in_block_comment = True
                i += 2
                continue
            # raw string: r"..." / r#"..."# / r##"..."##
            m = re.match(r'r(#*)"', line[i:])
            if m:
                closer = '"' + m.group(1)
                end = line.find(closer, i + len(m.group(0)))
                i = len(line) if end == -1 else end + len(closer)
                continue
            if c == '"':
                i += 1
                while i < len(line):
                    if line[i] == '\\':
                        i += 2
                        continue
                    if line[i] == '"':
                        i += 1
                        break
                    i += 1
                continue
            if c == "'":
                # char literal ('x' or '\n' or '{') vs lifetime ('a)
                m = re.match(r"'(\\.|[^\\'])'", line[i:])
                if m:
                    i += len(m.group(0))
                    continue
                i += 1
                continue
            if c == ';' and pending_cfg:
                # braceless cfg(test) item: it ends here and owns no block, so
                # the attribute must not reach forward to the next `{`.
                pending_cfg = False
            if c == '{':
                depth += 1
                if pending_cfg and test_depth is None:
                    test_depth = depth_before_line
                    pending_cfg = False
                    # the OPENING line is part of the gated block too: a hit can
                    # sit on the same line as the brace. Found by the two-arm
                    # control below, not in review.
                    gated.add(lineno)
            elif c == '}':
                depth -= 1
            i += 1

    return gated, depth


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

# ---- label every hit, and refuse to label at all if the walk lost track ------
gated_cache, unbalanced = {}, []
for k in hits:
    for f, _, _ in hits[k]:
        if f not in gated_cache:
            gated, final_depth = cfg_test_lines(f)
            gated_cache[f] = gated
            if final_depth != 0:
                unbalanced.append((f, final_depth))

if unbalanced:
    print("SELF-CHECK FAILED -- brace walk did not close at depth 0:")
    for f, d in unbalanced:
        print(f"  {f}  final depth {d}")
    print("Labels withheld: an instrument that cannot parse the file must not "
          "assert where its lines live.")
    sys.exit(3)


def label(f, ln):
    return 'test' if ln in gated_cache[f] else 'prod'


totals = collections.Counter()
for k in sorted(hits):
    kinds = collections.Counter(label(f, ln) for f, ln, _ in hits[k])
    totals.update(kinds)
    print(f"\n=== {k}: {len(hits[k])}   "
          f"(production {kinds['prod']} · cfg(test) {kinds['test']}) ===")
    for f, ln, s in hits[k]:
        print(f"  [{label(f, ln)}] {str(f).split('crates/')[-1]}:{ln}  {s[:80]}")

raw = sum(len(v) for v in hits.values())
print(f"\n=== TOTALS: {raw} raw · {totals['prod']} production · "
      f"{totals['test']} cfg(test) ===")
print("cfg(test) hits are REAL matches of the shape in code that does not ship.")
print("They are NOT filtered out: the R10 fixture will land in this same class,")
print("and a hunt that hides its own control certifies nothing.")
for k in sorted(hits):
    for f, ln, _ in hits[k]:
        if label(f, ln) == 'test':
            print(f"  cfg(test): {k}  {str(f).split('crates/')[-1]}:{ln}")

ctl = [h for h in hits['S3d'] if 'code_management_bifs' in str(h[0])]
ctl_where = f" [{label(ctl[0][0], ctl[0][1])}]" if ctl else ""
print(f"\nKNOWN-POSITIVE CONTROL (RF-006 defect #1 via S3d){ctl_where}: "
      f"{'PASS' if ctl else 'FAIL -- instrument is blind, its zeroes are meaningless'}")
sys.exit(0 if ctl else 2)
