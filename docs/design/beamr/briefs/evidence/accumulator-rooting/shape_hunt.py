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

## KNOWN-POSITIVE CONTROLS -- five, one per class, re-sited off the live defect

The original control keyed on code_management_bifs.rs's
`list = context.alloc_cons(tuple, list)?` -- RF-006 defect #1. ⭐ A CONTROL
KEYED ON A LIVE DEFECT IS DESTROYED BY THE PATCH, so it has been retired in
favour of committed fixtures at

    crates/beamr/src/ar1_shape_control.rs

one per shape class. Before that, S3a/S3b/S3c/S3e were WHOLLY UNCONTROLLED --
the TUPVEC regex could have broken and this script would still have printed
PASS -- and S3c had never once been shown to produce a presence, so every S3c
zero it ever reported was uninterpretable rather than reassuring.

⚠️ DECLARED COUPLING, and it rots silently if it is not read: the fixtures work
BECAUSE this hunt is a regex, blind to `cfg` and to attributes. That blindness
is load-bearing, not incidental -- a positive inside `#[cfg(test)]` is visible
here yet absent from the shipped binary, which is what lets the control survive
a structural remedy instead of dying when the remedy SUCCEEDS. ⇒ IF THIS HUNT
EVER BECOMES AST-BASED OR cfg-AWARE, THE FIXTURES STOP BEING VISIBLE TO IT AND
THE CONTROL MUST BE RE-SITED. The same goes for source_files(): the fixture file
must never be renamed to anything ending `_tests.rs`, or all five controls
vanish without a word.

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
walked = []
for p in source_files():
    walked.append(str(p))
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

# ---- KNOWN-POSITIVE CONTROLS: one per class, on committed fixtures ----------
# AR-1 R10. A single aggregate PASS is the loud-variant check that covers
# nothing of the silent one: the old control read S3d ONLY, so TUPVEC could
# have broken and this script would still have printed PASS -- and at fix time
# the reading on offer would have been "S3e sites were structurally
# eliminated". ⭐ A SILENTLY-BROKEN REGEX AND A SUCCESSFUL ELIMINATION PRODUCE
# THE SAME OUTPUT. Hence five, each failing on its own.
FIXTURE = "crates/beamr/src/ar1_shape_control.rs"
# ⭐ KEYED ON THE BINDING NAME, NOT ON "any hit of this class in the fixture
# file". Measured before this was tightened: the S3e fixture's own
# `.map(..).collect()` tail registered as an S3a hit, and the S3b fixture's
# match ARM (`_ => heap.alloc_cons(..)`) registered as an S3d hit -- the S3d
# regex reads any `X => alloc(..)` as a reassignment. Keyed loosely,
# EITHER WOULD HAVE HELD ITS NEIGHBOUR'S CLASS GREEN AFTER THAT NEIGHBOUR'S
# FIXTURE BROKE -- one fixture silently standing in for another is the
# mis-sited-mutation defect in a new dress. The marker is a binding name
# rather than a trailing comment so that no comment-reflow can strip it.
MARKERS = {'S3a': 's3a_mapped', 'S3b': 's3b_chosen', 'S3c': 's3c_pair',
           'S3d': 's3d_acc', 'S3e': 's3e_pairs'}
CLASSES = ('S3a', 'S3b', 'S3c', 'S3d', 'S3e')

# Denominator control (doctrine §5 ①). Five green fixture controls would also
# be produced by a run that walked the fixture file and NOTHING ELSE, so the
# population is asserted, not assumed.
print(f"\n=== CONTROLS: fixture {FIXTURE} ===")
print(f"  population walked: {len(walked)} files")
failed = []
if FIXTURE not in walked:
    failed.append(f"POPULATION -- {FIXTURE} was not walked at all "
                  f"(renamed? moved out of crates/? matched a source_files() skip?)")
elif len(walked) < 2:
    failed.append(f"POPULATION -- only {len(walked)} file(s) walked; the fixture "
                  f"cannot validate a search that searched nothing else")

for k in CLASSES:
    ctl = [h for h in hits[k] if str(h[0]) == FIXTURE and MARKERS[k] in h[2]]
    if not ctl:
        failed.append(f"{k} -- fixture `{MARKERS[k]}` not found; this class's zeroes "
                      f"are meaningless (reflowed by fmt? renamed? regex broken?)")
        print(f"  FAIL  {k}  fixture `{MARKERS[k]}` not detected")
        continue
    # UNIQUENESS, not existence. Two hits carrying one marker means the control
    # cannot say WHICH line kept it green -- an existence predicate answering a
    # uniqueness question is how the neighbouring-fixture defect got in above.
    if len(ctl) != 1:
        failed.append(f"{k} -- `{MARKERS[k]}` matched {len(ctl)} lines, expected exactly 1; "
                      f"the control is ambiguous and cannot be trusted either way")
        print(f"  FAIL  {k}  `{MARKERS[k]}` matched {len(ctl)} lines, expected 1")
        continue
    # The fixture is `#[cfg(test)]`, so a `prod` label here means the labeller
    # has lost track -- the control would still be green while the production
    # column it feeds is wrong.
    where = {label(f, ln) for f, ln, _ in ctl}
    if where != {'test'}:
        failed.append(f"{k} -- fixture detected but labelled {sorted(where)}, expected ['test']")
        print(f"  FAIL  {k}  labelled {sorted(where)}, expected ['test']")
        continue
    print(f"  PASS  {k}  line(s) {sorted(ln for _, ln, _ in ctl)} [test]")

if failed:
    print("\n⛔ CONTROL FAILURE -- this run's zeroes are uninterpretable:")
    for f in failed:
        print(f"  {f}")
    sys.exit(2)
print(f"\nALL {len(CLASSES)} CLASS CONTROLS PASS -- every class has been shown, "
      f"THIS RUN, to be able to produce a presence.")
sys.exit(0)
