# AION-ENCODE-GC-DEFECT — signature repro run record

**Lane:** AION-ENCODE-GC-DEFECT (Tom via Vesper Lynd, 2026-07-23; ratified by
Tom 2026-07-23 21:01Z). Build seat: Osiris Yogo. Domain owner/tear: Artemis
Peach. Landing: Waffles the Terrible. This directory is committed lane
evidence, **not a permanent gate**: registry-locked artifacts age, and gates
bind tree bytes — the harness never joins `gates.json` or the workflow.
Permanent coverage is the in-tree walls (the C1 collision wall
`boolean_list_cons_is_not_walked_as_a_refcounted_binary`, the
accounting-sanity wall, and the multibyte-encode walls — Artemis's tear).

## What is proven

The production defect — aion workflow `2062659a-afd4-4b9d-820c-d675bd29d5ed`
crashing `badarg` in `json:encode_binary` on fully valid content at
2026-07-23 19:31:42, on a host running beamr **0.16.0** cargo-installed from
aion's `Cargo.lock`, co-resident with ~25 GB resident memory — is the C1
GC release-walk defect (REVIEW-23-07, fixed in beamr **0.16.2**), proven
**at the deployment artifacts** (Waffles' bar): the legs pin exact registry
versions in committed lockfiles, not checked-out source.

- **RED** — `red/` (beamr `=0.16.0`, checksum `2e6413f8…`): deterministic
  crash, 7/7 runs (4 release, plus debug), in under 10 ms, at the first
  minor collections. See `runs/red-runs.txt`.
- **GREEN** — `green/` (beamr `=0.16.2`, checksum `c4860a45…`): the same
  harness body and fixture complete all 25,000 iterations, exit
  `Normal`, in ~103 s, **maximum resident set 934 MB** against ~1.9 GB of
  cumulative fresh-ProcBin churn — bounded, flat memory. The green leg
  covers **both** production symptoms: no crash AND no unbounded
  residency. See `runs/green-run.txt`.

## The backtrace ties expression to mechanism at the locked artifact

`runs/red-segv-backtrace.txt` (macOS crash report, debug-build run, same
lock): the fault is `EXC_BAD_ACCESS / KERN_INVALID_ADDRESS at 0x21` with
**`x2 = 0x19`** — the atom `false` raw encoding — live as the pointer being
dereferenced (`0x21` = `0x19` + the 8-byte offset of `Vec::len`'s field).
The frames, innermost out:

```
Vec::len
release_proc_bin_arc                      ← the misread release
release_refcounted_resources_in_young
HeapRegion::visit_allocated_boxed_objects ← the word[0]-inference visitor
minor::collect ← ensure_space ← alloc_binary
bif_json_encode_binary                    ← the production entry point
```

The crash fires **inside `json:encode_binary`** — the exact BIF that
badarg'd on the crash host — when its result allocation triggers a minor
GC whose release walk misreads a `[false | _]` cons at an allocation start
as a ProcBin and dereferences the atom's raw value as an `Arc` data
pointer. This retires the objection that production's badarg might have
been a different 0.16.0 bug: expression and mechanism are recorded fact at
the locked artifact.

## Expression difference, stated plainly (ruling of record: Artemis, 2026-07-23)

The repro expresses the corruption as **SIGSEGV**; production expressed
**badarg** (plus collateral: the co-resident workflow `1edfa1ec…` failed 8
minutes later with "engine NIF state is not installed" — the host's own
proof that the arbitrary free lands beyond the badarg). The defect class
is arbitrary-memory corruption from a misread release walk; **the
downstream expression is allocation-layout luck — SEGV and badarg are
dice faces of the same die.** The bool-dense fixture always rolls the
fatal face first (a misread cons whose neighbour word is a small raw
value, dereferenced immediately); the production heap's sparser dice
rolled the survivable face (mapped pointer-valued neighbour → bogus
refcount write + zeroed term slot + garbage `saturating_sub` into the
virtual-binary-heap pacing counter → downstream badarg on a
valid-when-constructed term, and binary-GC pacing silently zeroed —
the ~25 GB residency). A deterministic red is strictly stronger evidence
than a fixture tuned toward one benign-looking face; tuning toward badarg
would be layout-manufactured and brittle across allocator versions.

`fixture/encode_gc_repro_badarg.erl` + `runs/sparse-negative-runs.txt` is
the **recorded negative**: a sparse-trigger shape aimed at the gentle
expression completes normally at 0.16.0 (3/3). It documents that the
gentle expression needs layout conditions this shape does not
manufacture. **It does not document 0.16.0 ever being safe.**

**Banked question (not a lane gate):** what follows a false-headed cons in
young-region allocation order through the bytecode paths — the layout
condition selecting badarg over SEGV. If the badarg face ever needs
pinning, that is its own investigation with its own justification.

## Layout

- `main-body.rs` — shared harness body, included verbatim by both legs;
  mirrors aion's embedding (`Scheduler::with_code_server`, shared
  `AtomTable`/`BifRegistryImpl`, registration order gate1 → gate3 →
  stdlib stubs → gleam ffi → otp stubs).
- `fixture/encode_gc_repro.erl`(+`.beam`) — the L01-faithful bool-rich
  shape, derived from the verbatim crash payload (Vesper Lynd, from the
  norn session store of workflow `2062659a…`): eight 2-field claim
  records, gate outcome `pass=false` with per-run booleans
  true/true/false, three ~16 KB multibyte-seeded strings rebuilt per
  iteration as fresh ProcBins, all booleans runtime-computed (nothing
  constant-folds), a bounded cross-iteration boolean accumulator for
  old-generation coverage, unguarded positional encodes (the
  `gleam_json_ffi` trust shape). Asserts observable surface only.
- `fixture/encode_gc_repro_badarg.erl`(+`.beam`) — the recorded negative
  (above).
- `red/`, `green/` — one crate per leg, own committed `Cargo.lock`;
  builds in place at default target dirs.
- `runs/` — committed run outputs and the backtrace record. Committed
  once; corrections forward-only.

## Reproduce

```sh
(cd fixture && erlc encode_gc_repro.erl encode_gc_repro_badarg.erl)
(cd red   && cargo build --release && ./target/release/encode-gc-repro-red   ../fixture/encode_gc_repro.beam)
(cd green && cargo build --release && ./target/release/encode-gc-repro-green ../fixture/encode_gc_repro.beam)
```

## Environment (recorded at run time, 2026-07-24 AEST)

- macOS 26.5.2 (25F84), Darwin 25.5.0, Apple M5 Pro (arm64)
- rustc 1.95.0 (59807616e 2026-04-14), cargo 1.95.0
- erlc from Erlang/OTP 29 (erts 17.0.3), Homebrew
- beamr from crates.io: red `0.16.0` checksum `2e6413f8490d7d08…`,
  green `0.16.2` checksum `c4860a45d64945…` (full checksums in the
  committed lockfiles)
