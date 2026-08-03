# gate-logs/70-sender-extract — lane #70 SENDER-EXTRACT

Evidence for the split of `crates/beamr/src/distribution/sender.rs` (2,168 lines
at base `e2c406e404db884e6a2f2039097791b4bd740b80`) into a `sender/` child module
of three files. **No behavior changes. No new tests. No new public surface.**

## NO RED LEG — RULED OUT

**NO RED LEG — RULED OUT (Cally 1e191d8b, #67 Q4 precedent: a split has no
behavior to demonstrate failing; a red would be theatre.)** Nothing in this lane
alters a single executable token, so there is no behavior whose absence could be
demonstrated first. The instrument that stands in for a red here is the
**mechanical census** (`census.sh`): it exits non-zero the instant one moved byte
differs, one base line is unaccounted, or one landed line appears from nowhere.

## The re-export set is EMPTY — as expected, confirmed at the bytes

The brief's expected outcome was an EMPTY re-export set, and that is what landed.
`grep -rn 'pub use|pub(crate) use|pub(super) use' sender/` returns **nothing**.
Every externally-reachable item stayed in `mod.rs`:

| item | visibility | landed at |
| --- | --- | --- |
| `DIST_SEND_THREAD_NAME` | `pub` | `sender/mod.rs:191` |
| `DIST_SEND_QUEUE_CAP` | `pub` | `sender/mod.rs:207` |
| `DIST_SEND_QUEUE_BYTE_BUDGET` | `pub` | `sender/mod.rs:223` |
| `DIST_CONTROL_QUEUE_CAP` | `pub` | `sender/mod.rs:239` |
| `WRITE_TIMEOUT` | `pub(crate)` | `sender/mod.rs:251` |
| `DistOutbound` | `pub` | `sender/mod.rs:258` |
| `ControlOutbound` | `pub` | `sender/mod.rs:276` |
| `ControlEnqueueError` | `pub` | `sender/mod.rs:288` |
| `DistSender` | `pub` | `sender/mod.rs:353` |

Everything that moved to `residency.rs` is `pub(super)` and nothing else —
**ZERO `pub(crate)`, ZERO `pub`** (the eight tokens are enumerated as E-b in
`EDITS.txt`). The module path `distribution::sender::tests` is preserved by
`#[cfg(test)] mod tests;` in `mod.rs`, so no test path moved either: the 22
sender test names run by leg 5 are byte-identical to the 22 in the base file.

## Shape

| file | lines | contents |
| --- | --- | --- |
| `sender/mod.rs` | 584 | module doc (byte-intact) + Design Sentence, imports, consts, `DistOutbound`/`ControlOutbound`, `ControlEnqueueError`, `DistSenderInner`+`Drop`, `DistSender`+impl, FUTURE comment |
| `sender/residency.rs` | 95 | the residency cluster WHOLE: `LaneResidency`(+impl), `ResidencyCharge`(+`Drop`), `ChargedOutbound` |
| `sender/tests.rs` | 1,504 | the entire `cfg(test)` body, de-nested |

## The census

`census.sh` counts, and its exit code is the verdict:

```
full-diff e2c406e..HEAD  ==  SUM(byte-intact block moves) + SUM(enumerated edits)
remainder EMPTY
```

`census.rc` = **0**. `census.log` records:

- **12** block moves declared, **12** block diffs rc **0** (`block-NN.diff` /
  `block-NN.rc`).
- OLD-side partition over base 1..2168 **EXACT**: 2,153 block-covered + 15
  enumerated edit lines, no gap, no overlap.
- NEW-side partition **EXACT** on all three landed files: `mod.rs` 567 + 17,
  `residency.rs` 82 + 13, `tests.rs` 1,504 + 0.
- `etf.rs`'s working diff is **exactly** the enumerated E-c pair.
- Working tree confined to `sender.rs` (deleted), `sender/`, `etf.rs`, and this
  directory — checked over TRACKED changes *and* over `git status
  --untracked-files=all`, because `git diff` alone cannot see a stray new file.
- Code bar counted base-vs-landed: `#[allow]` 0/0, `_ =>` 0/0, `.unwrap(` 0/0,
  `expect` 149/149, `panic` 5/5.

### Dedent direction — a disclosed departure from the brief's wording

