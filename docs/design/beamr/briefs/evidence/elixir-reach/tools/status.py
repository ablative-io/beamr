#!/usr/bin/env python3
"""status.py <out-dir> <NATIVE-MODULES.tsv> — fills D[].status in every per-module JSON:
native-stub (module has >=1 native registration at the base), loaded-bytecode (the OTP module's own T3 row is loaded), absent (neither)."""
import json, sys, glob, os
out, nat_p = sys.argv[1:3]
native = {l.split(':')[0] for l in open(nat_p) if l.strip() and not l.startswith('#')}
t3 = {}
for f in glob.glob(f"{out}/T3/*.json"):
    j = json.load(open(f)); t3[j['module']] = j['load_status']
n = 0
for f in glob.glob(f"{out}/T*/*.json"):
    j = json.load(open(f))
    if isinstance(j.get('D'), list):
        for d in j['D']:
            m = d['module']
            d['status'] = 'native-stub' if m in native else ('loaded-bytecode' if t3.get(m) == 'loaded' else 'absent')
            d['t3_load_status'] = t3.get(m)
        json.dump(j, open(f, 'w'), indent=1, sort_keys=True); open(f, 'a').write('\n'); n += 1
print(f"filled D status in {n} files; native modules={len(native)}; t3 rows={len(t3)}")
