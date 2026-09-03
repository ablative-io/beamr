#!/usr/bin/env python3
"""cprime-report.py <outC> <out> <erts-modules.txt> — Supplementary C: per tier, calls into erts-preloaded modules with no
registered native (deferred under those targets when erts ebin is NOT loaded), by MFA with C6 call counts; plus unresolved."""
import json, glob, sys, collections, os
outC, out, ertsp = sys.argv[1:4]
erts = {l.strip() for l in open(ertsp) if l.strip()}
def callext(tier, m):
    p = f"{out}/{tier}/raw/{m}.otp.tsv"; d = {}
    if os.path.exists(p):
        for l in open(p):
            f = l.rstrip('\n').split('\t')
            if f[0] == 'callext': d[f[1]] = int(f[2])
    return d
print("\n## Supplementary C — missing natives with the erts ebin NOT loaded (`tools/cprime.sh`, `outC/`)\n")
print("`--dir` = the 41 minus `erts-17.0.5/ebin`. A call whose target module is one of the erts-preloaded modules and has no\nregistered native lands in `deferred` under that target; those are listed. `unresolved` here = target module loaded but export\nabsent. Call counts from C6 (`call_ext*` occurrences in the calling module).\n")
grand = collections.Counter(); grand_calls = collections.Counter(); grand_mods = collections.defaultdict(set)
for tier in ['T1','T2','T3','T4']:
    files = sorted(glob.glob(f"{outC}/{tier}/*.probe.json"))
    if not files: print(f"### {tier} — ABSENT\n"); continue
    names = collections.Counter(); calls = collections.Counter(); mods_with = 0; loaded = 0; n = 0; unres = collections.Counter(); bytarget = collections.defaultdict(set); bytarget_calls = collections.Counter()
    for f in files:
        n += 1
        try: j = json.load(open(f))
        except Exception as e: print(f"PARSE FAIL {f}: {e}"); continue
        if j.get('load_status') != 'loaded': continue
        loaded += 1; m = j['module']; ce = callext(tier, m); hit = False
        for mfa in (j.get('deferred') or []):
            t = mfa.split(':')[0]
            if t in erts:
                hit = True; names[mfa] += 1; c = ce.get(mfa, 0); calls[mfa] += c; bytarget[t].add(mfa); bytarget_calls[t] += c
                grand[mfa] += 1; grand_calls[mfa] += c; grand_mods[mfa].add(m)
        for mfa in (j.get('unresolved') or []): unres[mfa] += 1
        mods_with += hit
    print(f"### {tier} — {n} modules, {loaded} loaded; **{len(names)} distinct missing-native MFAs, {sum(calls.values())} call sites, in {mods_with} modules**\n")
    print("| target module | distinct MFAs | call sites |\n|---|---|---|")
    for t, c in bytarget_calls.most_common(): print(f"| `{t}` | {len(bytarget[t])} | {c} |")
    top = calls.most_common(40)
    if top: print(f"\nTop MFAs by call sites ({tier}): " + '; '.join(f"`{m}` ×{c} ({names[m]} mod)" for m, c in top))
    if unres: print(f"\nunresolved (module loaded, export absent) ({tier}): " + '; '.join(f"`{m}` ({c} mod)" for m, c in unres.most_common(30)))
    print()
print(f"### All tiers — **{len(grand)} distinct missing-native MFAs, {sum(grand_calls.values())} call sites**\n")
print("| MFA | call sites | calling modules |\n|---|---|---|")
for m, c in sorted(grand_calls.items(), key=lambda kv: (-kv[1], kv[0])): print(f"| `{m}` | {c} | {len(grand_mods[m])} |")
