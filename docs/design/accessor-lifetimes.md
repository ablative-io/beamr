# Accessor lifetimes — retiring the `&'static` exposure at the source

Status: design of record for the change that ties borrowed binary and bignum
slices to a shared borrow of the storage they point into.

Every claim below is marked **OBSERVED** (command output or `file:line` read on
the tree) or **REASONED**. Coordinates are from `main` `c55ac360` as carried on
this branch, re-derived here and not copied from any prior document.

---

## 0. The defect

**OBSERVED** — five signatures in the four accessor families return `&'static`
slices over memory that a garbage collection can move, reclaim or release:

| # | Coordinate | Signature | Family |
|---|---|---|---|
| 1 | `crates/beamr/src/term/boxed/accessors.rs:113` | `pub fn limbs(self) -> &'static [u64]` | BigInt |
| 2 | `crates/beamr/src/term/boxed/accessors.rs:281` | `pub fn as_bytes(self) -> &'static [u8]` | ProcBin |
| 3 | `crates/beamr/src/term/boxed/accessors.rs:340` | `pub fn as_bytes(self) -> &'static [u8]` | SubBinary |
| 4 | `crates/beamr/src/term/boxed/accessors.rs:357` | `fn parent_bytes(parent: Term) -> Option<&'static [u8]>` | SubBinary (private helper) |
| 5 | `crates/beamr/src/term/shared_binary.rs:79` | `pub(crate) fn bytes_from_raw_word(raw: u64) -> &'static [u8]` | SharedBinary |

**OBSERVED** — two more members of the same families carry the same defect and
the tie cannot complete without them:

- `crates/beamr/src/term/binary.rs:70` — `Binary::as_bytes(self) -> &'static [u8]`.
  Called directly by `parent_bytes` (`accessors.rs:359`).
- `crates/beamr/src/term/binary_ref.rs:30` — `BinaryRef::as_bytes(&self) -> &'static [u8]`,
  the fan-in that dispatches to all three binary families (`binary_ref.rs:31-35`).

`&'static` is a *lie in the type system*: it says the bytes outlive the program.
They outlive the boxed object that owns them, which a collection can end. The
0.16.2/0.16.3 remediation swept five `string_bifs.rs` call sites; the signatures
that manufacture the next unsound caller survived that sweep untouched
(**OBSERVED** — `string_bifs.rs:351` `utf8_str`, `:355` `binary_bytes`, both
still `-> &'static`).

---

## 1. (a) The chosen mechanism

### `HeapBorrow<'heap>` — a witness token carried in argument position

A new term-layer type, `crates/beamr/src/term/heap_borrow.rs`:

```rust
#[derive(Copy, Clone, Debug)]
pub struct HeapBorrow<'heap> {
    storage: PhantomData<&'heap [u64]>,
}

impl<'heap> HeapBorrow<'heap> {
    pub const fn of_words(words: &'heap [u64]) -> Self { … }
}
```

and the byte/limb accessors take it **by value, in argument position**:

```rust
impl BigInt    { pub fn limbs<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u64] }
impl Binary    { pub fn as_bytes<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u8] }
impl ProcBin   { pub fn as_bytes<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u8] }
impl SubBinary { pub fn as_bytes<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u8] }
impl BinaryRef { pub fn as_bytes<'heap>(&self, heap: HeapBorrow<'heap>) -> &'heap [u8] }
fn parent_bytes<'heap>(parent: Term, heap: HeapBorrow<'heap>) -> Option<&'heap [u8]>
impl SharedBinary { fn bytes_from_raw_word<'heap>(raw: u64, heap: HeapBorrow<'heap>) -> &'heap [u8] }
```

Witness sources, each constraining `'heap` from a **shared reference argument**:

| Producer | Signature |
|---|---|
| `process::heap::Heap` | `pub fn borrow_terms(&self) -> HeapBorrow<'_>` |
| `process::Process` | `pub fn borrow_terms(&self) -> HeapBorrow<'_>` |
| `native::ProcessContext<'_>` | `pub fn borrow_terms(&self) -> HeapBorrow<'_>` |
| word storage directly (tests, decoded scratch heaps) | `HeapBorrow::of_words(&words)` |

`HeapBorrow` is `Copy` and zero-sized: it carries no data, only the borrow
region. Passing it costs nothing at run time; what it costs is a *borrow*.

### Why the witness sits on the accessor method, not the constructor

