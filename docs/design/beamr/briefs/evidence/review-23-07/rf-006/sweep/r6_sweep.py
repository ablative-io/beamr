#!/usr/bin/env python3
"""RF-006 R6 sweep — stale-Term / raw-pointer capture audit.

Stage 1: index every fn in crates/beamr and crates/beamr-wasm, marking
         cfg(test) spans.
Stage 2: fixpoint the COLLECTING set — functions that can transitively reach
         gc::ensure_space / collect_minor / collect_major / jit alloc_words /
         jit_test_heap / any non-_prereserved ProcessContext::alloc_*.
Stage 3: for every call to a collecting function in non-test code, find
         Term / Vec<Term> / [Term] / *mut u64 / *const u64 locals bound
         BEFORE the call and USED AFTER it, inside the same fn.

Output is a candidate list. Every verdict is then taken at the bytes by hand;
this script narrows, it does not rule.
"""
import json
import pathlib
import re
import sys

ROOTS = ["crates/beamr/src", "crates/beamr-wasm/src"]

# Which reading of R6's "a call which can reach `gc::ensure_space`
# (transitively: ...)" to use.
#
#   direct  -- the parenthetical is a CLOSED ENUMERATION of the allocation
#              primitives. This is the reading R6 specifies and the one that
#              produced the committed candidates.json.
#   closure -- read the parenthetical as an instruction to close a call graph.
#              Kept ONLY so the ceiling it produces stays re-derivable: it
#              marks 2,985 of 5,030 production fns as "can collect" and yields
#              1,630 candidates. 59% of a crate is not a finding.
#
# This was previously applied by hand-editing the script between runs, so the
# committed instrument did not reproduce the committed output. See the audit's
# correction C-iv.
COLLECTING_MODE = "direct"

FN_RE = re.compile(
    r'^(?P<indent>\s*)'
    r'(?:pub(?:\([^)]*\))?\s+)?'
    r'(?:default\s+)?(?:async\s+)?(?:const\s+)?(?:unsafe\s+)?'
    r'(?:extern\s+"[^"]*"\s+)?'
    r'fn\s+(?P<name>\w+)'
)
MOD_RE = re.compile(r'^(?P<indent>\s*)(?:pub(?:\([^)]*\))?\s+)?mod\s+(?P<name>\w+)\s*\{')

# Seeds. jit_test_heap and alloc_words are the JIT-side entries; the
# ProcessContext::alloc_* family is added by discovery below.
SEEDS = {
    "ensure_space", "collect_minor", "collect_minor_with_live", "collect_major",
    "alloc_words", "alloc_words_rooted", "jit_test_heap",
}

# Immunity classes carried from AUDIT.md so the two records agree.
IMMUNE_SUFFIX = "_prereserved"


def files():
    out = []
    for root in ROOTS:
        out.extend(sorted(pathlib.Path(root).rglob("*.rs")))
    return out



FILE_TEST_MOD = re.compile(r'^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(?P<name>\w+)\s*;')


def test_files():
    """Files that ARE a cfg(test) module, declared `#[cfg(test)] mod x;`.

    A file-level declaration is a DIFFERENT SPELLING of the same thing as an
    inline `#[cfg(test)] mod x { .. }`, and indexing only the inline form
    silently counts 39 whole test files as production code.
    """
    marked = set()
    for p in files():
        lines = p.read_text().splitlines()
        pending = False
        for line in lines:
            st = line.strip()
            if st.startswith("#[cfg(test)]"):
                pending = True
                continue
            m = FILE_TEST_MOD.match(line)
            if m and pending:
                base = p.parent if p.name == "mod.rs" or p.name == "lib.rs" else p.parent / p.stem
                for cand in (base / f"{m.group('name')}.rs", base / m.group("name") / "mod.rs"):
                    if cand.exists():
                        marked.add(str(cand))
                pending = False
                continue
            if st and not st.startswith(("#[", "//", "///", "//!")):
                pending = False
    return marked


def index_file(path):
    """Return (fns, test_spans). fns = list of dicts."""
    lines = path.read_text().splitlines()
    fns, test_spans = [], []
    pending_cfg_test = False
    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith("#[cfg(test)]"):
            pending_cfg_test = True
            continue
        m = MOD_RE.match(line)
        if m and pending_cfg_test:
            end = close_of(lines, i, m.group("indent"))
            test_spans.append((i, end))
            pending_cfg_test = False
            continue
        m = FN_RE.match(line)
        if m:
            end = close_of(lines, i, m.group("indent"))
            fns.append({
                "file": str(path), "name": m.group("name"),
                "start": i, "end": end,
                "body": "\n".join(lines[i:end + 1]),
            })
            pending_cfg_test = False
            continue
        if stripped and not stripped.startswith(("#[", "//", "///", "//!")):
            pending_cfg_test = False
    return fns, test_spans


