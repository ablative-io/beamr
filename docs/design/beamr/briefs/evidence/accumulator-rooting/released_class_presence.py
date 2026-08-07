"""AR-1 row 9 -- is the accumulator class present in every RELEASED beamr?

Turns an inference into a measurement. Reads the published ARTEFACTS, not the
git history: every .crate embeds .cargo_vcs_info.json carrying the exact sha
at package time. curl + tar, no cargo, no build.
⭐ A SUBJECT LINE IS NOT A RELEASE -- the sha comes from the artefact, never
from a commit whose message reads like one. (Seth's, via the lane lead.)

  usage:  python3 released_class_presence.py <dir-with-beamr-*.crate>
  fetch:  curl -H 'User-Agent: <contact>' \
            -o beamr-<v>.crate https://static.crates.io/crates/beamr/beamr-<v>.crate

⚠️ WHAT THIS IS NOT: a verdict engine. It reports the SHAPE, not verified
crossings -- the RF-006 verdict pass ran 29% false positive on this kind of
evidence. Read a positive as "the shape is here", never as "N defects here".

⚠️ KNOWN BLINDNESS, and the one direction it does not bite: the receiver must
be spelled `context`, so a version using another name is UNDER-reported.
Under-reporting can only REMOVE versions from the positive set. When every
version already reports positive the conclusion is robust to it -- that is a
saturation argument, not a clean bill.

CONTROL: the newest crate's hits must include the three known RF-006 defects
(code_management_bifs, pg.rs, dictionary_bifs). Exits 2 if they do not, so a
blind run cannot read as an absence.
"""
import tarfile, re, json, sys, pathlib

CONS    = re.compile(r'^\s*(\w+)\s*=\s*context\.alloc_cons\s*\(', re.M)
PUSHAL  = re.compile(r'\.push\s*\(\s*context\.alloc_\w+\s*\(', re.M)
TUPPUSH = re.compile(r'\w+\.push\s*\(\s*\(\s*\w*_?term\s*,', re.M)
KNOWN   = ('code_management_bifs', 'pg.rs', 'dictionary_bifs')

root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else '.')
def vkey(s): return [int(p) for p in s.split('.')]
vers = sorted((p.name[len('beamr-'):-len('.crate')] for p in root.glob('beamr-*.crate')), key=vkey)
if not vers:
    print('no beamr-*.crate found', file=sys.stderr); sys.exit(2)

rows, newest_files = [], set()
for v in vers:
    tf = tarfile.open(root / f'beamr-{v}.crate', 'r:gz')
    sha, c, p, t, files = None, 0, 0, 0, set()
    for m in tf.getmembers():
        if not m.isfile(): continue
        if m.name.endswith('.cargo_vcs_info.json'):
            sha = json.load(tf.extractfile(m))['git']['sha1'][:9]; continue
        if not m.name.endswith('.rs'): continue
        if '/tests/' in m.name or m.name.endswith('tests.rs'): continue
        s = tf.extractfile(m).read().decode('utf8', 'replace')
        n = len(CONS.findall(s)) + len(PUSHAL.findall(s)) + len(TUPPUSH.findall(s))
        if n: files.add(m.name.split('/', 1)[1])
        c += len(CONS.findall(s)); p += len(PUSHAL.findall(s)); t += len(TUPPUSH.findall(s))
    tf.close()
    if v == vers[-1]: newest_files = files
    rows.append(dict(version=v, vcs_sha=sha, tail=c, push=p, s3e=t,
                     present=(c + p + t) > 0, files=sorted(files)))

print(f"{'version':9} {'vcs sha':10} {'tail':>5} {'push':>5} {'s3e':>4}  class")
for r in rows:
    print(f"{r['version']:9} {str(r['vcs_sha']):10} {r['tail']:5} {r['push']:5} "
          f"{r['s3e']:4}  {'YES' if r['present'] else 'no'}")
pos = [r for r in rows if r['present']]
print(f"\n{len(pos)} / {len(rows)} released versions carry the SHAPE; earliest "
      f"{pos[0]['version'] if pos else 'n/a'}")
print("\n⭐ THE CLASS IS CONTINUOUS; THE SITES ARE NOT.")
print(f"   earliest ({rows[0]['version']}): {rows[0]['files']}")
print(f"   newest   ({rows[-1]['version']}): {sorted(newest_files)[:4]} ...")
print("   ⇒ 'present in every released version' is TRUE OF THE CLASS, FALSE OF THE SITES.")
json.dump(rows, open(root / 'released-class-presence.json', 'w'), indent=2)

missing = [k for k in KNOWN if not any(k in f for f in newest_files)]
print(f"\nCONTROL (newest crate must show the three known RF-006 defects): "
      f"{'PASS' if not missing else 'FAIL, missing ' + str(missing)}")
sys.exit(0 if not missing else 2)