The plan's T3 puts the witness on `X::new`. **OBSERVED** — the pre-estimate for
construction sites is 279 (`BinaryRef::new` 94, `Binary::new` 111, `BigInt::new`
37, `ProcBin::new` 23, `SubBinary::new` 14; `rg` over `crates/`), against **69**
compiler-measured byte-reading sites in the library (§2). **REASONED**: the two
placements are equally strong — in both, an argument constrains the returned
lifetime — so the smaller diff wins. Putting it on `new` would additionally
force a witness onto `len()`, `is_empty()` and every `BinaryRef::new` that never
touches a byte.

Two consequences of that choice, both handled:

- `ProcBin::len` was `self.as_bytes().len()`. It is rewritten to read the
  `Vec`'s length through `SharedBinary::len_from_raw_word` — no slice is
  produced, so no witness is needed.
- `SubBinary::new` used `parent_bytes(…)?.len()` for its bounds check
  (`accessors.rs:319-323`). It is rewritten against a new `parent_len` helper,
  for the same reason. Constructing an accessor stays witness-free; only
  *reading bytes* needs a live borrow.

### The one place raw pointers become slices

`HeapBorrow::slice` is the single `unsafe` primitive in the tied path:

```rust
pub(crate) const unsafe fn slice<T>(self, ptr: *const T, len: usize) -> &'heap [T]
```

`'heap` comes from `self`, which was constrained at construction. There is no
other way to obtain a `&'heap [T]` in the accessors, so caller inference cannot
widen the result — see §4.

---

## 2. (b) Caller inventory

**Instrument, stated**: the authoritative count is **the compiler's error list**
after the signatures changed, produced by a discarded implementation spike
(`cargo check --workspace --features beamr/encode --all-targets
--message-format=short`). Grep counts are a pre-estimate only: `.limbs()`
collides with `BigIntValue::limbs`, `.as_bytes()` with `str`/`String`/
`SharedBinary`.

**OBSERVED** — **69 call sites, all `E0061` (missing witness argument), in one
crate (`beamr`), across 34 files.** The library is the whole of the count
because compilation stops at the library: test-target and doctest sites are
additional and are re-counted after implementation in the report.

| Layer | Sites | Files |
|---|---|---|
| `native/` (BIF layer) | 36 | 19 |
| `term/` (compare, hash, format, json, bigint_math) | 9 | 5 |
| `ets/copy.rs` | 8 | 1 |
| `interpreter/opcodes/binary/` | 6 | 3 |
| `mailbox/` | 3 | 1 |
| `distribution/etf.rs`, `etf/encode.rs` | 4 | 2 |
| **`jit/`** | **2** | **2** |
| `io/standard_io.rs` | 1 | 1 |
| **Total** | **69** | **34** |

Full per-file listing (**OBSERVED**, spike error output):

```
8  ets/copy.rs                                    2  interpreter/opcodes/binary/matching.rs
6  native/stdlib_stubs/json_bifs.rs               2  etf/encode.rs
4  term/json.rs                                   2  distribution/etf.rs
4  native/stdlib_stubs/misc_bifs.rs               1  term/hash.rs
3  native/stdlib_stubs/type_conversion_bifs.rs    1  term/format.rs
3  native/meridian_ffi.rs                         1  term/bigint_math.rs
3  native/gate3_bifs/type_conversion.rs           1  native/udp_bifs.rs
3  mailbox/mod.rs                                 1  native/tcp_bifs.rs
3  interpreter/opcodes/binary/construction/segments.rs
2  term/compare/mod.rs                            1  native/stdlib_stubs/uri_bifs.rs
2  native/stdlib_stubs/io_bifs.rs                 1  native/stdlib_stubs/string_bifs.rs
2  native/file_bifs.rs                            1  native/stdlib_stubs/gleam_stdlib_ffi2.rs
2  native/etf_bifs.rs                             1  native/stdlib_stubs/encoding_bifs.rs
1  native/otp_stubs/erlang_stubs.rs               1  native/gate3_bifs/mod.rs
1  native/gate3_bifs/additional.rs                1  native/file_meta_bifs.rs
1  native/code_management_bifs.rs                 1  io/standard_io.rs
1  jit/runtime_binary_match.rs                    1  jit/runtime_binary_build.rs
1  interpreter/opcodes/binary/construction.rs
```

**The JIT is included and is not optional**: `jit/runtime_binary_match.rs:221`
and `jit/runtime_binary_build.rs:90` are in the count, and
`jit/runtime_binary_match.rs:212` `fn slice(self, bits) -> Option<&'static [u8]>`
is a compiler-forced consumer that the tie retires with them.

### Two call sites the compiler proved are the real hazard

**OBSERVED** — `crates/beamr/src/ets/copy.rs:275-280`:

