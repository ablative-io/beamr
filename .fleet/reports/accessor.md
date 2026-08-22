Model: `claude-opus-5` — this seat's own identifier. Pin verified in-band, not
assumed: `env | grep ANTHROPIC_MODEL` → see §12. The harness reports the served
model as `claude-opus-5[1m]` (the 1M-context variant of the pinned model).

---

# REPORT — beamr accessor lifetimes, FIX SEAT (gate round 4)

Flight `beamracc-lifetimes-flight3`, branch `fleet/accessor-lifetimes-base`,
2026-08-22. Base: beamr `main` `c55ac360`. Tree graded: `1abfdad4`.
Gate handed to me: `.fleet/gates/RED-LATEST.md` — md5
`08f30b25c1b82c638f0cf77a88dd03e6`, **byte-identical to `.fleet/gates/red-2.md`**;
there is no `red-3.md`, so the entry has not been re-run by a gate seat since
round 2. Unedited, untouched.

Every claim below is **OBSERVED** (command output / `file:line`) or **REASONED**.
Round 3's report — which carries round 2's and the builder's inside it — is
preserved verbatim below mine. **I inherited no disposition.** Round 3 had
already dispositioned these five reds; a disposition taken from another seat is
not a measurement, so I took every one again: fresh base worktree, fresh target
directory, fresh canon, and — new at this seat — the compile-error green
**re-derived by `rustc` directly** instead of read off a rustdoc pass/fail bit,
plus a **miri differential** the earlier rounds did not run.

Full transcript: **`docs/evidence/beamr-accessor-gate-round4.txt`**.

## Outcome in one line

**All five gate reds are pre-existing on beamr `main`, re-measured at the base
by me from scratch, and all five stand. Nothing in scope was found red at this
seat, so under my disposition algorithm there was nothing to fix.** The gate is
still RED on the same five legs; under the workflow's grammar that arrives as
`gates_red/Failed`, and per the measured-red wall the differential is the
criterion — **no leg that is green at the base is red here, and no leg changed
its rc.**

---

## 1. DISPOSITION OF EVERY RED — re-measured at the base, by me, from scratch

`git worktree add --detach ../base-r4 c55ac360`, `CARGO_TARGET_DIR=../base-r4-target`
(never shared with the fix tree), `rustc 1.97.1` as pinned by `rust-toolchain.toml`.
Evidence file §1 and §3.

| # | Leg | rc BASE | rc HEAD | What I measured, and how I know it is not this flight | Disposition |
|---|---|---|---|---|---|
| 1 | `nostd-ratchet` | **3** | **3** | Base and HEAD logs **byte-identical** (`diff` → no output). Cause read at source: `scripts/gate-nostd-ratchet.sh:182` is `LOG="$(mktemp -t nostd-ratchet)"`; GNU coreutils rejects a `-t` template with no `X`s, so `LOG` is empty, `:184`'s redirect fails, **cargo never runs**, and the script's own REFUSE arm fires correctly on an absent tally. Script byte-identical to `main`. **No tree can move this leg.** | **(b) REPORTED, NOT FIXED** |
| 2 | `clippy` | **101** | **101** | 15 errors. Lint+file multiset base ↔ HEAD: **identical**. 11 primary spans in `io/uring.rs`, a file **absent from this flight's diff entirely**. The other 4+4 are `clippy::unnecessary_cast` on `libc::S_IFMT as u32` &c. in `file_meta_bifs.rs`, which this flight *does* touch — so I diffed the flagged region: base `245-270` vs HEAD `247-272` → **byte-identical**; the +2 shift is this flight's one added `use` plus one statement split. libc 0.2.189 already types those constants `u32` here: dependency/platform drift. | **(b) REPORTED, NOT FIXED** |
| 3 | `clippy-all-features` | **101** | **101** | The same 15, at base and at HEAD, and identical to leg 2's multiset. | **(b) REPORTED, NOT FIXED** |
| 4 | `tests` | **101** | **101** | ONE test: `distribution_runtimes_exist_only_when_configured_and_are_joined_at_shutdown` (`crates/beamr/tests/thread_inventory_distribution.rs:59`), **failing identically at the base** — same message, same numbers, `dist-send=1, net-kernel=0`. Test file **untouched** by this flight. Base 66 result lines / 2126 passed; HEAD 66 / **2127** — the +1 is this flight's own `accessor_proof_tests` control, which passes. | **(b) REPORTED, NOT FIXED** |
| 5 | `tests-all-features` | **101** | **101** | The same one test, same message. Base 66 / 2136; HEAD 66 / **2137**, same +1. **SIGBUS lines: 0** — round 1's TEST-COUNT trip has now failed to reproduce five consecutive full runs. | **(b) REPORTED, NOT FIXED** |

**REASONED, and it is the whole disposition:** a leg that returns the same rc
for the same reason on untouched `main` is not measuring this flight. Four of
the five are beamr source outside the accessor-lifetime scope; the fifth is
beamr's own gate script, and relaxing it is what the script's own REFUSE text
forbids. My algorithm's arm (a) — a red whose cause is the new lifetimes, their
callers, the design artifact, the evidence files or the `.fleet` files — **was
never reached: no such red exists on this tree.**

---

## 2. THIS TREE, GRADED BY THE UNEDITED ENTRY — my own canon, round 4

The canon leg loop transcribed verbatim from `.github/workflows/ci.yml:66-112`
(no `set -e`, subshell around `eval`, redirect never pipe), run whole against
`1abfdad4`. Round 3's artefact set was preserved first
(`../gate-rc-round3-preserved`, 28 files) so nothing already graded was
destroyed to make room.

```
LEG 1 (fmt) rc=0                            LEG 6 (blocking-call-in-native-bif) rc=0
LEG 2 (clippy) rc=101                       LEG 7 (clippy-all-features) rc=101
LEG 3 (wasm32-check) rc=0                   LEG 8 (tests-all-features) rc=101
LEG 4 (wasm-tests) rc=0                     LEG 9 (nostd-ratchet) rc=3
LEG 5 (tests) rc=101                        ### CANON LOOP DONE ###
```

`$ ./.fleet/gate-entry.sh` → `ENTRY_EXIT=1`, and its report is **identical to
gate rounds 2 and 3, leg for leg and count for count** — 2127 passed, 2137
passed, 86 passed, the same five FAILs.

⚠️ **A standing note for whoever runs the entry next.** `scripts/ci-verdict.sh`
only **grades** `gate-rc/`; it does not run the legs, and `gate-rc/` is
gitignored. **Run the canon loop first or the entry grades whatever the
directory already holds** — that is how round 1 came to grade the base.

⏱ **Timing discipline.** Every long run polled under a **stated** deadline;
**never one blocking sleep sized to the expected duration**. Canon at HEAD:
deadline 900 s, actual 3 m 03 s. Base re-measurement: 1800 s, actual 5 m 04 s.
Miri whole-module: 1500 s, actual 1 m 11 s (HEAD) / 1 m 08 s (base). Miri
per-family: 1200 s, actual 1 m 38 s (HEAD) / 1 m 08 s (base). **No deadline
expired, so there is no TIMEOUT to report** — and a TIMEOUT would have been
reported as its own outcome, distinct from a red.

---

## 3. THE SWEEP, RE-RUN AT THIS SEAT — `:357` FOUND AND RETIRED

Command with output (evidence file §4), not an assertion. The base carried
**23** `&'static [` sites across the workspace; HEAD carries **7**, none of them
in the four families.

```
$ git grep -n "&'static \[" -- 'crates/**/*.rs'          # HEAD
  crates/beamr-wasm/src/artifact_loader_tests.rs:37:    Bytes(&'static [u8]),
  crates/beamr/src/interpreter/opcodes/binary/construction.rs:214:    fn bytes(...) -> Option<&'static [u8]>
  crates/beamr/src/interpreter/opcodes/binary/mod.rs:91:pub(super) fn slice_from_words(...) -> &'static [u8]
  crates/beamr/src/jit/runtime_binary_build.rs:194:    fn bytes(...) -> Option<&'static [u8]>
  crates/beamr/src/native/stdlib_stubs/encoding_bifs.rs:63,68,88   &'static [u8; 64] BASE64_ALPHABET

$ git grep -n "'static" -- crates/beamr/src/term/         # the families' own files
  crates/beamr/src/term/json.rs:23:    UnsupportedTerm(&'static str),      <- literal, not heap
  crates/beamr/src/term/json.rs:35:    AllocationFailed(&'static str),     <- literal, not heap

$ git grep -n 'fn parent_bytes' -- crates/
  crates/beamr/src/term/boxed/binary_accessors.rs:207:
    fn parent_bytes<'heap>(parent: Term, heap: HeapBorrow<'heap>) -> Option<&'heap [u8]>

$ git grep -n 'parent_bytes' -- crates/beamr/src/term/boxed/accessors.rs
  (absent, rc=1 — it moved with ProcBin/SubBinary into boxed/binary_accessors.rs)
```

| # | base coordinate | signature at base | disposition |
|---|---|---|---|
| 1 | `boxed/accessors.rs:113` | `BigInt::limbs -> &'static [u64]` | **FOUND AND RETIRED** |
| 2 | `boxed/accessors.rs:281` | `ProcBin::as_bytes -> &'static [u8]` | **FOUND AND RETIRED** |
| 3 | `boxed/accessors.rs:340` | `SubBinary::as_bytes -> &'static [u8]` | **FOUND AND RETIRED** |
| 4 | **`boxed/accessors.rs:357`** | **`parent_bytes -> Option<&'static [u8]>`** | **FOUND AND RETIRED** ← the gate's KNOWN ANSWER |
| 5 | `shared_binary.rs:79` | `bytes_from_raw_word -> &'static [u8]` | **FOUND AND RETIRED** |
| 6 | `binary.rs:70` | `Binary::as_bytes -> &'static [u8]` | FOUND AND RETIRED (in-family) |
| 7 | `binary_ref.rs:30` | `BinaryRef::as_bytes -> &'static [u8]` | FOUND AND RETIRED (in-family fan-in) |
| 8–15 | `compare/mod.rs:337,377` · `matching.rs:602,806` · `jit/runtime_binary_match.rs:212` · `encoding_bifs.rs:203` · `string_bifs.rs:355` · `otp_stubs/tests.rs:407` | eight more `&'static` returns over heap memory | RETIRED (compiler-forced) |

**Zero remaining `'static` returns over heap memory in the four families.** The
three heap-bearing survivors (`slice_from_words`, `BinaryBuilder::bytes` ×2) are
the **binary-BUILDER** family, not the four; all three are byte-identical to
`main` (`binary/mod.rs` is absent from the diff; the other two verified
line-range-identical). §9 item 3.

---

## 4. THE COMPILE-FAIL EVIDENCE, PER ACCESSOR FAMILY — RE-DERIVED BY THE COMPILER

**New at this seat, and it matters.** A passing `compile_fail` doctest is a
*bit*: "this did not compile", not "this did not compile for the stated reason"
— and this tree's own control (`term::accessor_proof_tests`) records that the
`,E0502` annotation is **not enforced** on the pinned 1.97.1. So I handed the
five programs to `rustc` directly, outside the repo, against the gate-built
rlib, and quoted the **diagnostic text**:

```
$ rustc --edition 2024 --crate-type bin -L dependency=target/debug/deps \
    --extern beamr=target/debug/deps/libbeamr-98228346e42209a5.rlib \
    ../r4-compilefail/proof.rs -o /dev/null           ; rc=1

error[E0502]: cannot borrow `process` as mutable because it is also borrowed as immutable
  --> proof.rs:19:38                       <- FAMILY 1  BigInt::limbs
18 |     let limbs = bigint.limbs(process.borrow_terms());
   |                              ------- immutable borrow occurs here
19 |     let _ = beamr::gc::collect_minor(&mut process);
   |                                      ^^^^^^^^^^^^ mutable borrow occurs here
20 |     assert_eq!(limbs, &[7, 9]);
   |     -------------------------- immutable borrow later used here
   … the same error, verbatim, at :34 (ProcBin::as_bytes), :48 (SubBinary::as_bytes,
     which is also the only reader of the private parent_bytes), :60 (Binary::as_bytes)
     and :70 (BinaryRef::as_bytes)
error: aborting due to 5 previous errors
```

**Five errors, five families, one code.** The dangle is a type error, per
accessor family, measured at this seat by the compiler itself.

The matched-pair half — the same programs *without* the collection line — run:

```
$ cargo test --doc -p beamr --features encode          ; rc=0
running 6 tests   … 6 passed   <- positive controls COMPILE AND RUN
running 7 tests   … 7 passed   <- the compile_fail proofs
```

⚠️ **Re-confirmed, and still true: the gate does not execute those doctests.**
`grep -c 'Doc-tests' gate-rc/tests.log` → **0**; all 66 test binaries run and
`cargo test` stops at the pre-existing failure of §1 row 4 **before** the
Doc-tests phase. The only ways to put them back in-gate are `--no-fail-fast` in
`gates.json` — **editing the gate** — or fixing `thread_inventory_distribution.rs`
— **out of scope**. Both refused. The **structural** half of each pair *is*
gate-executed (`gate-rc/tests.log:1829`,
`term::accessor_proof_tests::every_compile_fail_proof_is_its_positive_control_plus_the_collection … ok`).

---

## 5. NEW AT THIS SEAT — THE MIRI DIFFERENTIAL

The venue has miri provisioned: **miri 0.1.0 (8925ea358a 2026-08-20)**, and the
R1 red used it. No earlier round ran miri *after* the fix. I did, on the tree
and then identically on the base.