Block 12 (`tests.rs`) carries the `dedent4` transform. The BASE side is the
indented one — it lives inside `mod tests {` — and the LANDED side is already at
file top level. The transform is therefore applied to the **BASE** extract before
the diff, following the #67 precedent
(`gate-logs/67-dist-extract/census.sh`). The dispatching brief's evidence
paragraph says to apply `sed 's/^    //'` to the *landed* side; that would strip
four spaces off an already-dedented side and could never reach rc 0. **Disclosed,
not absorbed** — the note is repeated in `census.sh`'s header.

## Edit categories (`EDITS.txt`, 47 lines)

| category | lines | what |
| --- | --- | --- |
| E-a | 19 | plumbing: 7 consumed base lines, 7 new `mod.rs` lines (`mod residency;`, `#[cfg(test)] mod tests;`, `use residency::{ChargedOutbound, LaneResidency};`, separators), 5 new `residency.rs` use-lines |
| E-b | 16 | the 8 `pub(super)` tokens, each as an OLD/NEW pair |
| E-c | 12 | `etf.rs:60` deictic re-point (OLD/NEW pair) + the 10 Design Sentence lines in `mod.rs` |
| E-f | 0 | **none — `cargo fmt --all --check` forced nothing** |

E-f being empty is the interesting one: the four-space de-nest did not push a
single wrapped construct back onto one line, so `tests.rs` is the base bytes
minus four leading spaces and nothing else.

### The one base import that could not stay

Base `sender.rs:159` (`use std::sync::atomic::{AtomicUsize, Ordering};`) is
consumed (E-a). Its only consumers were inside the residency cluster, which left
for `residency.rs`; leaving it in `mod.rs` would be an unused import and leg 2
runs `-D warnings`. The equivalent import is re-stated in `residency.rs`.

### The Design Sentence placement — a judgment call, disclosed

The brief pins `mod.rs`'s module doc `:1-157` as BYTE-INTACT, so the Design
Sentence can only be appended after `:157`. Appended as a bare paragraph it would
have read as the tail of the preceding `## Wedged-peer write deadline` section,
which is false. **One line beyond the brief-verbatim sentence was therefore
written** — the heading `//! ## Module shape — what deliberately stays in
`mod.rs`` at `mod.rs:159` — plus its two `//!` spacers. All ten lines are
enumerated individually as E-c in `EDITS.txt`; the sentence itself is verbatim.
Flagged for the coordinator rather than absorbed.

## Quote parity — EQUAL, no E-f delta

`quote-parity-pre.txt` (base bytes) vs `quote-parity-post.txt` (landed
`tests.rs`). The comparable scope is the base `cfg(test)` region `662-2168`,
which is exactly what becomes `tests.rs`.

| instrument | base test region | landed `tests.rs` | expected |
| --- | --- | --- | --- |
| token-boundary raw strings | 0 | 0 | 0 |
| odd-quote (multi-line-string) lines | 22 | 22 | 22 = 11 strings |
| continuation lines | 14 | 14 | 14 |

**Posterior counts EQUAL prior counts on all three axes**, so the ruled
condition's first arm is satisfied and no E-f enumeration is owed. (The base
whole-file odd-quote figure is 24: two of those lines live in the prod-side docs
and stayed in `mod.rs`; recorded so the residue is visible, not hidden.)

## Deictic verification, per sentence

| deictic | claim | verdict |
| --- | --- | --- |
| `mod.rs:70` "The budgets below bound what the two QUEUES retain" | the budgets are below this line, in this file | **TRUE, no edit** — `DIST_SEND_THREAD_NAME` 191, `DIST_SEND_QUEUE_CAP` 207, `DIST_SEND_QUEUE_BYTE_BUDGET` 223, `DIST_CONTROL_QUEUE_CAP` 239, `WRITE_TIMEOUT` 251. Line number unchanged from base. |
| `tests.rs:905` (base `1568`) "Bounded by the harness only through the channel timeout below" | the `recv_timeout` is below, in the same block | **TRUE** — `tests.rs:915` `.recv_timeout(Duration::from_secs(10))`; both lines are inside the same moved block. |
| `tests.rs:942` (base `1605`) "The tests below are the RED-FIRST artifact for lane #64" | the #64 tests follow, in the same file | **TRUE** — the #64 banner block travelled whole; first `#[test]` after it at `tests.rs:1033`, last at `tests.rs:1479`. |
| `etf.rs:60` "(`distribution/sender.rs`)" | names where `DIST_SEND_QUEUE_CAP`/`DIST_CONTROL_QUEUE_CAP` live | **WAS FALSE AFTER THE SPLIT, RE-POINTED** to "(`distribution/sender/`)", mirroring #67's `sender.rs:204` → `distribution/connection/` treatment. The consts stay in `sender/mod.rs`. |