```rust
if let Some(bigint) = BigInt::new(term) {
    let limbs = bigint.limbs();                        // borrow of source storage
    let words = heap.alloc_slice(3 + limbs.len())…;    // &mut Heap
    return boxed::write_bigint(words, bigint.is_negative(), limbs)  // read after
```

and `:296-303`, which pass `binary.as_bytes()` / `proc_bin.as_bytes()` /
`sub_binary.as_bytes()` straight into a function that takes `&mut Heap`. Under
the tie these are borrow-checker conflicts unless the source witness is distinct
from the destination heap — which is exactly the question the old signature
allowed nobody to ask. `mailbox/mod.rs:413,466,496` have the same shape.

---

## 3. (c) API-breakage assessment

**OBSERVED** — all four families are public: `lib.rs:62` `pub mod term;` →
`term/mod.rs:10,11,12` `pub mod binary; pub mod binary_ref; pub mod boxed;` →
`term/boxed/mod.rs:12` `pub use accessors::{…, ProcBin, …, SubBinary, …};`.

**Breaking changes to the `beamr` crate's public surface:**

| Item | Before | After |
|---|---|---|
| `term::boxed::BigInt::limbs` | `fn limbs(self) -> &'static [u64]` | `fn limbs<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u64]` |
| `term::binary::Binary::as_bytes` | `fn as_bytes(self) -> &'static [u8]` | `fn as_bytes<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u8]` |
| `term::boxed::ProcBin::as_bytes` | `fn as_bytes(self) -> &'static [u8]` | `fn as_bytes<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u8]` |
| `term::boxed::SubBinary::as_bytes` | `fn as_bytes(self) -> &'static [u8]` | `fn as_bytes<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u8]` |
| `term::binary_ref::BinaryRef::as_bytes` | `fn as_bytes(&self) -> &'static [u8]` | `fn as_bytes<'heap>(&self, heap: HeapBorrow<'heap>) -> &'heap [u8]` |

**Additive, non-breaking:** `term::heap_borrow::HeapBorrow`,
`Heap::borrow_terms`, `Process::borrow_terms`, `ProcessContext::borrow_terms`.

**Internal only (`pub(crate)`, no external breakage):**
`SharedBinary::bytes_from_raw_word` gains the witness;
`SharedBinary::len_from_raw_word` is new. `parent_bytes` is private to its
module.

**Not a signature change but a module move:** `ProcBin` and `SubBinary` move
from `term::boxed::accessors` to a new private sibling
`term::boxed::binary_accessors`. Both are re-exported from `term::boxed`
unchanged, so **the public path `beamr::term::boxed::{ProcBin, SubBinary}` is
identical**. The move exists because `accessors.rs` is at 472 lines
(**OBSERVED** `wc -l`) against a 500-line file wall, and lifetime parameters
plus re-derived safety comments do not fit in 28 lines.

**What this means for the next version:** every one of the five public changes
is source-breaking for any downstream caller of the binary or bignum accessors.
Under semver this is a **major-version-compatible break** for a `0.x` crate —
i.e. a minor bump of `0.y`. **This document reports the fact; the version
decision is not this change's to make**, and no version, changelog or release
metadata is touched here.

---

## 4. (d) Why the GC/borrow interaction becomes unrepresentable

### The allocation and collection surface is uniformly `&mut`

**OBSERVED**:

| Coordinate | Signature |
|---|---|
| `process/heap.rs:285` | `Heap::alloc(&mut self, words) -> Result<*mut u64, HeapFull>` |
| `process/heap.rs:298` | `Heap::alloc_slice(&mut self, words) -> Result<&mut [u64], HeapFull>` |
| `process/mod.rs:445` | `Process::heap_mut(&mut self) -> &mut Heap` |
| `gc/mod.rs:101` | `gc::collect_minor(process: &mut Process)` |
| `gc/mod.rs:118` | `gc::collect_major(process: &mut Process)` |
| `gc/mod.rs:135` | `gc::alloc(process: &mut Process, words)` |
| `gc/mod.rs:149` | `gc::ensure_space(process: &mut Process, words, live_x)` |
| `gc/mod.rs:282` | `gc::release_refcounted_resources_in_young(process: &mut Process, …)` |
| `gc/mod.rs:321` | `gc::release_all_refcounted_resources(process: &mut Process)` |
| `process/gc.rs:43` | `root_set(process: &mut Process, live_x) -> RootSet` |

