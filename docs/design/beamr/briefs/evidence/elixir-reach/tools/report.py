#!/usr/bin/env python3
"""report.py <out-dir> <T3-REACH.tsv> — prints the per-tier report sections (markdown) from the merged JSON.
Order per tier: first_refused_opcode histogram FIRST, then the tier table, then top-20 unresolved targets, then D."""
import json, glob, sys, collections, os
out, t3p = sys.argv[1:3]
names = {185:'bif3/6',186:'is_any_native_record/2',187:'is_native_record/4',188:'get_record_elements/3',189:'put_record/6',190:'is_record_accessible/3',191:'get_record_field/5'}
for tier in ['T1','T2','T3','T4']:
    files = sorted(glob.glob(f"{out}/{tier}/*.json"))
    if not files: print(f"\n## {tier} — ABSENT (no merged rows)\n"); continue
    rows = [json.load(open(f)) for f in files]
    n = len(rows); ls = collections.Counter(r['load_status'] for r in rows)
    hist = collections.Counter(r['first_refused_opcode'] for r in rows if r['load_status']=='decode-failed')
    lf = [r for r in rows if r['load_status']=='load-failed']
    print(f"\n## {tier} — {n} modules · load_status: " + ', '.join(f"{k} {v}" for k,v in sorted(ls.items())))
    print(f"\n**first_refused_opcode histogram ({tier})** — modules whose load beamr refuses, by the first opcode it refuses:\n")
    print("| opcode | OTP 29 name | modules |\n|---|---|---|")
    if hist:
        for op,c in sorted(hist.items()): print(f"| {op} | `{names.get(op,'?')}` | {c} |")
    else: print("| — | — | 0 (every module in this tier decodes) |")
    if lf:
        errs = collections.Counter((r['load_error'] or '')[:60] for r in lf)
        print(f"\nload-failed (not a decode refusal), by error: " + '; '.join(f"`{e}` ×{c}" for e,c in errs.items()))
    loaded = [r for r in rows if r['load_status']=='loaded']
    gen = collections.Counter(); gen_mods = collections.Counter()
    for r in loaded:
        for k,v in r['A']['generic'].items():
            if v['count']: gen[k]+=v['count']; gen_mods[k]+=1
    dis = [r for r in rows if not r['A']['agreement'].startswith('ok')]
    unres_names = collections.Counter(); unres_calls = collections.Counter(); unres_mods = 0; deferred = 0; denied = 0; cli_neq = 0
    by_target = collections.Counter(); by_target_calls = collections.Counter()
    for r in loaded:
        C = r['C']
        if not isinstance(C, dict): continue
        if C['unresolved']: unres_mods += 1
        for u in C['unresolved']:
            unres_names[u['mfa']] += 1; unres_calls[u['mfa']] += u['calls']
            t = u['mfa'].split(':')[0]; by_target[t] += 1; by_target_calls[t] += u['calls']
        deferred += C['deferred']; denied += len(C['denied']); cli_neq += (0 if C['cli_stream_equal'] else 1)
    inst = sum(r['A']['instruction_count'] or 0 for r in loaded)
    print(f"\n| modules | decode-failed | load-failed | loaded | instructions (loaded) | A generic: 67 / 183 / 184 occurrences (modules) | B (complement) | C unresolved: distinct MFAs / total calls / modules with ≥1 | C unmeasurable | deferred (sum) | denied (sum) | CLI stream ≠ probe |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|")
    print(f"| {n} | {ls.get('decode-failed',0)} | {ls.get('load-failed',0)} | {ls.get('loaded',0)} | {inst} | {gen['set_tuple_element']} ({gen_mods['set_tuple_element']}) / {gen['executable_line']} ({gen_mods['executable_line']}) / {gen['debug_line']} ({gen_mods['debug_line']}) | 0 by construction | {len(unres_names)} / {sum(unres_calls.values())} / {unres_mods} | {n-len(loaded)} | {deferred} | {denied} | {cli_neq} |")
    print(f"\n**C — unresolved natives by target module ({tier}, top 20 by calls)**: " + ('; '.join(f"`{t}` {by_target[t]} names / {by_target_calls[t]} calls" for t,_ in by_target_calls.most_common(20)) if by_target else "none"))
    top = unres_calls.most_common(25)
    if top: print(f"\n**C — top unresolved MFAs ({tier})**: " + '; '.join(f"`{m}` ×{c} ({unres_names[m]} mod)" for m,c in top))
    D = collections.Counter(); Dst = collections.defaultdict(set)
    for r in loaded:
        if isinstance(r['D'], list):
            for d in r['D']: D[d['module']] += 1; Dst[d['module']].add(d.get('status') or '?')
    if D: print(f"\n**D — OTP behaviour/facility modules reached ({tier}, module → importing modules, beamr status)**: " + '; '.join(f"`{m}` {c} [{'/'.join(sorted(Dst[m]))}]" for m,c in D.most_common(40)))
    print(f"\n**Disagreements ({tier})**: " + ('; '.join(f"`{r['module']}`: {r['A']['agreement']}" for r in dis) if dis else "(none)"))