No intra-doc link needed re-pathing: **zero** of the prod-side `[`…`]` links name
`LaneResidency`, `ResidencyCharge` or `ChargedOutbound`, and the residency
cluster's own doc comments contain **no** intra-doc links at all. The ~20 stale
`docs/` spec pins and the historical `gate-logs/` were left untouched, as ruled.

## Battery — 6/6 green

Legs run VERBATIM from the worktree-root `gates.json`, in order. Before each leg
`df -k /System/Volumes/Data` was captured to `leg-N.df` and its Available column
checked against `THRESHOLD.txt` (45,088,768 KiB). Logs are by redirect; every rc
is a separate `echo $?`.

| leg | command | rc | Available before (KiB) |
| --- | --- | --- | --- |
| 1 | `cargo fmt --all --check` | 0 | 97,855,084 |
| 2 | `cargo clippy --workspace --all-targets --message-format=json --keep-going -- -D warnings` | 0 | 97,852,908 |
| 3 | `cargo check -p beamr-wasm --target wasm32-unknown-unknown --locked` | 0 | 97,145,908 |
| 4 | `wasm-bindgen-test-runner --version && … cargo test --manifest-path crates/beamr-wasm/Cargo.toml --target wasm32-unknown-unknown --locked` | 0 | 96,286,208 |
| 5 | `cargo test --workspace` | 0 | 96,288,132 |
| 6 | `ast-grep scan -r .ast-grep/rules/blocking-call-in-native-bif.yml crates/beamr/src/native/ --json` | 0 | 92,195,772 |

Leg 2's jq extract over `leg-2.log` yields **0** rows at `error` or `warning`.
Leg 6 emitted `[]`.

### Leg 5 axes, at an awk over `leg-5.log`

```
result-lines=72 passed=2064 failed=0 ignored=0
```

Expected exactly **72 / 2064 / 0 / 0** — matched on every axis.
**named-new-tests = THE EMPTY SET**, verified as a set difference, not asserted:
the 22 `distribution::sender::tests::*` names in `leg-5.log` are exactly the 22
`#[test]`/`#[tokio::test]` function names in the base file — nothing added,
nothing lost.

## Disk

`target/debug/incremental` was deleted inside the worktree after the battery
(standing rule), then `du-final.txt` was taken. **`du-final.txt` = 4,494,584 KiB**
for `/Users/tom/Developer/ablative/stack/beamr-sender`.

Datum, disclosed because it echoes the banked "under-return" phenomenon at a NEW
kind of site: whole-tree `du -sk` before the delete was **5,625,016** KiB and
`du -sk target/debug/incremental` was **1,751,012** KiB, yet the delete returned
only **1,130,432** KiB — an **under-return of 620,580 KiB**. This is an in-tree
`rm -rf` of a build subdirectory, not a worktree removal, and both measurements
are `du` (no `df` involved), so it is not the same measurement pair as the banked
points. Reported as an observation, not a conclusion — no mechanism is claimed.

## File index

- `BLOCKS.txt` — the 12 block-move declarations, `block NN: base sender.rs:A-B -> <file>:C-D <transform>`.
- `block-01..12.diff` / `.rc` — per-block byte-intactness; all rc 0.
- `EDITS.txt` — every enumerated edit line: `category ⇥ side ⇥ file ⇥ line ⇥ text ⇥ note`.
- `census.sh` / `census.log` / `census.err` / `census.rc` — the mechanical census; rc 0, remainder EMPTY.
- `quote-parity-pre.txt` / `.rc`, `quote-parity-post.txt` / `.rc` — the ruled parity condition.
- `THRESHOLD.txt` — the 43 GiB self-certifying disk floor, verbatim from the dispatching sanction.
- `leg-1..6.log` / `.err` / `.rc` / `.df` — the battery.
- `du-final.txt` / `.rc` — worktree size after the incremental delete.
