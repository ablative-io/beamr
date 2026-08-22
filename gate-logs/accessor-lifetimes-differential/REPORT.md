# Accessor-lifetimes landing — independent differential, leg for leg

**Measured by:** Artemis Peach (beamr owner seat), 2026-08-22
**Venue:** rocketfish/omarchy, Linux 6.19.9, 16 cores, rustc 1.97.1 (repo-pinned)
**base:** `c55ac360eb573148ef868ce7eb7d03babf05523a` (origin/main at flight time)
**land:** `e1cb306052aa8519c49fc5941940614cd131ed45` (`fleet/accessor-lifetimes-result`)

Two fresh worktrees, created from a hash-verified bundle, each with its own
`target/`. No shared build cache, no `sccache`, no `CARGO_TARGET_DIR`. The
battery definition is byte-identical on both arms (`gates.json` sha256
`b5c9a50bfe4ede80da894d33be22a8b3bafd621193016566264201b7997a74f6`), so the
differential is well-posed. Leg commands were read from `gates.json` at runtime
and run verbatim — never restated.

## VERDICT: no leg differs. The claim holds on my own evidence.

| leg | base rc | land rc | agree |
|---|---|---|---|
| fmt | 0 | 0 | ✅ |
| clippy | 101 | 101 | ✅ |
| wasm32-check | 0 | 0 | ✅ |
| wasm-tests | 0 | 0 | ✅ |
| tests | 101 | 101 | ✅ |
| blocking-call-in-native-bif | 0 | 0 | ✅ |
| clippy-all-features | 101 | 101 | ✅ |
| tests-all-features | 101 | 101 | ✅ |
| nostd-ratchet | 3 | 3 | ✅ |

### The red set, verified one by one

1. **`mktemp` no-X template, `scripts/gate-nostd-ratchet.sh:182`** — CONFIRMED
   pre-existing, with mechanism. `mktemp -t nostd-ratchet` has no `X`s: BSD
   accepts it, GNU refuses with `too few X's in template`. Both arms rc=3, both
   hitting the script's own REFUSE arm. **Bigger than a red: this leg has never
   measured anything on any Linux venue, CI included.** It fails *safe* — the
   empty tally is caught and declines to report rather than passing. Cure
   `mktemp -t nostd-ratchet.XXXXXX` verified rc=0 on macOS and Linux. NOT landed;
   the lead is drafting these as typed board defects.
2. **15-lint drift** — CONFIRMED pre-existing. Multisets keyed on
   (level, lint, file, message) are **equal at 15 on both arms**, on both the
   `clippy` and `clippy-all-features` legs. The only movement is line drift:
   the 8 `clippy::unnecessary_cast` in `file_meta_bifs.rs` shift 251→253 etc.
   **A red that MOVED is not a red that CHANGED** — the key deliberately
   excludes line number, because the landing's ripple shifts lines and a
   line-keyed multiset would report a delta that is pure drift.
3. **`thread_inventory_distribution`** — CONFIRMED pre-existing and
   **deterministic, not load-dependent**. Fails on plain main with
   `dist-send=1, net-kernel=0`. Interleaved A/B/A/B, five rounds each arm at
   matched load: **10/10 FAIL, base and land alike.** Interleaving was the point
   — another fleet run was live on this box, and a thread-inventory test is
   exactly what contention perturbs. Sequential arms would have let scheduling
   order masquerade as a code difference.
4./5. **clippy-all-features twins** — CONFIRMED, fold into #2 as relayed.

### The hole I had to close in the battery itself

`tests` and `tests-all-features` carry no `--no-fail-fast`, so cargo stops at
the first failing target. Compared under truncation, "equal multisets" only
means "equal up to the first failure" — a land-only failure behind it would be
invisible, and the invisibility reads exactly like cleanliness. Re-run with
`--no-fail-fast` on both arms:

- **80 suites on each arm**, against 66 under truncation — truncation was
  hiding 14 suites.
- Failing-test multiset **identical**: exactly one failure, the same one.
- **Nothing lost**: zero tests pass on base and fail-or-vanish on land.
- The landing is **strictly additive**: +11 passing tests untruncated.

The one new test is
`term::accessor_proof_tests::every_compile_fail_proof_is_its_positive_control_plus_the_collection`,
and it is worth reading. It **measured** that `compile_fail,E0502` doctests are
not enforced on the pinned toolchain 1.97.1 — a proof block greens on *any*
compile error (a missing argument, a bad import) while still reading as "the
borrow is enforced". Deleting the `HeapBorrow` argument from one of the five
proofs left it green. The module closes that by asserting the `compile_fail`
block is the positive control plus exactly one line, and that the line is the
collection. That is a matched pair rather than an annotation taken on trust.

## The one substantive thing the gate cannot see

Every leg agrees, so nothing here reopens the ratification on the red set. But
the differential is blind to this, and it should be a decision rather than an
accident.

**The landing adds 20 `.to_vec()` and 2 `.to_owned()` calls on production
paths**, concentrated in:

| file | +to_vec | path |
|---|---|---|
| `crates/beamr/src/ets/copy.rs` | +5 | ETS insert (`copy_boxed_to_heap`) |
| `crates/beamr/src/interpreter/opcodes/binary/matching.rs` | +4 | `bs_get_binary`, `bs_get_tail`, `run_command_args` — no `cfg(test)` in the file at all |
| `crates/beamr/src/mailbox/mod.rs` | +3 | `copy_binary`, `copy_bigint`, `copy_sub_binary` — the message-send path |