**Feature selection was measured, not guessed** — the R1 red's
`--no-default-features --features std` works for an *integration* target but not
for the lib unit-test target, which pulls the lib's own `#[cfg(test)]` modules:
`std` → 12 × E0432/E0433 (`crate::io` is behind `threads`); `std,threads` → 128
(io needs `libc` from net/fs); `std,threads,net,fs` → 8 (scheduler needs
jit+embedded). The default set compiles. All four attempts are kept in evidence.

**Per family, `cargo +nightly miri test -p beamr --lib` at HEAD — the reads that
REMAIN, now witness-bounded:**

| miri filter | BASE (`c55ac360`) | HEAD (`1abfdad4`) | verdict |
|---|---|---|---|
| `term::binary::` | 3 passed, **no UB**, no leaks, rc=0 | 3 passed, **no UB**, no leaks, rc=0 | identical |
| `term::binary_ref::` | 4 passed, **no UB**, 4 leaks, rc=1 | 4 passed, **no UB**, 4 leaks, rc=1 | identical |
| `term::shared_binary::` | 6 passed, **no UB**, 6 leaks, rc=1 | 6 passed, **no UB**, 6 leaks, rc=1 | identical |
| `term::sub_binary::` | 4 passed, **no UB**, 2 leaks, rc=1 | 4 passed, **no UB**, 2 leaks, rc=1 | identical |
| `term::boxed::` (incl. `BigInt`) | 13 passed, **no UB**, no leaks, rc=0 | 13 passed, **no UB**, no leaks, rc=0 | identical |
| `term::accessor_proof_tests` | (did not exist) | 1 passed, rc=0 | new, green |

**OBSERVED: zero Undefined Behaviour in any of the four families, at the base
and at HEAD.** The `rc=1`s are miri's *leak* checker, not UB — leaked
`Arc<Vec<u8>>` allocations from `write_proc_bin` in tests whose heap is a plain
`Vec<u64>` with no GC to run `gc::release_*`. **The leak counts and sizes are
identical at base and HEAD** (16/40/8/40; 40/65/8/40/102400/40; 32/40), so this
flight neither introduced nor removed one. Pre-existing test-harness condition.

**One real UB, and it is the base's.** The unfiltered `-- term::` run aborts at
`term::compare::tests::comparing_long_lists_does_not_stack_overflow` with a
Stacked-Borrows read violation in `Cons::word`
(`term/boxed/accessors.rs:72` at HEAD). I ran the **same command on the
untouched base** and got the **same UB, in the same test, in the same function**
(`accessors.rs:71` there — the one-line shift this flight caused by moving
`ProcBin`/`SubBinary` out). `Cons` is byte-identical to `main` and the test is
untouched. **PRE-EXISTING — reported, not chased** (§9.9). It is the
wildcard-provenance shape the R1 red evidence already flagged: beamr's `Term`
is a tagged word, so every heap read is an exposed int-to-pointer cast.

---

## 6. NO LAUNDERING — re-checked at HEAD over the whole tied path

Over the **path**, not just the diff: a diff census cannot see a pre-existing
hole the new signatures now route through.

```
$ git grep -n -F 'transmute'      -- crates/beamr/src/term/ crates/beamr/src/process/   -> none (rc=1)
$ git grep -n -F 'from_raw_parts' -- crates/beamr/src/term/ crates/beamr/src/process/
  process/heap.rs:568     unsafe { std::slice::from_raw_parts(src, len).to_vec() }   <- owns immediately
  term/heap_borrow.rs:79  unsafe { core::slice::from_raw_parts(ptr, len) }           <- the ONE site
$ git grep -nE "fn [a-z_]+<'[a-z]+>\(.*\*(const|mut) .*\) -> &'" -- .../term/ .../process/  -> none (rc=1)
```

**REASONED:** every byte and limb the four families hand out flows through
`HeapBorrow::slice` (`term/heap_borrow.rs:75-80`). Its `'heap` is the witness's
own lifetime, and a witness is constructible **only from a real shared borrow in
argument position** — `HeapBorrow::of_words(words: &'heap [u64])`,
`Heap::borrow_terms(&self)`, `Process::borrow_terms(&self)`,
`ProcessContext::borrow_terms(&self)`, `OwnedTerm::borrow_terms(&self)`,
`ConstantPool::borrow_terms(&self)`. A caller cannot widen it by inference —
which is the property the whole mechanism rests on, and precisely what a
`transmute` or a `fn(*const T) -> &'a [T]` helper would destroy.

Census over the flight's **2037** added lines under `crates/` (fixed-string,
re-run by me): `transmute` **0** · `'static` **0** · `#[allow` **0** ·
`#[expect` **0** · `#[deny` **0** · `#[warn` **0** · `#[ignore` **0** ·
`.unwrap()` **0** · `from_raw_parts` **1** · `unsafe` **30** · `SAFETY` **27**.
The workspace's only three `transmute`s are pre-existing JIT function-pointer
casts, outside the tied path and untouched.

### 6.1 `with_frame` — judged at this seat, not inherited

`HeapBorrow::with_frame` fabricates a witness from a call-frame-local array for
the **16** readers a `std` trait signature leaves no room for (`PartialEq`/`Ord`
for `Term`, `Hash` for `EtsKey`, `ets::copy_term_to_ets`). Whether that is the
laundering the wall forbids is a judgement, so here is mine and its reasoning:

- **It cannot restore a caller-chooseable lifetime.** The bound is
  higher-ranked — `for<'frame> FnOnce(HeapBorrow<'frame>) -> R` — so neither the
  witness nor any slice derived from it can escape `f`. The signature fix cannot
  be undone through it. **That is why I do not read it as laundering.**
- **What it does not enforce** is "do not allocate *inside* the closure": that
  obligation is written, not typed. **This is the honest residual**, and it is
  strictly better than the base, where the same readers called `&'static`
  accessors with no bound, no marker, and nothing to grep.
- It is `unsafe`, it is `pub(crate)` (`term/heap_borrow.rs:58` — **no public API
  exposes it**), and I verified every one of the 16 call sites sits inside an
  `unsafe` block carrying a written SAFETY argument.

Design §4 and §7.1 already name it. I re-measured the count (`git grep -c -F
'HeapBorrow::with_frame('` → 16 across 9 files) rather than accept it.

---

## 7. THE CALLER INVENTORY, RE-COUNTED AFTER IMPLEMENTATION