**REASONED** — a `HeapBorrow<'heap>` is produced from `&Heap`, `&Process` or
`&ProcessContext`. While it (or any slice derived from it) is live, that shared
borrow is live, so the `&mut` every row above requires **cannot be produced**.
"Hold a slice across a GC-triggering allocation" is not a thing the borrow
checker permits a program to say — it is E0502/E0499/E0505 at the point the
allocation is attempted. Nothing is checked at run time; there is no generation
counter, no rooting registry, and no `debug_assert` in the mechanism.

### The `Arc` case is not an exception

`SharedBinary::bytes_from_raw_word` points at `Arc<Vec<u8>>` memory, which GC
never moves — so a naive reading says `'static` is harmless there. It is not.
**OBSERVED** — GC *releases* that `Arc`: `gc/mod.rs:282`
`release_refcounted_resources_in_young`, `:310`
`release_all_refcounted_resources_in_heap`, `:321`
`release_all_refcounted_resources`, all reaching `release_proc_bin_arc(ptr)`.
When the owning ProcBin dies in a collection the strong count drops and the
`Vec` can be freed under the slice. **REASONED** — the bound is therefore the
ProcBin's liveness, which is the heap's, so the same witness is the right one.

### The measured exception, and why it closes

**OBSERVED** — `gc/mod.rs:310`
`pub(crate) fn release_all_refcounted_resources_in_heap(heap: &Heap)` **frees**
ProcBin `Arc`s through a **shared** `&Heap`, walking via
`Heap::visit_boxed_objects(&self, …)` (`process/heap.rs:452`). A `&Heap`-derived
witness is not, on its face, protected against it.

**OBSERVED** — its sole caller is `replay/driver.rs:34`, inside
`impl Drop for DecodedHeaps` (`:31-37`), over `Vec<Heap>` scratch heaps that
never run a GC.

**REASONED** — the hole closes by the same mechanism, not by an argument outside
it: `Drop::drop` takes `&mut self`. A live `HeapBorrow<'heap>` derived from
`&heaps.0[i]` holds a shared borrow of that `Heap`, which is a shared borrow of
`*heaps`; `&mut self` cannot be produced while it lives, so the drop — and
therefore the release walk — is unreachable for as long as any borrowed slice
into those heaps exists. The signature does not need `&mut Heap` for the bound
to hold. This function is **not modified**: it is not one of the four families.

### Why a lifetime parameter alone would prove nothing

**REASONED** — a lifetime that appears only in the return type, or only in a
`PhantomData` field, is *unconstrained*: the caller's inference picks whatever
it needs, `'static` included. That passes a `'static` sweep and keeps the bug.
The mechanism is walled against it in three places:

1. `HeapBorrow<'heap>` cannot be constructed without a `&'heap` argument —
   `of_words(words: &'heap [u64])`, or the `&self` of a `borrow_terms`.
2. Every retired accessor names `heap: HeapBorrow<'heap>` **in argument
   position**, and `'heap` in the return type is *that* parameter's.
3. `HeapBorrow::slice` is the only route from a raw pointer to a slice inside
   the tied path, and it takes `self`, so its output lifetime is the witness's.

**No `transmute` in any spelling, no pointer round-trip that widens a lifetime,
and no fresh `slice::from_raw_parts` outside `HeapBorrow::slice` appears in the
tied path.** The report re-runs that as a census over the diff.

### The residual, stated plainly

**REASONED** — the tie proves *a shared borrow of term storage is live*. It does
not prove the term being read points into *that* storage: `BigInt::limbs` cannot
check that its bignum lives in the heap whose witness it was handed. This is not
a weakness introduced here — it is exactly the limitation of the mechanism the
brief prescribes (`&Process`/`&Heap`-tied slices), where nothing checks that the
`&Heap` passed is the one the term lives in either. Closing it needs a lifetime
on `Term` itself, which §5 shows is not representable.

### Readers that cannot be handed a witness

**OBSERVED** — three `std` trait implementations read heap bytes through
signatures that have no room for a witness argument:

- `term/mod.rs:65-69` `impl PartialEq for Term` → `compare::partial_eq` →
  `compare::binary_bytes` (`compare/mod.rs:337`);
- `term/mod.rs:79-83` `impl Ord for Term` → `compare::raw_cmp`;
- `term/hash.rs:43-47` `impl Hash for EtsKey` → `term_hash` → `hash.rs:157`
  `state.write(binary.as_bytes())`.

**REASONED** — no parameter can be added to `PartialEq::eq`, `Ord::cmp` or
`Hash::hash`. For these, and only these, the design provides

```rust
pub(crate) unsafe fn with_frame<R>(f: impl for<'frame> FnOnce(HeapBorrow<'frame>) -> R) -> R
```

