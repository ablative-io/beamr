# Currency release 0.19.1 — results at the bytes

Dispatched under Tom's estate-wide word (2026-08-18, relayed ~06:3xZ):
"everything needs to be on the latests, no exceptions." Lane + publish
pre-authorized by Waffles (dm d04abab2): patch if the public API is
untouched, measured not asserted.

## Published set (verified at the sparse index, independent of the publisher)

| crate | version | checksum (sha256) | note |
|---|---|---|---|
| gleam-types | 0.4.4 | `d584956cc629238409467c899f7fb219bb60bfb78afef3a715842c6bed87f9e2` | ecow edge DELETED; deps now camino only |
| beamr | 0.19.1 | `bd7d2a8b408452efead5933fb672bee795b64666873f2aa6dbf717451c8b2dd7` | reqs verified in index: gleam-types ^0.4.4, cranelift ^0.134.3, base64 ^0.23.1 |

beamr-wasm 0.9.0 and beamr-cli 0.6.0 are NOT republished: their `^0.19.0`
admits 0.19.1, so consumers pick the patch up passively. Publish order was
gleam-types first — beamr's dry-run strips the path dep and needs 0.4.4 on
the registry to resolve.

## Why 0.19.1 and not 0.20.0

The dep-major moves (cranelift 0.131→0.134, base64 0.22→0.23, ecow deletion)
are all internal. Measured: no cranelift/base64/ecow type is nameable from
beamr's public surface (single-line and multi-line pub-signature sweeps; the
two `JITModule` occurrences inside public structs are private fields), and
gleam-types' API is untouched by removing a dependency nothing referenced.
The cranelift adaptation is semantics-preserving by construction: the
deprecated `*_imm` builders were replaced by their `_s` (sign-extending)
variants, which reproduce the old `Imm64` behaviour — per-site `_u`
"improvements" were deliberately not made.

## The ecow finding (census residue, banked)

The workspace pin `ecow = "=0.2.6"` ("pin ecow for gleam-core
compatibility", 4765a34, 2026-06-09) guarded a DEAD EDGE: zero
ecow/EcoString references anywhere in gleam-types (usage removed in later
work; the manifest edge survived), compile-proven clean without it. Upstream
gleam-lang/gleam's lock is on ecow 0.3.0 anyway. Deleted, not bumped: a pin
guarding an edge no code uses is a shield, not a constraint. The published
gleam-types 0.4.3 declaring the stale exact pin was itself the
stale-requirement defect class one level down; 0.4.4 discharges it.

## Battery (pin a415810, priors pre-registered in CONTENTION.md)

COMPLETE 9/9 rc 0, graded by ci-verdict.sh (self-test 7/7 before grading).
Axes EXACT against the pre-registered priors: leg4 2/86/0/0 · leg5
76/2150/0/0 · leg8 76/2160/0/0 (result-lines/passed/failed/ignored).
nostd-ratchet: tally 1072, exactly at ceiling — the bumps moved nothing in
the no_std population. Neither named risk fired (wasm-bindgen runner skew;
ratchet shift). Run contended (another cargo held the package cache earlier
in the hour); timings are NOT price points.

## Census provenance

gate-logs sibling of this run; census instrument ran control-first (all four
verdict arms proven on minted inputs before the real run), population 35
unique (crate, req) direct-dep pairs, totals sum. Residues NOT discharged by
this lane: duplicate in-tree resolutions getrandom {0.3.4, 0.4.2} and rand
{0.9.4, 0.10.1} — held by transitive edges, named for upstream filing.

## Tags

`beamr-v0.19.1` and `gleam-types-v0.4.4`, annotated with the checksums
above, at the release-state commit a415810 (the receipts commit after it is
gate-logs only). Per-crate scheme per Waffles' ruling 2026-08-18.