Mechanism, read from the code rather than inferred. Base:

```rust
let bytes = binary.as_bytes();                    // borrows the heap
let words = alloc_words(heap, ...)?;              // needs &mut heap  <- E0502
crate::term::binary::write_binary(words, bytes)
```

Land:

```rust
// Own the bytes before allocating: see `copy_bigint`.
let bytes = binary.as_bytes(heap.borrow_terms()).to_vec();
let words = alloc_words(heap, ...)?;
crate::term::binary::write_binary(words, &bytes)
```

The borrow cannot span the heap mutation, so the bytes are copied to a
temporary first. **heap→heap becomes heap→Vec→heap**: one extra allocation and
memcpy per binary and per bigint, on every inter-process message send, every
ETS insert, and every binary-match opcode that returns bytes.

This is a consequence of making the borrow sound, not gratuitous — the prior
code is what the landing exists to fix. Two honest caveats:

- **The magnitude is UNMEASURED.** I am not reporting a regression; I am
  reporting an unpriced change with a named mechanism and coordinates.
- **No leg in `gates.json` would ever see it** — there is no bench leg. beamr
  *does* carry two criterion targets (`jit_comparison`,
  `jit_comparison_extended`), but they benchmark JIT-vs-interpreter on
  fibonacci, list processing and pattern matching. **They do not exercise
  mailbox copy, ETS copy, or `bs_get_*` at all.** Running them and reporting
  green would be an asleep instrument: it would read as "no regression" while
  never touching a changed path. I deliberately did not do that.

A two-phase alternative exists (size first, allocate, then re-borrow and copy
straight in) that would avoid the temporary. Whether that is worth the
complexity is the estate's call, not mine.

## Venue drift worth recording

`wasm-bindgen-cli` on this box was **rewritten today at 09:00:37 +1000 and is
now 0.2.123**, against a lock requiring 0.2.127 — most likely for the
`framelen-flight1` run that was live during my measurement. The 0.2.127 reading
recorded for OB-004 on 2026-08-20T21:09Z was true when taken; the venue has
since drifted back to the skew that obligation existed to retire. Both arms saw
the same 0.2.123, so the differential is unaffected — but the *aligned venue is
no longer aligned*, and the next reader should not inherit "aligned" from the
obligation text.

Incidentally, `wasm-tests` returned **86 passed on both arms at 0.2.123**, the
same 86 measured at 0.2.127 — a third independent data point that this leg is
indifferent to that skew.

## Reproducing this

The three drivers are committed beside this report, so the measurement is
re-runnable rather than merely reported:

- `accessor-diff.sh` — makes the two worktrees, runs all nine legs verbatim
  from each arm's own `gates.json`, captures per-leg rc, timing and load.
- `accessor-diff-p2.sh` — the untruncated `--no-fail-fast` reruns and the
  interleaved A/B on `thread_inventory_distribution`.
- `accessor-normalize.py` — applies each JSON leg's own `extract` from
  `gates.json` via jq and diffs the multisets.

Raw per-leg output is committed here as `rawlogs.tar.gz` (414 818 bytes, 62
entries, ~2.5 MB uncompressed). It was initially left on the measuring host and
NOT committed -- a judgement the project lead reversed at teardown, on the
grounds that the structured artifacts are only half the record and the raw
bytes are what you want when a question arrives late. The host was cleared
immediately afterwards, so this archive is now the only copy. It was
hash-verified at both ends of the transfer and its integrity checked by full
decompression before the source was removed. Archive sha256
`9045427e3404324fe40366a49ad14242d546b022552142b610c359feca17b446`
(`accessor-diff-rawlogs.tar.gz`, 414 818 bytes). The four clippy captures that
carry the red set:

```
bb682119dcf8ec51bcff7d31b3ec808bbb0a0d79ed12384df8dd15b327cd7ef1  base.clippy.json
14860ba8aa1a128f97aa61c7368be8c9af624956fe23fbfd67b317a02a19f1e3  land.clippy.json
3dc8b777f39e847c3f2911b1df5603200675aff00a4146fed9ff58ab704fd701  base.clippy-all-features.json
1e7ac723bdee34ee11c49273d7e08e557c34b68211a0c87fee0b75d11f8ea1c5  land.clippy-all-features.json
```

## Delivery note

Meridian was unavailable throughout this sitting: the MCP bridge sends but has
no DM reader, two send attempts hung and one failed after 1800s, and the
`collective` CLI's local server refuses connections. This report is therefore
landed as repo evidence so it is citable by sha regardless of the message path,
rather than held until a channel comes back.

## Instrument notes (mine, for the laws file)

- **Dead pathspec, third occurrence — and I had the wrong mechanism twice.**
  `git diff ... -- $PATHS` returned empty with rc=0 and no stderr. It is not the
  glob form I blamed before: **this shell is zsh, which does not word-split
  unquoted parameter expansions**, so four paths became one pathspec containing
  spaces, matching nothing. Confirmed with an argument-count probe: `$PATHS`
  expands to 1 argument, not 4. The positive control ("does this selector match
  anything at all?") is what caught it; without it the empty result reads as a
  clean tree.
- A `local a="$1" d="$ROOT/$a"` in bash expands every word of the `local`
  command before assigning any of them, so `$a` is unbound under `set -u`. Cost
  one launch.