Design §2 counted **69** library sites in **34** files in one crate, using the
authoritative instrument (a discarded spike's `E0061` list), and said in terms
that test-target and doctest sites are additional and re-counted here.

**Re-count at HEAD.** Instrument stated: a tied accessor now *requires* a
witness, so a call with an empty argument list is a compile error and cannot
exist in a tree that builds. The population is therefore every call with an
argument:

```
$ git grep -nE '\.(as_bytes|limbs)\([^)]' -- 'crates/**/*.rs' | wc -l
204
```

| Crate :: layer | Sites |
|---|---|
| `beamr :: native/` | 97 |
| `beamr :: term/` | 38 |
| `beamr :: interpreter/` | 17 |
| `beamr :: ets/` | 11 |
| **`beamr :: jit/`** | **6** |
| `beamr :: gc/` | 6 |
| `beamr :: etf/` | 5 |
| `beamr :: constant_pool/` | 5 |
| `beamr :: tests/` (integration) | 4 |
| `beamr :: mailbox/` | 4 |
| `beamr :: distribution/` | 3 |
| `beamr :: scheduler/`, `beamr :: io/` | 1 + 1 |
| **`beamr-wasm :: src/`** | **6** |
| **Total** | **204** |

Witness form: `…​.borrow_terms()` **123** · a threaded `heap` parameter **70** ·
`HeapBorrow::of_words(&words)` in unit tests **11**.

**The JIT is in the count and is not optional** — the six, named:
`jit/runtime.rs:570`, `:618`, `jit/runtime_binary_build.rs:91`,
`jit/runtime_binary_match.rs:232`, `:508`, `jit/runtime_map.rs:190`.

---

## 8. THE API-BREAKAGE LIST

Re-extracted by me from the diff (every `pub fn` line added or removed, all
crates), then cross-checked against design §3 and §7.2 — **they agree item for
item, and nothing in the diff is missing from the design's list.**

**BREAKING — a public signature gains the witness (10):**
`term::boxed::BigInt::limbs` · `term::binary::Binary::as_bytes` ·
`term::binary_ref::BinaryRef::as_bytes` · `term::boxed::ProcBin::as_bytes` ·
`term::boxed::SubBinary::as_bytes` · `term::bigint_math::BigIntValue::from_bigint`
· `::from_term` · `term::bigint_convert::integer_term_to_string_radix` ·
`beamr_wasm::convert::term_to_js_value` · `::terms_to_js_array`.

**ADDITIVE — non-breaking:** the new public module `term::heap_borrow`
(`HeapBorrow<'heap>`, `::of_words`), and `borrow_terms` on `Heap`, `Process`,
`ProcessContext`, `ets::copy::OwnedTerm`, `constant_pool::ConstantPool`.

**INTERNAL ONLY:** `SharedBinary::bytes_from_raw_word` gains the witness,
`::len_from_raw_word` is new (both `pub(crate)`); `parent_bytes` is private.

**MODULE MOVE, NOT A PATH CHANGE:** `ProcBin`/`SubBinary` moved to
`term::boxed::binary_accessors` and are re-exported from `term::boxed`, so
`beamr::term::boxed::{ProcBin, SubBinary}` is **identical**. The move keeps
`accessors.rs` under the 500-line wall.

**RENAME ONLY:** `bif_os_putenv`/`bif_os_unsetenv` changed `_context` →
`context`; types identical, not a break.

**What it means for the next version:** all ten are source-breaking for any
downstream caller of the binary or bignum accessors — for a `0.x` crate, a minor
bump of `0.y`. **This reports the fact. The version decision is not this
flight's**, and `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`,
`RELEASE-OBLIGATIONS.json` are all **unchanged** (verified).

---

## 9. MEASURED REDS BEYOND THIS SCOPE — REPORTED, NOT CHASED

1. **The five gate legs of §1.** All pre-existing at the base, all re-measured
   by me.
2. **The doctest-unreachability of §4** — a consequence of red 1.4, not its own
   defect. Clears itself when `thread_inventory_distribution.rs` is fixed.
3. **The binary-BUILDER family — the same defect class, still live.** Confirmed
   at HEAD by my own sweep, all byte-identical to `main`:
   `interpreter/opcodes/binary/mod.rs:91` `slice_from_words(...) -> &'static [u8]`;
   `interpreter/opcodes/binary/construction.rs:214` `bytes(...) -> Option<&'static [u8]>`;
   `jit/runtime_binary_build.rs:194` `bytes(...) -> Option<&'static [u8]>`. Also
   `interpreter/opcodes/binary/mod.rs:95`
   `heap_slice<'a>(ptr: *mut u64, words: usize) -> &'a mut [u64]` — a **fully
   caller-inferred** lifetime from a raw pointer, weaker still.
   **These are the next signatures that will manufacture an unsound caller**,
   and closing them is the highest-value follow-on from this flight. Not in the
   four families, so not chased here.
4. **`process_from_abi -> Option<&'static mut Process>`** (`jit/runtime.rs`,
   `jit/ir_exceptions.rs`) — a `&'static mut` over a raw ABI pointer.
5. **EIGHT undocumented `unsafe` blocks in touched files — PRE-EXISTING.** Found
   at this seat by running a SAFETY-comment checker at HEAD *and at the base*
   over the same 58-file set: the same eight appear at both, one for one, only
   line-shifted, and each block is byte-identical (`jit/runtime_binary_build.rs`
   ×2, `jit/runtime_binary_match.rs` ×4, `native/udp_bifs.rs` ×2). **This flight
   touched none of them**; the wall it is held to — every `unsafe` block *this
   flight touches* carries a re-derived safety comment — is satisfied.
6. **ONE `.expect()` in library code — PRE-EXISTING.**
   `native/stdlib_stubs/json_bifs.rs:487` (`:480` at the base, the same line,
   not in this flight's diff). Found by the same base-vs-HEAD differential.
   This flight added **zero** `.unwrap()` and **zero** library-code `.expect()`.
7. **The `with_frame` residual — 16 sites, named, bounded, `unsafe`,
   `pub(crate)`.** §6.1. The honest residual, not a hidden one.
8. **The `mktemp -t` bug in `gate-nostd-ratchet.sh:182`** — a one-word fix that
   would restore a dead gate on every GNU-coreutils venue. **Not mine to make**,
   and the script's own REFUSE text forbids relaxing it to pass.
9. **A pre-existing miri Stacked-Borrows UB in `Cons::word`** — §5. Reproduced
   at the base by me; `Cons` is untouched by this flight.

None of these is in the accessor-lifetime scope. Under my disposition
algorithm every one is arm **(b): committed as evidence, reported, not fixed.**

---

## 10. HARD WALLS — re-measured at this seat, not inherited

| Wall | Measurement at round 4 |
|---|---|
| No lint suppressions in any spelling | **0** `#[allow`/`#[expect`/`#[deny`/`#[warn` in the added lines; per-file re-check over every touched `.rs`: no hits |
| No ignore attributes on tests | **0** `#[ignore` added, in any spelling |
| Never `.unwrap()`/`.expect()` in library code | **0** added. Instrument: a brace-tracking pass excluding `#[cfg(test)]` bodies and `///` doc comments, run at HEAD **and at the base** over the same 58 files. One survivor at each, the same pre-existing line (§9.6). All **70** added `.expect(` are in a `#[cfg(test)]` module, a `*_tests.rs`/`tests/` file, or a `///` doctest |
| No file over 500 lines of code | **Nothing crossed 500 because of this flight.** All 38 touched files over 500 at HEAD were already over 500 at the base (checked pairwise); `term/compare/mod.rs` went **down**, 713 → 663. New files: `binary_accessors.rs` 225, `accessor_proof_tests.rs` 200, `compare/bigint.rs` 104, `heap_borrow.rs` 81 |
| Every touched `unsafe` carries a re-derived safety comment | Checker run at HEAD and at the base: the same eight undocumented blocks at both, byte-identical, **none touched by this flight** (§9.5). The comments this flight wrote were **read, not counted** — e.g. `shared_binary.rs:82-91` argues about GC *releasing* the `Arc`, not only about object motion, and concludes on `'heap` |
| No new dependencies / version bumps / changelog / release prep | `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, both crate manifests, `RELEASE-OBLIGATIONS.json`: **all unchanged** vs `main` |
| Never edit the gate entry / weaken a check | `git diff --quiet main..HEAD` → **IDENTICAL** for `gates.json`, `scripts/ci-verdict.sh`, `.github/workflows/ci.yml`, `scripts/gate-nostd-ratchet.sh`, `scripts/gate-blocking-call.sh`. `.fleet/gates/*` and `.fleet/gate-entry.sh` untouched at this seat |
| Never one blocking sleep sized to expected duration | Every long run polled under a **stated** deadline (§2). No deadline expired, so **no TIMEOUT to report**; TIMEOUT would have been its own outcome |
| R1 before R2 | `git log --reverse`: design `43eb6ce6` 10:36:50 → red evidence `af9c729a` 10:43:22 → implementation `016bd0a8` 11:24:41 → green evidence `765f81aa` 11:25:38. **Design and red both precede the first moved signature.** |

---

## 11. THE DIVERGENT BRANCH — BY MEASUREMENT, re-derived at this seat

`git fetch --prune origin` (rc=0), then `git for-each-ref refs/remotes/origin`
with `git rev-list --left-right --count origin/main...<ref>`. Full table in the
evidence file §10. Excluding this workflow's own two `origin/fleet/*` refs,
**fourteen** divergent branches:

- **`main` is an ancestor of NO divergent working branch** — every one is
  behind. The audit's *"173 ahead with `main` an ancestor"* **describes nothing
  in this repository today**, exactly as gate binding note 2 anticipated.
  Re-derived here from a fresh fetch at this seat.
- **The topical branch is `origin/fix/0163-borrow-across-alloc`** — the 0.16.3
  call-site-sweep lane for this very defect — **15 ahead / 401 behind** `main`,
  head `9d0d0e0d`.
- The next-largest divergences: `diana/seat-preserve-annabelbox` 4/270,
  `seth/replay-rebuild-3aecb622` 3/296, `diana/beamr-wedge002-3aecb622` 3/280,
  `artemis/backport-0.17.1` 3/204, `juniper/docs-package` 1/526.

⚠️ **Re-merging this fix into that branch, or any other, is the OWNER's work.**
It is not done here.

---

## 12. LANE BOUNDARY

This flight produces a **CANDIDATE result branch**. **Nothing was merged to
beamr `main` at this seat, and nothing was pushed** — the landing is ratified by
the repo owner's seat. `git log origin/fleet/accessor-lifetimes-base..HEAD`
shows the work sitting local to the run tree, as the fan-in contract expects.

Model pin, verified in-band at this seat, not assumed:

```
$ env | grep ANTHROPIC_MODEL
ANTHROPIC_MODEL=claude-opus-5
```

---

## 13. RECEIPT

Path is the wire contract: **`.fleet/reports/accessor.md`**, byte-exact.
Confirmed with `git ls-tree HEAD -- .fleet/reports/` after the commit; the
listing showing the blob at the contracted path is the receipt.

Evidence added at this seat:
- **`docs/evidence/beamr-accessor-gate-round4.txt`** — the base re-measurement
  from a fresh worktree, the round-4 canon and entry, the sweep, the
  rustc-derived E0502 ×5, the no-laundering census, the walls differential, the
  caller re-count, the API-breakage extraction, the branch table, and the miri
  differential.

⚠️ **FOR THE NEXT GATE SEAT.** `gate-rc/` is gitignored: it travels with the
machine, never with the branch, and the entry only **grades** whatever
`gate-rc/` already holds. **Run the canon leg loop before the entry means
anything.** `gate-rc/` on this venue now holds **round-4** artefacts, produced
from `1abfdad4`; rounds 2 and 3 are preserved beside it at
`../gate-rc-round2-preserved` and `../gate-rc-round3-preserved`.

---
---

# APPENDIX — THE PRIOR ROUNDS, verbatim and unedited

What follows is the round-3 fix seat's report exactly as it stood, and nested
inside it, verbatim, the round-2 fix seat's report and the builder seat's.
Nothing below this line was edited at round 4.

Model: `claude-opus-5` — this seat's own identifier, verbatim. Pin verified
in-band, not assumed: `env | grep ANTHROPIC_MODEL` → `ANTHROPIC_MODEL=claude-opus-5`
(OBSERVED). The harness reports the served model as `claude-opus-5[1m]` (the
1M-context variant of the pinned model).

---

# REPORT — beamr accessor lifetimes, FIX SEAT (gate round 3)

Flight `beamracc-lifetimes-flight3`, branch `fleet/accessor-lifetimes-base`,
2026-08-22. Base: beamr `main` `c55ac360`. Tree graded: `f0527dd2`.
Gate round 2: RED (`.fleet/gates/RED-LATEST.md` = `.fleet/gates/red-2.md`,
unedited, untouched).

Every claim below is **OBSERVED** (command output / `file:line`) or
**REASONED**. Round 2's report and the builder's are preserved verbatim below
mine. **I inherited no disposition.** Round 2 had already dispositioned these
five reds; a disposition taken from another seat is not a measurement, so I
took every one again — fresh base worktree, fresh target directory, fresh canon.

## Outcome in one line

**All five gate reds are pre-existing on beamr `main`, re-measured at the base
by me from scratch, and all five stand. Nothing in scope was found red at this
seat.** The gate is still RED on the same five legs; under the workflow's
grammar that arrives as `gates_red/Failed`, and per the measured-red wall the
differential is the criterion — **no leg that was green at the base is red
here, and no leg changed its rc.**

---

## 1. THE BASE, RE-MEASURED FROM SCRATCH AT THIS SEAT

`git worktree add --detach ../base-r3 c55ac360`, `CARGO_TARGET_DIR=../base-r3-target`
(never shared with the fix tree), `rustc 1.97.1` as pinned by
`rust-toolchain.toml`. Transcript: **`docs/evidence/beamr-accessor-gate-round3.txt` §1**.

| # | Leg | rc at BASE | rc at HEAD | What I measured, and how I know it is not this flight | Disposition |
|---|---|---|---|---|---|
| 1 | `nostd-ratchet` | **3** | 3 | Output **byte-identical** at base and HEAD (`diff` → no difference). Cause read at source: `scripts/gate-nostd-ratchet.sh:182` is `mktemp -t nostd-ratchet`; GNU coreutils rejects a template with no `X`s, so `LOG` is empty, line 184's redirect to `""` fails, **cargo never runs**, and the script's own REFUSE arm fires correctly. Script byte-identical to `main`. **No tree can move this leg.** | **(b) REPORTED, NOT FIXED** |
| 2 | `clippy` | **101** | 101 | 15 errors. `diff` of the lint+file multiset base↔HEAD → **IDENTICAL**. 11 primary spans in `io/uring.rs`, **byte-identical to `main`**; 8 `unnecessary_cast` on `libc::S_IFMT as u32` in `file_meta_bifs.rs`. The +2 line shift is the one `use` line this flight added at `:15` — `diff` of HEAD `250-268` vs base `248-266` → **byte-identical region**. Dependency/platform drift. | **(b) REPORTED, NOT FIXED** |
| 3 | `clippy-all-features` | **101** | 101 | The same 15 — and `diff` of the two legs' multisets on this tree → identical to each other as well. | **(b) REPORTED, NOT FIXED** |
| 4 | `tests` | **101** | 101 | ONE test: `distribution_runtimes_exist_only_when_configured_and_are_joined_at_shutdown` (`crates/beamr/tests/thread_inventory_distribution.rs:59`). Run isolated on untouched `main`: **same failure, same numbers — "dist-send=1, net-kernel=0"**. Test file byte-identical at HEAD. 66 result lines, 2127 passed, that one failure. | **(b) REPORTED, NOT FIXED** |
| 5 | `tests-all-features` | **101** | 101 | The same one test. 66 result lines, 2137 passed. **SIGBUS lines: 0** — a fourth consecutive clean full run, so round 1's TEST-COUNT trip has now failed to reproduce four times. | **(b) REPORTED, NOT FIXED** |

**REASONED, and it is the whole disposition:** a leg that returns the same rc
for the same reason on untouched `main` is not measuring this flight. Four of
the five are beamr source outside the accessor-lifetime scope; the fifth is
beamr's own gate script. Fixing either would be work this brief excludes, and
in the `nostd-ratchet` case would be exactly what the script's own REFUSE text
forbids.

---

## 2. THIS TREE, GRADED BY THE UNEDITED ENTRY — my own canon, round 3

The canon leg loop transcribed verbatim from `.github/workflows/ci.yml:66-112`
(no `set -e`, subshell around `eval`, redirect never pipe), run whole against
`f0527dd2`. **The round-2 artefact set was preserved first**
(`../gate-rc-round2-preserved`, 28 files) so nothing already graded was
destroyed to make room.

```
LEG 1 (fmt) rc=0                            LEG 6 (blocking-call-in-native-bif) rc=0
LEG 2 (clippy) rc=101                       LEG 7 (clippy-all-features) rc=101
LEG 3 (wasm32-check) rc=0                   LEG 8 (tests-all-features) rc=101
LEG 4 (wasm-tests) rc=0                     LEG 9 (nostd-ratchet) rc=3
LEG 5 (tests) rc=101                        ### CANON LOOP DONE ###
```

`$ ./.fleet/gate-entry.sh` → `ENTRY_EXIT=1`, and its report is **identical to
gate round 2's, leg for leg and count for count** — 2127 passed, 2137 passed,
86 passed, the same five FAILs. Full transcript: evidence file §2.

⏱ **Timing discipline, as the wall requires.** The canon ran 12:27:23 → 12:31:38
(~4m15s), **polled at 30 s under a stated 900 s deadline**. The deadline did
not expire, so there is **no TIMEOUT to report** — and TIMEOUT would have been
reported as its own outcome, not as a red.

---

## 3. NEW AT THIS SEAT — the compile-fail proofs are not gate-executed

**OBSERVED, and neither round 1 nor round 2 recorded it.**

```
$ grep -c 'Doc-tests' gate-rc/tests.log               -> 0
$ grep -c 'Doc-tests' gate-rc/tests-all-features.log  -> 0
$ grep -c '^     Running' gate-rc/tests.log           -> 66
$ tail -1 gate-rc/tests.log
error: test failed, to rerun pass `-p beamr --test thread_inventory_distribution`
```

All 66 test binaries run; the **pre-existing** failure of §1 row 4 is the last
of them, and `cargo test` then stops **without entering the Doc-tests phase**.
The five ` ```compile_fail ` accessor proofs are rustdoc doctests, so **as this
tree stands the gate never runs them.** A proof nobody executes is not
evidence, so I measured it directly instead of asserting it:

```
$ cargo test --doc -p beamr --features encode      ; rc=0
running 6 tests  ... test result: ok. 6 passed; 0 failed          <- positive controls RUN
running 7 tests  ... test result: ok. 7 passed; 0 failed          <- compile_fail proofs
```

Per family, OBSERVED: `BigInt::limbs`, `Binary::as_bytes`,
`BinaryRef::as_bytes`, `ProcBin::as_bytes`, `SubBinary::as_bytes` — each
positive control **compiles and runs**, each same-program-plus-`collect_minor`
**fails to compile**. (The other two `compile_fail` entries are
`process::Process`'s `!Send`/`!Sync` proofs, which several safety comments lean
on.) **The dangle is a type error, per accessor family, measured at this seat.**

**The structural half of the pair IS still executed in-gate** — it is a lib unit
test, so it runs long before the failing integration binary:

```
gate-rc/tests.log:1829              test term::accessor_proof_tests::
gate-rc/tests-all-features.log:1833   every_compile_fail_proof_is_its_positive_control_plus_the_collection ... ok
```

**Disposition.** The only two ways to put the doctests back inside the gate are
`--no-fail-fast` in `gates.json`'s test legs — **which is editing the gate** —
or fixing `thread_inventory_distribution.rs` — **which is out of scope**. Both
refused. **REPORTED, NOT FIXED**, and flagged for the owner's seat: this is a
live consequence of the pre-existing red, and it will clear itself the moment
that test is fixed.

---

## 4. THE SWEEP, RE-RUN AT THIS SEAT — `:357` FOUND AND RETIRED

```
$ git grep -n "&'static \[" HEAD -- crates/beamr/src/term/
(no matches — rc=1)

$ git grep -n "'static" HEAD -- crates/beamr/src/term/
crates/beamr/src/term/json.rs:23:    UnsupportedTerm(&'static str),      <- literal, not heap
crates/beamr/src/term/json.rs:35:    AllocationFailed(&'static str),     <- literal, not heap

$ git grep -n 'fn parent_bytes' HEAD -- crates/beamr/src/term/
crates/beamr/src/term/boxed/binary_accessors.rs:207:fn parent_bytes<'heap>(parent: Term, heap: HeapBorrow<'heap>) -> Option<&'heap [u8]> {

$ git grep -n 'parent_bytes' HEAD -- crates/beamr/src/term/boxed/accessors.rs
(absent — rc=1; it moved with ProcBin/SubBinary into boxed/binary_accessors.rs)
```

| # | base coordinate | signature at base | disposition |
|---|---|---|---|
| 1 | `boxed/accessors.rs:113` | `BigInt::limbs -> &'static [u64]` | **FOUND AND RETIRED** |
| 2 | `boxed/accessors.rs:281` | `ProcBin::as_bytes -> &'static [u8]` | **FOUND AND RETIRED** |
| 3 | `boxed/accessors.rs:340` | `SubBinary::as_bytes -> &'static [u8]` | **FOUND AND RETIRED** |
| 4 | **`boxed/accessors.rs:357`** | **`parent_bytes -> Option<&'static [u8]>`** | **FOUND AND RETIRED** ← the gate's KNOWN ANSWER |
| 5 | `shared_binary.rs:79` | `bytes_from_raw_word -> &'static [u8]` | **FOUND AND RETIRED** |
| 6 | `binary.rs:70` | `Binary::as_bytes -> &'static [u8]` | FOUND AND RETIRED (in-family) |
| 7 | `binary_ref.rs:30` | `BinaryRef::as_bytes -> &'static [u8]` | FOUND AND RETIRED (in-family fan-in) |
| 8 | `compare/mod.rs:337` | `binary_bytes -> &'static [u8]` | RETIRED (compiler-forced) |
| 9 | `compare/mod.rs:377` | `normalized_limbs -> &'static [u64]` | RETIRED (compiler-forced) |

**Zero remaining `'static` returns over heap memory in the four families.**
Command with output, not an assertion.

---

## 5. NO LAUNDERING — re-checked at HEAD over the whole tied path

Not just over the diff, because a diff census cannot see a pre-existing hole
the new signatures now route through.

```
$ git grep -n -F 'transmute'      -- crates/beamr/src/term/ crates/beamr/src/process/   -> none
$ git grep -n -F 'from_raw_parts' -- crates/beamr/src/term/ crates/beamr/src/process/
  process/heap.rs:568   unsafe { std::slice::from_raw_parts(src, len).to_vec() }   <- owns immediately
  term/heap_borrow.rs:79  unsafe { core::slice::from_raw_parts(ptr, len) }         <- the ONE site
$ git grep -n -E "fn [a-z_]+<'[a-z]+>\(.*\*(const|mut) .*\) -> &'" -- .../term/ .../process/  -> none
```

**REASONED:** every byte and limb the four families hand out flows through
`HeapBorrow::slice` (`term/heap_borrow.rs:75-80`). Its `'heap` is the witness's
own lifetime, and a witness is only constructible from a real shared borrow in
**argument** position (`HeapBorrow::of_words(words: &'heap [u64])`,
`Heap::borrow_terms(&self)`, `Process::borrow_terms(&self)`). A caller cannot
widen it by inference — which is the property the whole mechanism rests on, and
the one a `transmute` or a `fn(*const T) -> &'a [T]` helper would destroy.

Census over the flight's **2037** added lines under `crates/` (fixed-string):
`transmute` **0** · `'static` **0** · `#[allow` **0** · `#[expect` **0** ·
`#[deny` **0** · `#[warn` **0** · `#[ignore` **0** · `.unwrap()` **0** ·
`from_raw_parts` **1** · `unsafe` **30** · `SAFETY` **27**.

The workspace's only three `transmute`s are pre-existing JIT function-pointer
casts (`interpreter/opcodes/core.rs:883`, `jit/compiler/compiler_tests.rs:60`,
`jit/runtime_closure.rs:174`) — outside the tied path, untouched.

---

## 6. HARD WALLS — re-measured at this seat, not inherited

| Wall | Measurement at round 3 |
|---|---|
| No lint suppressions in any spelling | **0** `#[allow` / `#[expect` / `#[deny` / `#[warn` in the 2037 added lines |
| No ignore attributes on tests | **0** `#[ignore` added, in any spelling |
| Never `.unwrap()`/`.expect()` in library code | **0** `.unwrap()` added. All **70** added `.expect(` sites classified mechanically by file and by position relative to `#[cfg(test)]`: every one is in a `#[cfg(test)]` module, a `*_tests.rs`/`tests/` file, or a `///` doctest. **No library-code survivor.** |
| No file over 500 lines of code | **No file crossed 500 because of this flight** — every touched `crates/**/*.rs` over 500 at HEAD was already over 500 at the base (checked pairwise, base LOC vs HEAD LOC). New files: `binary_accessors.rs` 225, `accessor_proof_tests.rs` 200, `compare/bigint.rs` 104, `heap_borrow.rs` 81. **REPORTED:** the tree carries many pre-existing >500 files; that is the base's condition, not this flight's. |
| Every touched `unsafe` carries a re-derived safety comment | Read, not counted: the safety comments in `heap_borrow.rs`, `binary.rs`, `binary_accessors.rs`, `shared_binary.rs` are argued **against the new lifetimes** — e.g. `bytes_from_raw_word`'s reasons about GC *releasing* the `Arc`, not just about object motion, and concludes on `'heap` |
| No new dependencies / version bumps / changelog / release prep | `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md` absent from `git diff --name-only c55ac360..HEAD` |
| Never edit the gate entry / weaken a check | `git diff --quiet c55ac360 HEAD` → **IDENTICAL** for `gates.json`, `scripts/ci-verdict.sh`, `.github/workflows/ci.yml`, `scripts/gate-nostd-ratchet.sh`. `.fleet/gates/*` and `.fleet/gate-entry.sh` untouched at this seat |
| Never one blocking sleep sized to expected duration | The canon was **polled at 30 s under a stated 900 s deadline**; expiry would have been reported as **TIMEOUT**, a distinct outcome. It did not expire (~4m15s) |
| R1 before R2 | `git log --reverse`: design `43eb6ce6` 10:36:50 → red evidence `af9c729a` 10:43:22 → implementation `016bd0a8` 11:25:02 → green evidence `765f81aa` 11:25:38. **Design and red both precede the first moved signature.** |

---

## 7. MEASURED REDS BEYOND THIS SCOPE — REPORTED, NOT CHASED

1. **The five gate legs of §1.** All pre-existing at the base.
2. **The doctest-unreachability of §3** — a consequence of red #4/#5, not its own defect.
3. **The binary-*builder* family — the same defect class, still live.** Confirmed
   at HEAD by my own sweep, all **byte-identical to `main`**:
   `interpreter/opcodes/binary/mod.rs:91` `slice_from_words(...) -> &'static [u8]`,
   `interpreter/opcodes/binary/construction.rs:214` `bytes(...) -> Option<&'static [u8]>`,
   `jit/runtime_binary_build.rs:194` `bytes(...) -> Option<&'static [u8]>`.
   Each manufactures a `&'static [u8]` over a raw heap pointer with its own
   `from_raw_parts`. Also at `interpreter/opcodes/binary/mod.rs:95`,
   `heap_slice<'a>(ptr: *mut u64, words: usize) -> &'a mut [u64]` — a *fully
   caller-inferred* lifetime from a raw pointer, which is weaker still.
   **These are the next signatures that will manufacture an unsound caller**,
   and closing them is the highest-value follow-on from this flight. Not in
   the four families, so not chased here.
4. **`process_from_abi -> Option<&'static mut Process>`** (`jit/runtime.rs`,
   `jit/ir_exceptions.rs`) — a `&'static mut` over a raw ABI pointer.
5. **The `with_frame` residual — 16 sites, named, bounded, `unsafe`.** Where a
   `std` trait signature leaves no room for a witness (`PartialEq`/`Ord` for
   `Term`, `Hash` for `EtsKey`) or a public entry point takes only a `Term`
   (`ets::copy_term_to_ets`), the readers use `HeapBorrow::with_frame`, whose
   guarantee is weaker than the tie: escape is impossible (higher-ranked), but
   allocating *inside* the closure is not forbidden. **Verified at this seat:**
   `git grep -F 'HeapBorrow::with_frame('` → exactly **16** call sites, every
   one inside an `unsafe` block carrying a written SAFETY argument that reasons
   about the NEW lifetimes and cites design §4; and `with_frame` itself is
   `pub(crate)` (`term/heap_borrow.rs:58`), so no public API exposes it.
   **This is the honest residual** — `'static` was neither bounded nor
   greppable nor commented; this is all three. Design §4 and §7.1.
6. **The `mktemp -t` bug in `gate-nostd-ratchet.sh:182`** — a one-word fix that
   would restore a dead gate on every Linux venue. **Not mine to make**, and
   the script's own REFUSE text forbids relaxing it to pass.

---

## 8. THE DIVERGENT BRANCH — re-derived BY MEASUREMENT at this seat

`git fetch --prune origin`, then `git for-each-ref refs/remotes/origin` +
`git rev-list --left-right --count origin/main...<ref>`. Full table in the
evidence file §6. Excluding this workflow's own refs, **fourteen** branches:

- **`main` is an ancestor of NO divergent working branch** — every one is
  behind. The audit's *"173 ahead with `main` an ancestor"* **describes nothing
  in this repository today**, exactly as gate binding note 2 anticipated.
  Re-derived here from a fresh fetch, not repeated from round 2.
- **The topical branch is `origin/fix/0163-borrow-across-alloc`** — the 0.16.3
  call-site-sweep lane for this very defect — **15 ahead / 401 behind** `main`,
  head `9d0d0e0d` (2026-07-29).

⚠️ **Re-merging this fix into that branch, or any other, is the OWNER's work.**
It is not done here, and **nothing was merged to beamr `main` at this seat**.

---

## 9. LANE BOUNDARY

This flight produces a **CANDIDATE result branch**. Nothing was merged to beamr
`main` at this seat, and nothing may be — the landing is ratified by the repo
owner's seat.

---

## 10. RECEIPT

Path is the wire contract: **`.fleet/reports/accessor.md`**, byte-exact.
Confirmed with `git ls-tree HEAD -- .fleet/reports/`.

Evidence added at this seat:
- **`docs/evidence/beamr-accessor-gate-round3.txt`** — the base re-measurement
  from a fresh worktree, the round-3 canon, the doctest finding, the sweep, the
  no-laundering census, the branch table.

⚠️ **FOR THE NEXT GATE SEAT.** `gate-rc/` is gitignored: it travels with the
machine, never with the branch, and the entry only grades whatever `gate-rc/`
already holds. **Run the canon leg loop before the entry means anything** —
that is how round 1 came to grade the base. `gate-rc/` on this venue now holds
**round-3** artefacts, produced from `f0527dd2`; round 2's graded set is
preserved beside it at `../gate-rc-round2-preserved`.

---
---

# APPENDIX A — the round-2 fix seat's report, verbatim and unedited

# REPORT — beamr accessor lifetimes, FIX SEAT (gate round 2)

Flight `beamracc-lifetimes-flight3`, branch `fleet/accessor-lifetimes-base`,
2026-08-22. Base: beamr `main` `c55ac360`. Gate round 1: RED
(`.fleet/gates/RED-LATEST.md`, unedited, untouched).

Every claim is **OBSERVED** (command output / `file:line`) or **REASONED**.
The builder seat's report is preserved verbatim below mine; where I measured
something it got wrong, I say so with the measurement, not with an adjective.

## Outcome in one line

**Four of the gate's five reds are pre-existing on beamr `main` — each
re-measured at the base by me, independently, not inherited from the builder's
claim — and the fifth was a rustc SIGBUS that does not reproduce. None is in
scope, so all five stand. One red WAS in scope and I fixed it: the in-gate
compile-fail proof was pinned to `E0502` by an annotation the gated toolchain
ignores, so it greened on a typo.** The gate is still RED on the same five
legs; under the workflow's grammar that arrives as `gates_red/Failed`, and per
the measured-red wall the differential is the criterion.

---

## 1. THE FIRST FINDING — round 1 did not measure the fix

**OBSERVED.** `.fleet/gates/RED-LATEST.md` is the verbatim output of the
*grader* (`scripts/ci-verdict.sh`). The grader reads `gate-rc/`, which the
canon leg loop populates. `gate-rc/` is **gitignored** (`.gitignore:21`), so it
is not commit evidence — its only provenance is its mtimes.

| Fact | OBSERVED |
|---|---|
| every round-1 leg artefact | mtime **10:03:39 – 10:07:17** |
| HEAD in that window | **`506fa7b3`** — `git reflog`: checked out 09:56:57, next commit 10:13:13 |
| what `506fa7b3` is | beamr `main` `c55ac360` + the `.fleet` documents. **Zero beamr source changes.** |
| the fan-in that brought the fix in | **`0cade123`, 11:40:55** |
| the gate commit | **`1eeb54a3`, 11:40:56** — one second later, i.e. the ~1-second grader only |

The source corroborates the clock independently. The round-1 `clippy` log puts
its `unnecessary_cast` findings at `file_meta_bifs.rs:251-264`. Those are
**`main`'s** coordinates — the fixed tree's are `:253-266` (+2, the one `use`
line this flight added to that file).

**CONCLUSION, OBSERVED: the committed RED is a measurement of the flight's
BASE, not of the fix.** I did not treat that as grounds to dismiss it. It is
the reason I re-measured every leg at the base myself, in a separate worktree
with its own target directory, on the pinned toolchain — below.

I did not edit `.fleet/gates/RED-LATEST.md`, `.fleet/gate-entry.sh`,
`scripts/ci-verdict.sh` or `gates.json`. I ran the canon leg loop (transcribed
verbatim from `.github/workflows/ci.yml:66-112`) against the final tree so the
entry has an input that describes the tree it grades, and preserved the round-1
artefacts. Full transcript: **`docs/evidence/beamr-accessor-gate-round2.txt`**.

---

## 2. DISPOSITION OF EVERY RED — measured at the base, one by one

| # | Leg | rc | Root cause, MEASURED AT THE BASE | Disposition |
|---|---|---|---|---|
| 1 | `nostd-ratchet` | 3 | `gate-nostd-ratchet.sh:182` `mktemp -t nostd-ratchet` — GNU coreutils rejects a template with no `X`s, so cargo never runs and the script's own REFUSE arm fires. **Base and final output are BYTE-IDENTICAL** (`diff` → no difference). Tree-independent. | **(b) REPORTED, NOT FIXED** |
| 2 | `clippy` | 101 | 15 errors. Base worktree at `c55ac360`, same command, same toolchain → **rc=101, identical 15-error set** (`diff` of the lint+file multiset → identical). 7 in `io/uring.rs`, which is **byte-identical to `main`**; 8 `unnecessary_cast` on `libc::S_IFMT as u32` in a region of `file_meta_bifs.rs` byte-identical to `main`. Dependency/platform drift. | **(b) REPORTED, NOT FIXED** |
| 3 | `clippy-all-features` | 101 | The same 15, measured at the base too (`rc=101`, 15 errors, same set). | **(b) REPORTED, NOT FIXED** |
| 4 | `tests` / `tests-all-features` | 101 | ONE test: `distribution_runtimes_exist_only_when_configured_and_are_joined_at_shutdown` (`crates/beamr/tests/thread_inventory_distribution.rs:59`). Run **isolated on untouched `main`, quiet box**: same failure, same numbers — *"dist-send=1, net-kernel=0"*. The test file is **byte-identical** at HEAD. | **(b) REPORTED, NOT FIXED** |
| 5 | `tests-all-features` TEST-COUNT | — | Round 1 tripped the TEST-COUNT wall with **0 result lines**. Cause is in its own log: `error: rustc interrupted by SIGBUS` in `LLVMContextDispose` while codegen'ing the `beamr-cli` test binary. **NOT REPRODUCED**: two full re-runs at this seat (11:45, 12:08) → **66 result lines, 0 SIGBUS**. | **(b) venue event, REPORTED** |

**The grader was right about #5 and I am not softening it**: a leg that ran no
test binary is not a pass. What I add is the cause and the fact that it does
not reproduce — a red that vanishes on re-run is a fact about the venue the
next seat needs.

### The one red that WAS in scope — FIXED

**The in-gate compile-fail proof did not prove what its prose claimed.**
Disposition (a): the cause is the flight's own evidence mechanism.

**OBSERVED, isolated control, no beamr code** (throwaway lib crate, so
rustdoc resolves its own injected `extern crate`): a doc block whose real error
is **E0384**, annotated ` ```compile_fail,E0308 `, **PASSES** on the toolchain
`rust-toolchain.toml` pins (`channel = "1.97.1"`) and **FAILS** on nightly
1.100.0 with `Some expected error codes were not found: ["E0308"]`. The gate
does not run nightly. **On the gated toolchain ` ```compile_fail,E0502 ` means
exactly ` ```compile_fail `.**

**OBSERVED on this tree**: deleting the `HeapBorrow` witness argument from one
of the five proofs — an E0061, an entirely different defect — leaves the
doctest **green**. The builder's report §3 states the proofs are *"pinned to
the diagnostic so a typo cannot green them"*. **That is false as measured**,
and it is the load-bearing sentence of the flight's green evidence.

**The fix — the proof is a matched pair, and the pairing is asserted.** Each of
the five accessors now carries two adjacent doc blocks: a **bare runnable
positive control** (the same program *without* the collection) and the
unchanged `compile_fail` proof. `crates/beamr/src/term/accessor_proof_tests.rs`
(`#[cfg(test)]`, `include_str!`, 200 lines) asserts for all five pairs that the
proof is **its control plus exactly one line, and that the line is the
collection**, and that the population is exactly **five**.

| Instrument | What it establishes | Alone, it proves |
|---|---|---|
| positive control compiles **and runs** | every identifier, argument count and import is real | nothing about the borrow |
| structural control passes | the proof is that program plus one line, the collection | nothing about compilation |
| `compile_fail` proof does not compile | that one line is what breaks it | only "some error" |

Together: **the failure is the borrow bound.** The conclusion no longer rests
on an annotation the gated toolchain ignores.

**The new check FIRES — OBSERVED, not asserted.** On the exact edit rustdoc
waved through (witness argument dropped from the `compile_fail` block only):
rustdoc → *"7 passed"*; the structural control → **FAILED**, naming the line.
The vacuity control also fires: delete the collection and rustdoc reports
*"Test compiled successfully, but it's marked `compile_fail`"*.

The `E0502` annotation is **kept**, not deleted: it is correct, it documents
the diagnostic, and it becomes load-bearing for free when the pin moves.

Transcript: **`docs/evidence/beamr-accessor-compile-fail-control.txt`**.
Design amended: **`docs/design/accessor-lifetimes.md` §7.3**.
Commit: `ca2a54a5`.

**No lint was suppressed to land it.** The one clippy warning my module drew
(`clippy::manual_is_multiple_of`) was **fixed in the code** (`.is_multiple_of(2)`),
not `#[allow]`ed.

---

## 3. R3 — THE SWEEP. COMMAND WITH OUTPUT. `:357` IN THE LIST.

Full transcript: **`docs/evidence/beamr-accessor-sweep-r3.txt`**. OBSERVED:

```
$ git grep -n "&'static \[" HEAD -- crates/beamr/src/term/
(no matches — rc=1)

$ git grep -n "'static" HEAD -- crates/beamr/src/term/
HEAD:crates/beamr/src/term/json.rs:23:    UnsupportedTerm(&'static str),
HEAD:crates/beamr/src/term/json.rs:35:    AllocationFailed(&'static str),
```

Two survivors, both `&'static str` **error-message literals** — genuinely
`'static`, not over heap memory. **Zero `'static` returns over heap memory
remain in the four families.**

**THE KNOWN ANSWER, tracked to its new coordinate — OBSERVED:**

```
$ git show c55ac360:crates/beamr/src/term/boxed/accessors.rs | sed -n '357p'
fn parent_bytes(parent: Term) -> Option<&'static [u8]> {

$ git grep -n 'parent_bytes' HEAD -- crates/beamr/src/term/boxed/accessors.rs
(absent from accessors.rs — rc=1)

$ git grep -n 'fn parent_bytes' HEAD -- crates/beamr/src/term/
HEAD:crates/beamr/src/term/boxed/binary_accessors.rs:207:fn parent_bytes<'heap>(parent: Term, heap: HeapBorrow<'heap>) -> Option<&'heap [u8]> {
```

| # | Base coordinate | Signature at base | Disposition |
|---|---|---|---|
| 1 | `boxed/accessors.rs:113` | `BigInt::limbs -> &'static [u64]` | **FOUND AND RETIRED** |
| 2 | `boxed/accessors.rs:281` | `ProcBin::as_bytes -> &'static [u8]` | **FOUND AND RETIRED** |
| 3 | `boxed/accessors.rs:340` | `SubBinary::as_bytes -> &'static [u8]` | **FOUND AND RETIRED** |
| 4 | **`boxed/accessors.rs:357`** | **`parent_bytes -> Option<&'static [u8]>`** | **FOUND AND RETIRED — the gate's KNOWN ANSWER** |
| 5 | `shared_binary.rs:79` | `bytes_from_raw_word -> &'static [u8]` | **FOUND AND RETIRED** |
| 6 | `binary.rs:70` | `Binary::as_bytes -> &'static [u8]` | **FOUND AND RETIRED** (in-family) |
| 7 | `binary_ref.rs:30` | `BinaryRef::as_bytes -> &'static [u8]` | **FOUND AND RETIRED** (in-family fan-in) |
| 8 | `compare/mod.rs:337` | `binary_bytes -> &'static [u8]` | **RETIRED** (compiler-forced) |
| 9 | `compare/mod.rs:377` | `normalized_limbs -> &'static [u64]` | **RETIRED** (compiler-forced) |
| 10-11 | `json.rs:22`,`:34` | `&'static str` literals | benign, not over heap memory |

**Workspace census, OBSERVED:** `&'static [` across `crates/` went **22 → 7**.
The seven survivors: three genuine `'static` constants in `encoding_bifs.rs`
(the BASE64 alphabet — **must not be "fixed"**), one `&'static [u8]` test
fixture over a `const` in `beamr-wasm`, and the three Class-D binary-*builder*
sites of §6 below.

**No laundering — census over the 2037 added lines of the whole flight,
OBSERVED:** `transmute` **0**, `'static` **0**, `#[allow` **0**, `#[ignore`
**0**, `.unwrap()` **0**, `from_raw_parts` **1**, `unsafe` 30, `SAFETY` 27. The
single `from_raw_parts` is `term/heap_borrow.rs:79`, inside
`HeapBorrow::slice`, where `'heap` comes from `self` — a witness constrained by
a real shared borrow at construction, so caller inference cannot widen it. It
is the only raw-pointer-to-slice route in the tied path
(`git grep -n 'from_raw_parts' HEAD -- crates/beamr/src/term/` returns exactly
that one line).

**Every `unsafe` block in the four families' files carries a `SAFETY` comment**
— OBSERVED, checked mechanically over `heap_borrow.rs`, `boxed/accessors.rs`,
`boxed/binary_accessors.rs`, `shared_binary.rs`, `binary.rs`, `binary_ref.rs`:
zero blocks without one. The arguments are re-derived against the new
lifetimes, not carried over: `shared_binary.rs:81-95` reasons explicitly from
the `Arc` **release** at `gc::release_*`, i.e. from the ProcBin's liveness,
which is the heap's, which is what the witness borrows.

---

## 4. THE COMPILE-FAIL EVIDENCE, PER ACCESSOR FAMILY

`docs/evidence/beamr-accessor-lifetimes-green.txt` carries the verbatim
transcripts: **E0061 ×4** (the unchanged red program can no longer be spelled)
and **E0502 ×5**, one per family, at the `gc::collect_minor` call. In-gate, on
the final tree, OBSERVED:

```
running 6 tests   ← the five NEW positive controls + one pre-existing example
test ...BigInt::limbs (line 125) ... ok
test ...Binary::as_bytes (line 82) ... ok
test ...BinaryRef::as_bytes (line 43) ... ok
test ...ProcBin::as_bytes (line 49) ... ok
test ...SubBinary::as_bytes (line 152) ... ok

running 7 tests   ← the five compile_fail proofs + two pre-existing Process ones
test ...BigInt::limbs (line 143) - compile fail ... ok
test ...Binary::as_bytes (line 100) - compile fail ... ok
test ...BinaryRef::as_bytes (line 62) - compile fail ... ok
test ...ProcBin::as_bytes (line 69) - compile fail ... ok
test ...SubBinary::as_bytes (line 174) - compile fail ... ok
```

⚠️ **HONEST NOTE, unchanged from the builder's and re-verified by me:** the
gate's `tests` leg aborts at the first failing test binary
(`thread_inventory_distribution`), so the Doc-tests phase is **not reached**.
OBSERVED — `grep -c 'Doc-tests' gate-rc/tests.log` → **0**, and the same is
true at the base. No differential change; the proofs are in the gated tree and
pass when run. **The structural control is reached**, because it is an ordinary
`--lib` unit test — it is the `+1` in `tests` 2126 → **2127** and
`tests-all-features` 2136 → **2137**.

---

## 5. CORRECTIONS TO THE BUILDER SEAT'S RECORD — measured, not asserted

The builder's report below is substantially accurate and I verified its core
claims independently. Four statements do not survive measurement:

1. **§3: *"pinned to the diagnostic so a typo cannot green them."*** **FALSE on
   the gated toolchain** — §2 above, with the isolated control. This is the one
   I fixed.
2. **§3 item 1: *"E0061 ×5."*** **It is ×4** — OBSERVED, the evidence file's own
   transcript ends `due to 4 previous errors` and lists four. The red program
   declared four `&'static` bindings; `BinaryRef` was not among them. (The
   E0502 count of **×5** is correct.)
3. **§8: the `file_meta_bifs` lints *"read `:251-:264` at baseline and
   `:253-:266` at the end."*** The builder's own numbers are right for the
   source; but the **round-1 gate log reads `:251-264`**, i.e. `main`'s
   coordinates — which is the fingerprint that exposed §1. The builder's §8
   table reports rcs for a "FINAL" gate run whose artefacts are not the ones the
   gate graded.
4. **§9, the 500-line wall: *"three of the four files … did grow."*** The wall
   holds, but the count is understated by an order of magnitude. **OBSERVED: 33
   files that were already over 500 lines grew.** What matters and is **TRUE**:
   **no file crossed 500** — checked mechanically over every `crates/**/*.rs`,
   zero files went from ≤500 at base to >500 at HEAD. The three new Rust files
   are 177, 104 and 81 lines; mine is 200.

---

## 6. MEASURED REDS BEYOND THIS SCOPE — REPORTED, NOT CHASED

1. **Class D — the binary-*builder* family.** Same defect class, different
   source, not reachable through the four accessor families:
   `interpreter/opcodes/binary/mod.rs:91` `slice_from_words` (**byte-identical
   to `main`**), `interpreter/opcodes/binary/construction.rs:214`
   `BinaryBuilder::bytes`, `jit/runtime_binary_build.rs:194` `bytes`. Each
   manufactures a `&'static [u8]` with its own `from_raw_parts` over a raw heap
   pointer. **This is the next signature that will manufacture an unsound
   caller**, and it is the single highest-value follow-on from this flight.
2. **`process_from_abi -> Option<&'static mut Process>`** (`jit/runtime.rs`,
   `jit/ir_exceptions.rs`) — a `&'static mut` over a raw ABI pointer.
3. **`jit_bs_get_integer` / `jit_bs_get_utf*` receive no process pointer**
   (`jit/runtime_binary_match.rs`), so they use `HeapBorrow::with_frame`, whose
   guarantee is weaker than the tie (escape is impossible; allocating *inside*
   the closure is not forbidden) and strictly stronger than `'static`. **16
   `with_frame` sites total**, OBSERVED, each with a written safety argument,
   each `pub(crate)`, none public. Closing the JIT two needs an ABI change in
   the compiler's call emission. **This is the residual and it is bounded,
   listed and auditable** — `'static` was none of those.
4. **The five gate reds of §2.**
5. **The `mktemp -t` bug in `gate-nostd-ratchet.sh:182`** is a one-word fix that
   would restore a dead gate on every Linux venue. **Not mine to make.**

---

## 7. THE DIVERGENT BRANCH — BY MEASUREMENT, RE-DERIVED AT THIS SEAT

OBSERVED — `git for-each-ref` + `git rev-list --left-right --count main...<ref>`
+ `git merge-base --is-ancestor`, excluding this workflow's own refs
(`refs/*/fleet/*`, `origin/HEAD`, `leg/accessor`):

```
REF                                        BEHIND  AHEAD  main-is-ancestor
origin/artemis/audit-amendment-4              197      0  no
origin/artemis/backport-0.17.1                204      3  no
origin/artemis/beamr-85-and-26                 12      0  no
origin/artemis/empty-bundle-class             180      0  no
origin/artemis/jit-operator-switch             23      2  no
origin/artemis/release-0.18.2                  37      0  no
origin/artemis/rf-006                         145      0  no
origin/diana/beamr-wedge002-3aecb622          280      3  no
origin/diana/seat-preserve-annabelbox         270      4  no
origin/feat/jit-cache-enumeration              18      1  no
origin/fix/0163-borrow-across-alloc           401     15  no
origin/fix/aion85-nested-run-suspend           16      0  no
origin/juniper/docs-package                   526      1  no
origin/seth/replay-rebuild-3aecb622           296      3  no
```

- **`main` is an ancestor of NO divergent working branch.** The audit's shape —
  *"173 ahead with `main` an ancestor"* — **describes nothing in this
  repository**, exactly as gate binding note 2 anticipated. Independently
  re-derived here; not repeated from the builder.
- **The topical branch is `origin/fix/0163-borrow-across-alloc`** — the prior
  0.16.3 call-site-sweep lane for this very defect: **401 behind / 15 ahead**,
  merge-base `67f89c41`. OBSERVED —
  `git diff --stat main...origin/fix/0163-borrow-across-alloc -- crates/beamr/src/term/`
  is **empty**: it carries **no** accessor changes to re-merge. Its 15 commits
  touch 38 files, mostly `docs/design/beamr/briefs/evidence/…`, `CHANGELOG.md`,
  `Cargo.lock`, `RELEASE_CHECKLIST.md`.

⚠️ **Re-merging this fix into any of these is the OWNER's work. It is not done
here, and nothing was merged to beamr `main` at this seat.**

---

## 8. HARD WALLS — measured at this seat

| Wall | Measurement |
|---|---|
| No lint suppressions in any spelling | **0** `#[allow` in the flight's 2037 added lines. The one clippy warning my own module drew was **fixed in the code**. |
| No ignore attributes on tests | **0** `#[ignore` added; my control is an ordinary `#[test]` with no env gate |
| Never `.unwrap()`/`.expect()` in library code | **0** `.unwrap(` added. Every added `.expect(` is in a `#[cfg(test)]` module, a `*_tests.rs`/`tests/` file, or a `///` doctest — checked mechanically, no non-doc, non-test survivor |
| No file over 500 lines of code | **No file crossed 500** — mechanically checked over every `crates/**/*.rs`. My new file is 200. 33 files already over 500 at base grew; **REPORTED**, and it is a pre-existing condition of the tree, not one this flight created |
| Every touched `unsafe` carries a re-derived safety comment | zero un-commented `unsafe` blocks in the six files of the tied path |
| No new dependencies / version bumps / changelog / release prep | `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md` untouched — not in `git diff --name-only c55ac360..HEAD`. **`trybuild` was NOT added** to fix the pin; the control is 200 lines of `#[cfg(test)]` in-tree |
| Never edit the gate entry / weaken a check | `.fleet/gates/*`, `.fleet/gate-entry.sh`, `scripts/ci-verdict.sh`, `gates.json`, `scripts/gate-nostd-ratchet.sh`, `scripts/gate-blocking-call.sh` — **all untouched**. The only check that moved got **stronger** |
| Never one blocking sleep sized to expected duration | every long run polled at a stated interval (45 s / 60 s) under a stated deadline (540 s), with TIMEOUT declared as a distinct outcome. No expiry occurred |

---

## 9. LANE BOUNDARY

This flight produces a **CANDIDATE result branch**. **Nothing was merged to
beamr `main` at this seat, and nothing may be** — the landing is ratified by
the repo owner's seat.

---

## 10. RECEIPT

Path is the wire contract: `.fleet/reports/accessor.md`, byte-exact. Confirm
with `git ls-tree HEAD -- .fleet/reports/`.

Evidence added at this seat:
- `docs/evidence/beamr-accessor-gate-round2.txt` — the gate differential
- `docs/evidence/beamr-accessor-compile-fail-control.txt` — the pin control
- `docs/evidence/beamr-accessor-sweep-r3.txt` — the sweep, with `:357`

**Commit-to-canon alignment, OBSERVED.** The canon transcript of §1's evidence
file was taken at `ca2a54a5`; `git diff --name-only ca2a54a5..158fd8ba` touches
only `.fleet/reports/accessor.md` and two `docs/evidence/` files — no
`crates/`, no `Cargo.*`, no `scripts/`, no `gates.json`. The canon was
nonetheless **re-run whole at `158fd8ba`** and is identical leg for leg and
count for count (evidence §6), with a third consecutive SIGBUS-free
`tests-all-features`.

⚠️ **FOR THE NEXT GATE SEAT.** `gate-rc/` is gitignored: it travels with the
machine, never with the branch. **The canon leg loop MUST be run before the
entry means anything** — the entry only grades whatever `gate-rc/` already
holds, and it cannot tell a fresh artefact from a 93-minute-old one. That is
exactly how round 1 came to grade the base. `gate-rc/` on this venue now holds
artefacts from **this** tree.

---
---

# APPENDIX — the builder seat's report, verbatim and unedited

Preserved whole. Read §5 above first: four of its statements do not survive
measurement, and the corrections are there, not here.

---

Model: `claude-opus-5` — this seat's own identifier, verbatim. Pin verified
in-band, not assumed: `env | grep ANTHROPIC_MODEL` → `ANTHROPIC_MODEL=claude-opus-5`
(OBSERVED). The harness reports the served model as `claude-opus-5[1m]` (the
1M-context variant of the pinned model).

---

# REPORT — beamr accessor lifetimes, builder seat

Flight `beamracc-lifetimes-flight3`, branch `leg/accessor`, 2026-08-22.
Base: `fleet/accessor-lifetimes-base` = beamr `main` `c55ac360` + the `.fleet`
documents (`6b1f85cc`).

Every claim below is **OBSERVED** (command output / `file:line`) or **REASONED**.

## Outcome in one line

**The four accessor families are retired at the signatures, the compiler
enforces the bound (E0502 per family), and the gate is red on exactly the same
five legs it was red on before I started — same root causes, same counts, same
single failing test.** Under the workflow's grammar that arrives as
`gates_red/Failed`; per the brief's measured-red wall and the plan's V2
disposition, the differential is the success criterion and it is clean.

---

## 1. Commit ordering — verified in the log

OBSERVED — `git log --oneline 6b1f85cc..HEAD`:

```
2c1dfc2f fix: adapt beamr-wasm's wasm32 test targets to the tied accessors
765f81aa evidence: the dangle is now a type error (R3 green) + design amended by measurement
016bd0a8 fix: tie binary/bignum accessor slices to a heap borrow (R2)
af9c729a evidence: the dangle, demonstrated before the fix (R1 note)
43eb6ce6 design: tie binary/bignum accessor slices to a heap borrow (R1)
```

- **R1 design artifact (`43eb6ce6`) precedes any signature change** (`016bd0a8`).
- **The red evidence (`af9c729a`) precedes the fix commits.**
- **The compile-fail green evidence (`765f81aa`) follows the fix.**

OBSERVED — `git show --name-only 016bd0a8 | grep docs/evidence` returns nothing:
the green evidence is not in the fix commit.

88 files changed, 2935 insertions, 798 deletions.

---

## 2. R1 — the design artifact

`docs/design/accessor-lifetimes.md` (413 lines at R1, plus §7 amended after
implementation). It states (a) the chosen mechanism and every rejected
alternative with its reason, (b) the caller inventory and the instrument that
produced it, (c) the API-breakage assessment, (d) the unrepresentability
argument.

**The mechanism: `HeapBorrow<'heap>`** (`crates/beamr/src/term/heap_borrow.rs`)
— a `Copy`, zero-sized witness that carries only a borrow region. It cannot be
constructed without a shared reference:

| Producer | Signature |
|---|---|
| `Heap::borrow_terms` | `pub fn borrow_terms(&self) -> HeapBorrow<'_>` (`process/heap.rs:300`) |
| `Process::borrow_terms` | `pub fn borrow_terms(&self) -> HeapBorrow<'_>` (`process/mod.rs:454`) |
| `ProcessContext::borrow_terms` | `pub fn borrow_terms(&self) -> HeapBorrow<'_>` (`native/context/mod.rs:776`) |
| `OwnedTerm::borrow_terms` | ETS-owned storage (`ets/copy.rs:65`) |
| `ConstantPool::borrow_terms` | literal pool storage (`constant_pool/mod.rs:56`) |
| `HeapBorrow::of_words(&'heap [u64])` | the primitive the above build on |

**Alternatives rejected, on the record** (design §5): T5 (`Term<'heap>`) rejected
on **representability** — OBSERVED `process/mod.rs:165` `heap: Heap` sits in the
same struct as `:183` `x_regs: [Term; X_REG_COUNT]` and `:188`
`native_roots: Vec<Term>`, so `Term<'heap>` would make `Process`
self-referential, which safe Rust cannot express. T1/T2 with `&Heap`/`&Process`
in `term/` rejected on layering (OBSERVED `process/heap.rs:11` imports
`crate::term::boxed`, so `process` depends on `term`) and on reach (the term
layer's own tests store terms in stack `[u64; N]` arrays, which are term storage
without being a `Heap`). T3 (witness at construction) rejected on cost: 279
construction sites vs 69 byte-reading sites, no extra strength. Debug-asserts and
generation counters rejected by ruling — **none is added**. Copy-out rejected as
a cost-model change. `&self`-elision rejected as insufficient (it ties the slice
to a `Copy` raw pointer on the caller's stack, not to the heap).

---

## 3. R1 note — the red, and the green

**Instrument: miri**, OBSERVED `miri 0.1.0 (8925ea358a 2026-08-20)` after
`export PATH="$HOME/.cargo/bin:$PATH"`, with the ordinary test harness as a
read-back assertion. Committed verbatim: **`docs/evidence/beamr-accessor-dangle-red.txt`**
(558 lines, 4 `error: Undefined Behavior` blocks).

| Family | Verdict, before the fix |
|---|---|
| `ProcBin::as_bytes` | miri: **use after free** — *"alloc44661 has been freed, so this pointer is dangling"*; the `Arc` is released at `gc/mod.rs:391` under the live `&'static` slice |
| `Binary::as_bytes` | miri: Stacked Borrows — the tag created at `as_bytes()` is *"later invalidated by a Unique retag"* at `process/heap.rs:192`; native run reads `b"hello"` back as `[0,0,0,0,0]` |
| `BigInt::limbs` | miri: same shape; native run reads `[0x1122334455667788, 0x99]` back as `[0, 0]` |
| `SubBinary::as_bytes` | native: `b"456789"` → `[0,0,0,0,0,0]`. miri stops one step **earlier**, inside `SubBinary::new` → `parent_bytes` (`accessors.rs:358`) → `Binary::new` — the parent term's exposed tag is already invalidated by the second heap allocation. Same defect class, diagnosed before the slice escapes. **Reported, not chased.** |

The red program is **not** committed to the tree: it must fail the `tests` leg to
be a red, and this flight adds no red. It is carried verbatim inside the
evidence file.

**Green — the same program, after the fix.** `docs/evidence/beamr-accessor-lifetimes-green.txt`:

1. **Unchanged, it no longer compiles** — E0061 ×5. The four `&'static` bindings
   it declares cannot be spelled, because the accessors no longer return a
   lifetime the caller may choose.
2. **Witness supplied, it still does not compile** — **E0502 ×5, one per accessor
   family**: *"cannot borrow `process` as mutable because it is also borrowed as
   immutable"* at the `gc::collect_minor` call. That is the whole point.
3. **In-gate**: the same five proofs are ` ```compile_fail,E0502 ` rustdoc
   doctests on the five public accessors, pinned to the diagnostic so a typo
   cannot green them. OBSERVED `cargo test -p beamr --features encode --doc` →
   *"7 passed"* (five new + the two pre-existing `Process`-is-not-`Send` ones at
   `process/mod.rs:147`,`:156`). **No new dependency; `trybuild` is not used.**

⚠️ HONEST NOTE ON THE DOCTESTS IN-GATE: OBSERVED — the gate's `tests` leg aborts
at the first failing test binary (`thread_inventory_distribution`), so the
Doc-tests phase is not reached. **That is true at baseline too** — OBSERVED
`grep -c 'Doc-tests' /tmp/gate-rc-baseline/tests.log` → 0. The doctests are in
the gated tree and pass when run; the gate does not currently reach them because
of a pre-existing red. No differential change.

---

## 4. R2 — the implementation

### The signatures, before and after

| Coordinate (before) | Before | After |
|---|---|---|
| `term/boxed/accessors.rs:113` | `pub fn limbs(self) -> &'static [u64]` | `pub fn limbs<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u64]` — `accessors.rs:134` |
| `term/boxed/accessors.rs:281` | `ProcBin::as_bytes(self) -> &'static [u8]` | `pub fn as_bytes<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u8]` — `boxed/binary_accessors.rs:60` |
| `term/boxed/accessors.rs:340` | `SubBinary::as_bytes(self) -> &'static [u8]` | same shape — `boxed/binary_accessors.rs:142` |
| **`term/boxed/accessors.rs:357`** | **`fn parent_bytes(parent: Term) -> Option<&'static [u8]>`** | **`fn parent_bytes<'heap>(parent: Term, heap: HeapBorrow<'heap>) -> Option<&'heap [u8]>` — `boxed/binary_accessors.rs:159`** |
| `term/shared_binary.rs:79` | `bytes_from_raw_word(raw: u64) -> &'static [u8]` | `bytes_from_raw_word<'heap>(raw: u64, heap: HeapBorrow<'heap>) -> &'heap [u8]` — `shared_binary.rs:80` |
| `term/binary.rs:70` | `Binary::as_bytes(self) -> &'static [u8]` | `pub fn as_bytes<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u8]` — `binary.rs:91` |
| `term/binary_ref.rs:30` | `BinaryRef::as_bytes(&self) -> &'static [u8]` | `pub fn as_bytes<'heap>(&self, heap: HeapBorrow<'heap>) -> &'heap [u8]` — `binary_ref.rs:53` |

`ProcBin::len` and `SubBinary::new` no longer route through
`as_bytes`/`parent_bytes` (they read a length instead), so **constructing** an
accessor stays witness-free; only **reading bytes** needs a live borrow.

### The no-laundering census — command with output

OBSERVED — over the added lines of the whole diff
(`git diff --cached --unified=0 | grep '^+' | grep -v '^+++'`, 1599 lines):

```
transmute            0
from_raw_parts       1     <- HeapBorrow::slice, the single witness-bound primitive
'static              0
#[allow              0
#[ignore             0
unwrap(              0
unsafe              30
```

The one `from_raw_parts` is `crates/beamr/src/term/heap_borrow.rs:75`, inside

```rust
pub(crate) const unsafe fn slice<T>(self, ptr: *const T, len: usize) -> &'heap [T]
```

`'heap` comes from `self`, which was constrained by a real shared borrow at
construction, so **caller inference cannot widen the result**. There is no other
route from a raw pointer to a slice in the tied path.

OBSERVED — the three `transmute` occurrences anywhere in `crates/`
(`interpreter/opcodes/core.rs:883`, `jit/runtime_closure.rs:174`,
`jit/compiler/compiler_tests.rs:60`) are pre-existing JIT **function-pointer**
casts, untouched by this flight.

**The unconstrained-lifetime wall**: every retired signature names
`heap: HeapBorrow<'heap>` in **argument** position and returns `&'heap`. No
lifetime appears only in a return type or only in a `PhantomData` field.

### unsafe and its safety comments

OBSERVED — 30 `unsafe` and 27 `SAFETY` lines added. Every touched `unsafe` block
carries a safety comment **re-derived against the new lifetimes** — the argument
is now "`heap` witnesses a live shared borrow of the storage, and every path that
could move/free/release it needs `&mut`", not "the borrow points into
caller-owned storage that must outlive use". Examples: `accessors.rs:129-135`
(BigInt limbs), `binary.rs:92-100` (inline bytes), `shared_binary.rs:81-95`
(the `Arc` release argument, naming `gc::release_*` explicitly).

### The 16 witness-less readers — named, not hidden

OBSERVED — `rg -n 'HeapBorrow::with_frame' crates/` → 16 sites in 8 files
(design §7.1 tabulates every one). These are readers that **structurally cannot
be handed a witness**:

- `impl PartialEq for Term` (`term/mod.rs:65`), `impl Ord for Term` (`:79`),
  `impl Hash for EtsKey` (`hash.rs:43`) — `std` trait signatures;
- `format_term`, `term_to_value`, `encode_term` — `Term`-and-`AtomTable` readers
  with no heap handle in any caller (including `beamr-wasm`);
- `ets::copy_term_to_ets` — the *source* heap is not nameable at that boundary;
- **`jit_bs_get_integer` and `get_utf` (`jit/runtime_binary_match.rs:65`,`:330`)
  — the JIT ABI entries receive no process pointer at all.** `jit_bs_get_binary`,
  which *does* receive one, uses a real witness.

`HeapBorrow::with_frame` is `unsafe`, `pub(crate)`, and higher-ranked
(`impl for<'frame> FnOnce(HeapBorrow<'frame>) -> R`), so **no borrow can escape
the call**. REASONED: the guarantee there is weaker than the tie (it does not
forbid allocating *inside* the closure) and strictly stronger than `'static`
(which forbade nothing). Each site carries a written safety argument. **This is
the residual, and it is bounded, listed and auditable.**

---

## 5. R3 — the sweep. COMMAND WITH OUTPUT.

```
$ rg -n "'static" crates/beamr/src/term/
crates/beamr/src/term/json.rs:23:    UnsupportedTerm(&'static str),
crates/beamr/src/term/json.rs:35:    AllocationFailed(&'static str),
```

**Two lines survive, and both are accounted for**: `json.rs:23` and `:35` are
`&'static str` **error-message literals** — genuinely `'static`, not over heap
memory, out of the defect class. (They were `:22`/`:34` before; the file gained
one `use` line.) **Zero `'static` returns over heap memory remain in
`crates/beamr/src/term/`.**

The same command at the base, for the differential — OBSERVED
`git grep -n "'static" 6b1f85cc -- crates/beamr/src/term/`:

| # | Coordinate at base | Signature | Disposition |
|---|---|---|---|
| 1 | `boxed/accessors.rs:113` | `BigInt::limbs -> &'static [u64]` | **FOUND AND RETIRED** |
| 2 | `boxed/accessors.rs:281` | `ProcBin::as_bytes -> &'static [u8]` | **FOUND AND RETIRED** |
| 3 | `boxed/accessors.rs:340` | `SubBinary::as_bytes -> &'static [u8]` | **FOUND AND RETIRED** |
| 4 | **`boxed/accessors.rs:357`** | **`fn parent_bytes(parent: Term) -> Option<&'static [u8]>`** | **FOUND AND RETIRED — the gate's KNOWN ANSWER** |
| 5 | `shared_binary.rs:79` | `bytes_from_raw_word -> &'static [u8]` | **FOUND AND RETIRED** |
| 6 | `binary.rs:70` | `Binary::as_bytes -> &'static [u8]` | **FOUND AND RETIRED** (in-family, Class B) |
| 7 | `binary_ref.rs:30` | `BinaryRef::as_bytes -> &'static [u8]` | **FOUND AND RETIRED** (in-family fan-in, Class B) |
| 8 | `compare/mod.rs:337` | `binary_bytes -> &'static [u8]` | **RETIRED** (compiler-forced, Class C) |
| 9 | `compare/mod.rs:377` | `normalized_limbs -> &'static [u64]` | **RETIRED** (Class C, now `compare/bigint.rs:59`) |
| 10 | `json.rs:22` | `&'static str` literal | benign, explained above |
| 11 | `json.rs:34` | `&'static str` literal | benign, explained above |

`:357` specifically, OBSERVED:

```
$ git show 6b1f85cc:crates/beamr/src/term/boxed/accessors.rs | sed -n '357p'
fn parent_bytes(parent: Term) -> Option<&'static [u8]> {
$ grep -n 'parent_bytes' crates/beamr/src/term/boxed/accessors.rs
(absent from accessors.rs)
$ grep -n 'fn parent_bytes' crates/beamr/src/term/boxed/binary_accessors.rs
159:fn parent_bytes<'heap>(parent: Term, heap: HeapBorrow<'heap>) -> Option<&'heap [u8]> {
```

### Workspace census — 60 → 44, and every survivor explained

OBSERVED — `rg -n "fn .*->.*&'static" crates/ --glob '*.rs' | wc -l` → **44**
(60 at planning time). Excluding `&'static str` label functions and test-file
`Mutex`/`HashMap` statics, the survivors are:

| Coordinate | Class | Disposition |
|---|---|---|
| `interpreter/opcodes/binary/mod.rs:91` `slice_from_words` | **D** | **MEASURED RED BEYOND SCOPE → REPORTED, NOT CHASED.** Own `from_raw_parts` over a raw heap pointer; owes nothing to the four families. |
| `interpreter/opcodes/binary/construction.rs:214` `BinaryBuilder::bytes` | **D** | same — calls `slice_from_words` |
| `jit/runtime_binary_build.rs:194` `bytes` | **D** | same — own `from_raw_parts` over the builder pointer |
| `jit/runtime.rs:400`, `jit/ir_exceptions.rs:343` `process_from_abi -> Option<&'static mut Process>` | **E** | the JIT ABI raw-pointer boundary; a real and separate soundness question, **not** one of the four families. Reported. |
| `jit/runtime_closure.rs:144` `runtime_cache` | **E** | JIT cache, not process heap — plan V6, benign |
| `native/stdlib_stubs/encoding_bifs.rs:88` `base64_alphabet` | **E** | a genuine `'static` constant — plan V6, **must not be "fixed"**; it sits in the same file as a Class C site that WAS fixed (`:205`) |
| `io/resource.rs:278` `inner_ref` | **E** | FD table, not process heap |
| `telemetry/metrics.rs:265`, `native/inet_bifs.rs:392` | **E** | process-wide statics |

**No unexplained residue anywhere in the workspace census.**

---

## 6. R3 — the caller inventory, re-counted

**Instrument, stated: the compiler's error list, not grep.** Grep is a
pre-estimate only (`.limbs()` collides with `BigIntValue::limbs`, `.as_bytes()`
with `str`/`String`/`SharedBinary`).

**Pre-implementation (library only, first compile after the signatures moved):
69 sites, 34 files, one crate.**

| Layer | Sites | Files |
|---|---|---|
| `native/` (BIF layer) | 36 | 19 |
| `term/` (compare, hash, format, json, bigint_math) | 9 | 5 |
| `ets/copy.rs` | 8 | 1 |
| `interpreter/opcodes/binary/` | 6 | 3 |
| **`jit/`** | **2** | **2** |
| `distribution/etf.rs`, `etf/encode.rs` | 4 | 2 |
| `mailbox/` | 3 | 1 |
| `io/standard_io.rs` | 1 | 1 |
| **Total (library)** | **69** | **34** |

**Post-implementation, the full count** — every target the gate compiles, not
just the library. OBSERVED, successive `cargo check --all-targets --keep-going`
rounds: 69 (lib) → 115 (lib test cfg + integration tests) → 41 → 27 → 87 → …
down to **0**, plus **14** in `beamr-wasm`'s `wasm32-unknown-unknown` **test**
target, which only the gate's `wasm-tests` leg reaches (see §8 — I found that one
by running the gate, not by assertion). Final state, OBSERVED:

```
$ cargo check --workspace --features beamr/encode --all-targets --keep-going   -> 0 errors
$ cargo check --workspace --all-features    --all-targets --keep-going         -> 0 errors
$ cargo check -p beamr-wasm --target wasm32-unknown-unknown --all-targets      -> 0 errors
```

### Two live hazards the compiler found, not I

REASONED, from the E0502s the tie produced:

1. **`ets/copy.rs:284-292`** (`copy_boxed_to_heap`) held `bigint.limbs()` across
   `heap.alloc_slice(...)`, and passed `binary.as_bytes()` / `proc_bin.as_bytes()`
   / `sub_binary.as_bytes()` straight into a function taking `&mut Heap`.
   `mailbox/mod.rs:413,466,496` had the same shape. All now own the bytes before
   allocating, and **the borrow checker refuses the old form**.
2. **`interpreter/opcodes/binary/matching.rs`** (`bs_get_tail`, the `bs_match`
   command loop) held a matched slice across `allocate_binary` /
   `allocate_extracted_binary`, both `&mut Process`. Same remedy, same
   enforcement. This is the interpreter's binary-matching path, and its JIT twin
   at `jit/runtime_binary_build.rs:90` — **exactly where Tom's "jit is not
   optional" ruling bites**.

---

## 7. R3 — the API-breakage list

**Breaking** changes to `beamr`'s public surface (all four families are public:
`lib.rs:62` → `term/mod.rs:10,11,12` → `term/boxed/mod.rs:13-16`):

| Item | Change |
|---|---|
| `term::boxed::BigInt::limbs` | gains `heap: HeapBorrow<'heap>`; returns `&'heap [u64]` |
| `term::binary::Binary::as_bytes` | gains `heap`; returns `&'heap [u8]` |
| `term::boxed::ProcBin::as_bytes` | gains `heap`; returns `&'heap [u8]` |
| `term::boxed::SubBinary::as_bytes` | gains `heap`; returns `&'heap [u8]` |
| `term::binary_ref::BinaryRef::as_bytes` | gains `heap`; returns `&'heap [u8]` |
| `term::bigint_math::BigIntValue::from_bigint` | gains `heap` |
| `term::bigint_math::BigIntValue::from_term` | gains `heap` |
| `term::bigint_convert::integer_term_to_string_radix` | gains `heap` |
| `beamr_wasm::convert::term_to_js_value` | gains `heap` |
| `beamr_wasm::convert::terms_to_js_array` | gains `heap` |

**Additive, non-breaking:** `term::heap_borrow::HeapBorrow`,
`Heap::borrow_terms`, `Process::borrow_terms`, `ProcessContext::borrow_terms`,
`ets::OwnedTerm::borrow_terms`, `constant_pool::ConstantPool::borrow_terms`.

**Internal only:** `SharedBinary::bytes_from_raw_word` (+`len_from_raw_word`,
new) are `pub(crate)`; `parent_bytes` is module-private; `MatchContext::slice` is
`pub(crate)`.

**Not a signature change:** `ProcBin` and `SubBinary` moved from
`term::boxed::accessors` to a new private sibling `term::boxed::binary_accessors`
and are re-exported unchanged — **the public paths
`beamr::term::boxed::{ProcBin, SubBinary}` are identical** (OBSERVED
`term/boxed/mod.rs:16` `pub use binary_accessors::{ProcBin, SubBinary};`).

**What this means for the next version:** every row above is source-breaking for
a downstream caller of the binary or bignum accessors. For a `0.x` crate that is
a minor-version break (`0.19.4` → `0.20.0` under the usual convention).
**REPORTED AS FACT — the version decision is not this flight's, and no version,
changelog or release metadata was touched.**

---

## 8. R3 — THE GATE. Differential, per leg.

Entry: `.fleet/gate-entry.sh` → `./scripts/ci-verdict.sh gate-rc gates.json`,
**unedited**. Per plan V1, the entry only *grades*; I ran the canon leg loop
myself, transcribed from `.github/workflows/ci.yml:68-112` (no `set -e`,
subshell around `eval`, redirect never pipe), **on the untouched tree first** and
again on the fixed tree.

| # | Leg | BASELINE rc | FINAL rc | Verdict |
|---|---|---|---|---|
| 1 | `fmt` | 0 | **0** | UNCHANGED — green |
| 2 | `clippy` | 101 | **101** | UNCHANGED — 15 errors, identical set |
| 3 | `wasm32-check` | 0 | **0** | UNCHANGED — green |
| 4 | `wasm-tests` | 0 | **0** | UNCHANGED — green, 86 passed |
| 5 | `tests` | 101 | **101** | UNCHANGED — same one failure, 2126 passed |
| 6 | `blocking-call-in-native-bif` | 0 | **0** | UNCHANGED — green |
| 7 | `clippy-all-features` | 101 | **101** | UNCHANGED — 15 errors, identical set |
| 8 | `tests-all-features` | 101 | **101** | UNCHANGED — same one failure, 2136 passed |
| 9 | `nostd-ratchet` | 3 | **3** | UNCHANGED — instrument dead on this venue |

**No leg that was green at baseline is red at the end. No leg changed rc.**

### The grader's own verdict, quoted

```
GATE SCOPE: beamr's own CI verdict harness (gates.json), the real mechanism
self-test: all-green -> gamma: rc=0 pass (expected)
self-test: one-fail -> FAIL — measured red (expected)
self-test: cannot-measure -> CANNOT-MEASURE (expected)
self-test: uncontracted-2 -> alpha: rc=2 FAIL (expected)
self-test: malformed-rc -> MALFORMED rc (expected)
self-test: truncated-set -> LEG COUNT MISMATCH (expected)
self-test: empty-tests -> TEST-COUNT (expected)
declared legs: 9
recorded legs: 9
  wasm-tests: 2 result line(s), 86 passed
  tests: 66 result line(s), 2126 passed
  tests-all-features: 66 result line(s), 2136 passed
  blocking-call-in-native-bif: rc=0 pass
  clippy-all-features: rc=101 FAIL — measured red
  clippy: rc=101 FAIL — measured red
  fmt: rc=0 pass
  nostd-ratchet: rc=3 FAIL — measured red
  tests-all-features: rc=101 FAIL — measured red
  tests: rc=101 FAIL — measured red
  wasm-tests: rc=0 pass
  wasm32-check: rc=0 pass
ENTRY_EXIT=1
```

The baseline run produced the identical block (`ENTRY_EXIT=1`, same five FAILs,
`tests` 2126 passed, `tests-all-features` 2136 passed, `wasm-tests` 86 passed).

### The three root causes, all pre-existing, none mine

1. **`clippy` / `clippy-all-features` — 15 errors, content identical.** OBSERVED,
   fingerprinted from the JSON diagnostics in both runs: `io/uring.rs` ×7
   (`unused_imports` `:10`, `dead_code` `:55`,`:63`, `io_other_error` `:138`,
   `collapsible_if` `:187`,`:188`, `while_let_loop` `:250`) and
   `native/file_meta_bifs.rs` ×8 (`unnecessary_cast` `u32`→`u32`). **Same lints,
   same code, same count, both legs, both runs.** The `file_meta_bifs` line
   numbers read `:251-:264` at baseline and `:253-:266` at the end — that is a
   **+2 line shift from the `use` line this flight added to that file**, not a
   new lint; the eight lints and their spans are otherwise identical.
2. **`tests` / `tests-all-features` — the same ONE test.** OBSERVED, both runs:
   `distribution_runtimes_exist_only_when_configured_and_are_joined_at_shutdown`
   (`crates/beamr/tests/thread_inventory_distribution.rs:59`) —
   *"an owned distribution bundle must build both runtime workers: dist-send=1,
   net-kernel=0"*. Every other `test result:` line in both legs shows `0 failed`.
   Mechanism OBSERVED at planning time (`distribution/mod.rs:191-197` swallows
   the net-kernel runtime build error with `builder.build().ok()`); root cause not
   established, **out of scope**. No SIGBUS on leg 8 in either run.
3. **`nostd-ratchet` — the instrument is dead on this venue.** OBSERVED
   `scripts/gate-nostd-ratchet.sh:182` `LOG="$(mktemp -t nostd-ratchet)"`;
   GNU coreutils here rejects it — the leg log's first line is
   `mktemp: too few X's in template 'nostd-ratchet'`. The script's own REFUSE arm
   then fires correctly: *"This gate cannot measure, so it does not get to
   report."* **I did not touch the script, and I could not lower `CEILING` even
   if the change earned it**, because the tally cannot be measured here.

**I fixed none of these, `#[allow]`ed nothing, and edited no gate script and no
gate entry.**

---

## 9. Hard walls — measured

| Wall | Measurement |
|---|---|
| No lint suppressions in any spelling | **0** `#[allow` added (census §4). The clippy findings this flight introduced (4 unused imports, 3 `unnecessary to_vec`) were **fixed in the code**, not silenced. |
| No ignore attributes on tests | **0** `#[ignore` added |
| Never `.unwrap()`/`.expect()` in library code | **0** `.unwrap(` added. The 36 `.expect(` in added lines are all in `#[cfg(test)]` modules, `*_tests.rs` files or `crates/beamr/tests/` — OBSERVED per-file breakdown. `SubBinary::as_bytes` keeps its `unwrap_or`/`checked_add` discipline through the rewrite (`binary_accessors.rs:140-145`). |
| No file over 500 lines of code | **No file crossed 500.** OBSERVED — of every `.rs` file this flight touched, none went from ≤500 to >500. The most-edited file, `term/boxed/accessors.rs`, went **472 → 390** (376 after the split; the five `compile_fail` doctests add the rest) via the planned split; `term/compare/mod.rs` went **713 → 663**. New files: `boxed/binary_accessors.rs` 177, `compare/bigint.rs` 104, `term/heap_borrow.rs` 81. ⚠️ **REPORTED AGAINST ME:** three of the four files the brief named as "already over the wall, must not grow" **did grow** — `process/mod.rs` 1361→1370 (+9, the `borrow_terms` method), `native/context/mod.rs` 1815→1830 (+15, same), `native/stdlib_stubs/gc_rooting_tests.rs` 552→573 (+21, witness arguments on test helpers). The change cannot add an inherent method to `Process`/`ProcessContext` without adding lines, and I did not game the count. I trimmed the duplicated doc prose on those two methods to the minimum that still points at the canonical explanation; the rest is irreducible. |
| Every touched `unsafe` carries a re-derived safety comment | 30 `unsafe` / 27 `SAFETY` added; the arguments now reason from the witness borrow, not from `'static` |
| No new dependencies / version bumps / changelog / release prep | `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md` untouched — OBSERVED, not in `git diff --name-only 6b1f85cc..HEAD` |
| Never one blocking sleep sized to expected duration | Every long check was polled at a stated interval under a stated deadline (miri: 15 s / 90–105 s; gate: 20 s / 100 s per call), and each expiry re-polled rather than assumed |

---

## 10. Flagged follow-on — the divergent branch, BY MEASUREMENT

⚠️ **Re-merging this fix into the divergent working branch is the OWNER's work.
It is not done here.**

OBSERVED at flight time — `git for-each-ref` + `git rev-list --left-right --count
main...<ref>`, **excluding `refs/*/fleet/*` and `origin/HEAD`** (plan V5: those
four ancestor hits are this workflow's own refs, and reporting one as "the
divergent working branch" would be the stale-shape error from the opposite
direction):

```
REF                                             BEHIND    AHEAD main-is-ancestor
origin/artemis/backport-0.17.1                     204        3 no
origin/artemis/jit-operator-switch                  23        2 no
origin/diana/beamr-wedge002-3aecb622               280        3 no
origin/diana/seat-preserve-annabelbox              270        4 no
origin/feat/jit-cache-enumeration                   18        1 no
origin/fix/0163-borrow-across-alloc                401       15 no
origin/juniper/docs-package                        526        1 no
origin/seth/replay-rebuild-3aecb622                296        3 no
```

- **`main` is an ancestor of NO divergent working branch.** The audit's shape —
  *"173 commits ahead with `main` an ancestor"* — **describes nothing in this
  repository**, exactly as gate binding note 2 anticipated.
- **The topical branch is `origin/fix/0163-borrow-across-alloc`** — the prior
  0.16.3 call-site-sweep lane for this very defect: **401 behind / 15 ahead**,
  merge-base `67f89c41`. OBSERVED —
  `git diff --stat main...origin/fix/0163-borrow-across-alloc -- crates/beamr/src/term/`
  is **empty**: it carries **no** accessor changes to re-merge. Its 15 commits
  touch 38 files (`gate-logs/`, `gates.json`, `rust-toolchain.toml`, …).
- **Excluded as this workflow's own:** `origin/fleet/accessor-lifetimes-base`,
  `origin/fleet/beamracc-flight1-flagged`, `origin/HEAD`, and this leg's own
  `leg/accessor`.

---

## 11. Measured reds beyond this scope — REPORTED, NOT CHASED

1. **Class D, the binary-construction family** — same defect class, different
   source, not reachable through the four accessor families (the compiler
   confirmed: none of them broke):
   `interpreter/opcodes/binary/mod.rs:91` `slice_from_words`,
   `interpreter/opcodes/binary/construction.rs:214` `BinaryBuilder::bytes`,
   `jit/runtime_binary_build.rs:194` `bytes`. Each manufactures a `'static`
   slice with its own `from_raw_parts` over a raw heap pointer.
2. **`process_from_abi -> Option<&'static mut Process>`** (`jit/runtime.rs:400`,
   `jit/ir_exceptions.rs:343`) — a `&'static mut` over a raw ABI pointer. A real
   and separate soundness question at the JIT boundary.
3. **`jit_bs_get_integer` and `jit_bs_get_utf*` receive no process pointer**
   (`jit/runtime_binary_match.rs:44`, `:315`). Closing those two properly needs an
   ABI change in the JIT compiler's call emission — beyond this flight.
4. **The `Term`-has-no-lifetime root cause.** miri stopped on the SubBinary red
   *before* the slice escaped, inside `SubBinary::new`, because a bare `Term`'s
   exposed pointer tag is already invalidated by an intervening `&mut Heap`
   allocation. Only T5 (`Term<'heap>`) closes that, and T5 is not representable
   while `Process` stores `Term`s beside the `Heap` they point into (design §5).
5. **The gate's three pre-existing reds** (§8): the 15 clippy lints, the
   `thread_inventory_distribution` failure, and the dead `nostd-ratchet`
   instrument.

---

## 12. Lane boundary

This flight produces a **CANDIDATE result branch**, `leg/accessor`. **Nothing
was merged to beamr `main`, and nothing may be merged at any seat in this
workflow** — the landing is ratified by the repo owner's seat.

---

## 13. Report receipt

Path is the wire contract: `.fleet/reports/accessor.md`, byte-exact. Confirm
with `git ls-tree HEAD -- .fleet/reports/`.