a higher-ranked scope: neither the witness nor any slice derived from it can
escape `f`, because `R` is chosen independently of `'frame`. It is an `unsafe
fn`, so every use carries a written safety argument — that the closure performs
no allocation, collection or heap drop. This is **weaker than the tie** (it
bounds escape, not allocation-inside-the-scope) and **strictly stronger than
`'static`** (which bounded nothing). Every site that uses it is listed in the
report. It is `pub(crate)`: it is not part of the public API and no BIF, the
interpreter or the JIT may reach for it — all of those have a real witness.

---

## 5. Alternatives rejected

**T5 — lifetime-parameterise `Term<'heap>` itself.** The strongest and most
uniform tie, and the only one that would also cover §4's `std`-trait readers.
**Rejected on representability, not cost.** **OBSERVED** — `process/mod.rs:165`
`heap: Heap` sits in the same struct as `:178` `dictionary: Vec<(Term, Term)>`,
`:183` `x_regs: [Term; X_REG_COUNT]` and `:188` `native_roots: Vec<Term>`, all of
which hold terms pointing into that heap. **REASONED** — `Term<'heap>` would
make `Process` a self-referential struct (`x_regs: [Term<'?>; N]` borrowing
`self.heap`), which safe Rust cannot express. The same applies to `Mailbox` and
to `ets::OwnedTerm`. T5 is not a bigger version of this change; it is a
different program.

**T1/T2 with `&Heap`/`&Process` directly in the accessor signatures.**
**Rejected on layering and on reach.** **OBSERVED** — `process/heap.rs:11` `use
crate::term::boxed::{BoxedHeader, BoxedTag};`: `process` depends on `term`.
**OBSERVED** — `term/` imports `crate::process` in exactly two places, both test
modules (`term/json.rs:456`, `:742`). Putting `&Heap` in `term/`'s accessor
signatures inverts that dependency. It also cannot express the storage the term
layer's own tests use — stack `[u64; N]` arrays written by `write_binary`
(`binary.rs:86,101,114`) — which are term storage without being a `Heap`.
`HeapBorrow` keeps the tie and drops both problems: `Heap::borrow_terms` and
`Process::borrow_terms` are the intended producers, they just live on the
`process` side of the dependency.

**T3 — witness at accessor construction.** Equal strength, larger diff (279
construction sites vs 69 reading sites, §2), and it charges a witness to callers
that only ask for `len()`. Rejected on cost alone.

**T4 — witness from `&ProcessContext`.** Cannot be the sole mechanism:
`term/compare`, `term/hash`, `ets/copy.rs`, `mailbox/`, the interpreter's binary
opcodes and the JIT runtime hold no `ProcessContext`. **Adopted as a witness
*source*** — `ProcessContext::borrow_terms(&self)` — because it is what the BIF
layer has in hand, and because `ProcessContext`'s allocators take `&mut self`
(`context/alloc.rs:138` `alloc_binary(&mut self, …)`), so the conflict lands
exactly where a BIF would have dangled.

**Debug-asserts and GC generation counters.** Rejected by ruling: release builds
hold the real data, so a debug-only check is not a bound. They may exist as
defence in depth; they are not the criterion. None is added here.

**Return owned `Vec<u8>` / copy out.** Removes the dangle and changes the cost
model of every binary BIF and every message send. Performance work is out of
scope in both directions, and a silent copy is not a lifetime fix. (Where a
*copier* must own bytes before allocating — `ets/copy.rs`, `mailbox/` — the copy
is the operation being performed, not a substitute for the tie.)

**`&self`-elision alone** — `fn limbs(&self) -> &[u64]`. Ties the slice to a
borrow of the accessor struct, which is a `Copy` raw pointer on the caller's
stack — not to the heap. It kills the `BigInt::new(t).limbs()` temporary
pattern, which is a real improvement, but `let b = BigInt::new(t)?;` followed by
holding `b.limbs()` across an allocation still compiles. **Insufficient alone**,
and shipping it as the tie would be the fake fix this design exists to avoid.

---

## 6. Evidence plan

- **Red, before the change**: `docs/evidence/beamr-accessor-dangle-red.txt` —
  the dangle demonstrated with the instrument named in the file.
- **Green, after the change**: the same program is a **compile error**. Captured
  verbatim, per accessor family, and pinned in-gate as ` ```compile_fail,E0502 `
  rustdoc doctests — the harness the repository already carries
  (**OBSERVED** — `process/mod.rs:146`, `:155`). No new dependency.
- **Sweep**: zero `'static` returns over heap memory remain in the four
  families, `accessors.rs:357` `parent_bytes` in the found-and-retired list.
