#!/usr/bin/env python3
"""merge.py <tier> <module.beam> <probe.json> <otp.tsv> <cli.txt> <cli.rc> <DECODER-SET.txt> <EXECUTED-SET.txt> <T3-REACH.tsv> <out.json>
Merges the beamr probe's JSON with the OTP-side walker TSV into the brief's fixed per-module schema (R4)."""
import json, sys, hashlib, os
tier, beam, probe_p, otp_p, cli_p, cli_rc, dec_p, exe_p, t3_p, out_p = sys.argv[1:11]
probe = json.load(open(probe_p))
decoder = {int(l.split()[0]) for l in open(dec_p) if l.strip() and not l.startswith('#')}
exe_lines = [l.strip() for l in open(exe_p) if l.strip() and not l.startswith('#')]
executed = set(exe_lines)
comp_line = [l for l in open(exe_p) if 'complement' in l]
complement = [x.strip() for x in comp_line[0].split('=')[1].split(',')] if comp_line else []
t3 = {}
for l in open(t3_p):
    p = l.rstrip('\n').split('\t')
    if p[0] != 'module': t3[p[0]] = p[1]
otp = {'op': {}, 'impt': [], 'callext': {}, 'walk': None, 'refused': None}
for l in open(otp_p):
    p = l.rstrip('\n').split('\t')
    if p[0] == 'op': otp['op'][int(p[1])] = (p[2], int(p[3]))
    elif p[0] == 'impt': otp['impt'].append(p[1])
    elif p[0] == 'callext': otp['callext'][p[1]] = int(p[2])
    elif p[0] == 'walk':
        otp['walk'] = p[1]
        if p[1] == 'refused': otp['refused'] = int(p[2])
undecoded = sorted(o for o in otp['op'] if o not in decoder)
if otp['refused'] is not None and otp['refused'] not in undecoded: undecoded.append(otp['refused'])
undec_total = sum(otp['op'][o][1] for o in undecoded if o in otp['op']) + (1 if otp['refused'] is not None else 0)
ls = probe.get('load_status')
g = probe.get('generic') or {}
def cnt(op): return otp['op'].get(op, ('', 0))[1]
if ls == 'loaded':
    ok = (not undecoded) and cnt(67) == g.get('set_tuple_element', 0) and cnt(183) == g.get('executable_line', 0) and cnt(184) == g.get('debug_line', 0)
    agreement = 'ok' if ok else f"disagree: loaded but walker undecoded={undecoded} or generic counts walker(67,183,184)=({cnt(67)},{cnt(183)},{cnt(184)}) vs probe=({g.get('set_tuple_element',0)},{g.get('executable_line',0)},{g.get('debug_line',0)})"
elif ls == 'decode-failed':
    n = probe.get('first_refused_opcode')
    ok = bool(undecoded) and n in undecoded
    agreement = 'ok' if ok else f"disagree: decode-failed at {n} but walker undecoded={undecoded}"
else:
    agreement = f"disagree: load_status={ls} ({probe.get('load_error')})"
sha = hashlib.sha256(open(beam, 'rb').read()).hexdigest()
cli_lines = set(l.strip() for l in open(cli_p) if l.strip() and not l.startswith('beamr:'))
if ls == 'loaded':
    unresolved = sorted(probe.get('unresolved', [])); deferred = sorted(probe.get('deferred', [])); denied = sorted(probe.get('denied', []))
    C = {"unresolved": [{"mfa": m, "calls": otp['callext'].get(m, 0)} for m in unresolved],
         "deferred": len(deferred), "deferred_names": deferred, "denied": denied,
         "cli_rc": int(cli_rc), "cli_stream_equal": cli_lines == set(unresolved) | set(deferred),
         "cli_only": sorted(cli_lines - (set(unresolved) | set(deferred))), "probe_only": sorted((set(unresolved) | set(deferred)) - cli_lines)}
    reached = sorted({m.split(':')[0] for m in otp['impt']} & set(t3))
    D = [{"module": m, "status": None} for m in reached]  # status filled by status.py from the base registry
    B = {"by_variant": {v: 0 for v in complement}, "total": 0, "note": "complement variants are never produced by the decoder at this base (no numeric arm builds Badrecord/NifStart); counted 0 by construction"}
else:
    C = "unmeasurable: load"; D = "unmeasurable: load"; B = "unmeasurable: load"
out = {"module": probe.get('module') or os.path.basename(beam)[:-5], "tier": tier, "path": beam, "sha256": sha, "bytes": os.path.getsize(beam),
       "load_status": ls, "first_refused_opcode": probe.get('first_refused_opcode'), "load_error": probe.get('load_error'),
       "A": {"generic": {"set_tuple_element": {"count": g.get('set_tuple_element', 0), "at_execute": "raises"},
                         "executable_line": {"count": g.get('executable_line', 0), "at_execute": "no-op"},
                         "debug_line": {"count": g.get('debug_line', 0), "at_execute": "raises"}},
             "instruction_count": probe.get('instruction_count'),
             "disasm_undecoded": {"distinct": undecoded, "names": {str(o): (otp['op'][o][0] if o in otp['op'] else 'walker-refused') for o in undecoded}, "total": undec_total},
             "walker": otp['walk'], "agreement": agreement},
       "B": B, "C": C, "D": D, "impt_modules": sorted({m.split(':')[0] for m in otp['impt']}),
       "registration_deviation": probe.get('registration_deviation', []),
       "skipped_in_dirs": probe.get('skipped_in_dirs'), "dirs": len(probe.get('dirs', []))}
json.dump(out, open(out_p, 'w'), indent=1, sort_keys=True); open(out_p, 'a').write('\n')
print(f"{tier}\t{out['module']}\t{ls}\t{out['first_refused_opcode']}\t{agreement[:40]}")