def close_of(lines, start, indent):
    """Line index of the closing brace at `indent`, or EOF."""
    target = indent + "}"
    for j in range(start + 1, len(lines)):
        if lines[j].rstrip() == target or lines[j].rstrip() == target + ";":
            return j
    return len(lines) - 1


def main():
    all_fns, test_map = [], {}
    for p in files():
        fns, spans = index_file(p)
        all_fns.extend(fns)
        test_map[str(p)] = spans

    whole_test_files = test_files()

    def in_test(fn):
        if fn["file"] in whole_test_files:
            return True
        for a, b in test_map.get(fn["file"], []):
            if a <= fn["start"] <= b:
                return True
        return False

    for fn in all_fns:
        fn["test"] = in_test(fn)

    # Discover the ProcessContext::alloc_* family (non-_prereserved).
    alloc_family = {
        fn["name"] for fn in all_fns
        if fn["name"].startswith("alloc_")
        and not fn["name"].endswith(IMMUNE_SUFFIX)
        and not fn["test"]
    }
    collecting = set(SEEDS) | alloc_family
    collecting -= {n for n in collecting if n.endswith(IMMUNE_SUFFIX)}

    # Names ruled out at their definition bytes -- size calculations, bump-only
    # region allocators, and test helpers that a name match took for
    # allocators. Loaded from the committed record rather than inlined so the
    # ruling and its reason travel together.
    exclusions = json.loads((pathlib.Path(__file__).parent
                             / "family-exclusions.json").read_text())
    # Positive control: an excluded name that no longer exists in the tree is a
    # SILENT no-op, and a typo is indistinguishable from a correct ruling. Fail
    # loudly instead -- the exclusion list must still bite.
    indexed_names = {fn["name"] for fn in all_fns}
    inert = sorted(set(exclusions) - indexed_names)
    if inert:
        print(f"EXCLUSIONS NO LONGER PRESENT IN TREE: {inert}", file=sys.stderr)
        sys.exit(2)
    collecting -= set(exclusions)

    # Precompute the set of callee NAMES per production fn, once. The fixpoint
    # is then set intersection rather than a regex per (fn, target) pair.
    CALL_RE = re.compile(r'\b(\w+)\s*\(')
    prod = [fn for fn in all_fns if not fn["test"]]
    for fn in prod:
        fn["calls"] = set(CALL_RE.findall(fn["body"])) - {fn["name"]}

    mode = sys.argv[2] if len(sys.argv) > 2 else COLLECTING_MODE
    if mode not in ("direct", "closure"):
        print(f"unknown collecting mode: {mode}", file=sys.stderr)
        sys.exit(2)

    rounds = 0
    while mode == "closure":
        rounds += 1
        added = {fn["name"] for fn in prod
                 if fn["name"] not in collecting and (fn["calls"] & collecting)}
        if not added:
            break
        collecting |= added
        if rounds > 60:
            print("FIXPOINT DID NOT CONVERGE", file=sys.stderr)
            break

    print(f"files indexed            : {len(files())}")
    print(f"fns indexed (all)        : {len(all_fns)}")
    print(f"whole cfg(test) FILES     : {len(whole_test_files)}")
    print(f"fns in cfg(test)         : {sum(1 for f in all_fns if f['test'])}")
    print(f"fns in production        : {len(prod)}")
    print(f"seed names               : {len(SEEDS)}")
    print(f"alloc_* family (non-pre) : {len(alloc_family)}")
    print(f"family exclusions ruled  : {len(exclusions)}")
    print(f"collecting mode          : {mode}")
    print(f"COLLECTING set           : {len(collecting)}"
          + (f"  (fixpoint in {rounds} rounds)" if mode == "closure" else "  (closed list)"))

    out = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "r6-stage1.json")
    out.write_text(json.dumps({
        "collecting": sorted(collecting),
        "alloc_family": sorted(alloc_family),
        "fns": [{k: v for k, v in f.items() if k not in ("body", "calls")} for f in all_fns],
    }, indent=1))
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
