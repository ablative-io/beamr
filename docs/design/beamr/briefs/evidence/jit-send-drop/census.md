# beamr `jit_send_message` self-send-only defect — published-version census

Measured 2026-08-12 from the **packaged bytes of every published .crate** (downloaded from
static.crates.io; version list from the crates.io registry API — NOT from git tags).
Instrument: `census-crates/measure.py` over `census-crates/extracted/beamr-<v>/`;
raw per-row data in `census-crates/census-rows.json`. All 58 downloads HTTP 200, all 58
extractions rc 0, all 58 rows measured. **Zero FAILED-TO-MEASURE rows.**

Defect judged from packaged source: `pub(crate) extern "C" fn jit_send_message` whose only
mailbox push is guarded by `PidRef::Local(pid)` + `pid == process.pid()`, with no other
delivery path and no error/deopt on any other destination (non-self destinations silently
fall through and the raw message term is returned). Function-body identity was established
by comment/string-aware brace-counted extraction and SHA-256 hashing; exactly ONE body
variant exists across all carriers (`e350f274…`), independently confirmed by byte-level
`diff` of the packaged files (0.4.0 vs 0.10.0 vs 0.18.1: identical).

## Census table (all 58 published versions, oldest first)

| version | yanked | vcs sha | sha in repo | jit feature | jit in default | jit_send_message | defect pattern |
|---|---|---|---|---|---|---|---|
| 0.1.0 | no | e880b50ee7ef | yes | no | no | no (no src/jit/ dir) | absent |
| 0.2.0 | no | 0bb7f62cbcee | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.0 | no | 4de6b6c96a78 | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.1 | no | 529e82062090 | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.2 | no | bb219f9c2c39 | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.3 | no | 89b5fb4108d4 | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.4 | no | d885edf750e1 | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.5 | no | cd252fb2925b | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.6 | no | 6be3bb300210 | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.7 | no | a7522da18234 | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.8 | no | 161badc7ec70 | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.9 | no | c29a14a8a1cc | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.10 | no | 360fb2292154 | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.11 | no | 6bba51dbf585 | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.12 | no | 667aeade22b9 | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.13 | no | 9df853a0aaf9 | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.14 | no | 6a64dbabb42f | yes | no | no | no (no src/jit/ dir) | absent |
| 0.3.15 | no | bb8824822d46 | yes | no | no | no (no src/jit/ dir) | absent |
| 0.4.0 | no | 73bb378fde90 | yes | no (no feature — module compiled unconditionally) | no | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.4.1 | no | 1d561c96ffb6 | yes | no (no feature — module compiled unconditionally) | no | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.4.2 | no | f85ce94c5e1a | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.4.3 | no | 440010945685 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.4.4 | no | 563f3dbe7b88 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.4.5 | no | a3d2d2d9a04a | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.4.6 | no | 403a2ffe411b | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.4.7 | no | cfbef2030957 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.4.8 | no | 123f51b9625b | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.4.9 | no | 577d2ac3cbfc | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.5.0 | no | 1d708351a641 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.6.0 | no | 79ff72bdc671 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.6.1 | no | ffdeb96a38c7 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.6.2 | no | 258a42ec199e | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.6.3 | no | 65bd6cc05014 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.6.4 | no | 0d4623994669 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.7.0 | no | 4767cdfba92c | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.8.0 | no | 1d7340ad7362 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.8.1 | no | fb0f5d3d1a4e | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.8.2 | no | e37362fffb4f | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.9.0 | no | 30c41aefa700 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.10.0 | no | dff4dcfc1753 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.11.0 | no | 175f96c03d48 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.12.0 | no | 58987bb99872 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.13.0 | no | f6aa8e9b441e | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.13.1 | YES | 8a9eaf78dc7f | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.13.2 | YES | ba19256df8c0 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.14.0 | no | 1b07d034e03e | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.15.0 | no | 2cf30854357d | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.15.1 | no | d9de35e4e753 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.15.2 | no | d60f826bcf68 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.15.3 | no | 4086030a29e4 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.15.4 | no | c716992fdbe7 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.16.0 | no | 5ebf94dacc51 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.16.1 | no | 5291de4ba8c4 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.16.2 | no | 5206e7af16e8 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.16.3 | no | 9d0d0e0d9006 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.17.0 | no | 377b6de0d115 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.18.0 | no | 0460a7045afb | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |
| 0.18.1 | no | 2551841ba4c6 | yes | yes | yes | yes (src/jit/runtime_message.rs) | PRESENT |

## Affected-range statement

**40 of 58 published versions are affected: every published version >= 0.4.0, with no holes
in the published sequence.** Exact list (includes both yanked versions):

0.4.0, 0.4.1, 0.4.2, 0.4.3, 0.4.4, 0.4.5, 0.4.6, 0.4.7, 0.4.8, 0.4.9,
0.5.0, 0.6.0, 0.6.1, 0.6.2, 0.6.3, 0.6.4, 0.7.0, 0.8.0, 0.8.1, 0.8.2,
0.9.0, 0.10.0, 0.11.0, 0.12.0, 0.13.0, 0.13.1 (yanked), 0.13.2 (yanked), 0.14.0,
0.15.0, 0.15.1, 0.15.2, 0.15.3, 0.15.4, 0.16.0, 0.16.1, 0.16.2, 0.16.3,
0.17.0, 0.18.0, 0.18.1.

Unaffected: 0.1.0 through 0.3.15 (18 versions) — these ship **no `src/jit/` directory at
all** and no `jit` feature.

### Exposure per version (feature gating)

- **0.4.0, 0.4.1** — no `jit` cargo feature exists; `src/lib.rs` declares `pub mod jit;`
  with **no cfg gate**, and all five cranelift deps are **non-optional**. The defective code
  compiles **unconditionally** — it cannot be disabled by feature selection.
- **0.4.2 through 0.18.1 (all 38)** — `jit` feature exists, is gated
  (`#[cfg(feature = "jit")] pub mod jit;`), and **`jit` is in `default`** in every one of
  these versions. Default builds carry the defect; `default-features = false` builds do not
  compile the jit module.
- There is **no published version where `jit` exists but is not default.**

### Correction to the dispatching brief (both claims refuted at the packaged bytes)

1. **"First shipped v0.10.0, per git tag --contains" is wrong — first shipped is v0.4.0.**
   `git tag --contains 19e0f34` in fact lists v0.4.0 .. v0.18.1 (37 tags); its output is
   **lexically** sorted, so `v0.10.0` is merely the first LINE. Ancestry confirms:
   `git merge-base --is-ancestor 19e0f34 73bb378fd` (0.4.0's vcs sha) succeeds.
2. **No published version ever carried the fn in `src/jit/runtime.rs`.** The defect commit
   19e0f34 (2026-06-09, "feat(beamr-jit): lower message send and receive opcodes") added it
   to runtime.rs, and the B-167 split 68423a1 moved it to runtime_message.rs the **same
   day** — both predate the v0.4.0 release, so every packaged copy lives at
   `src/jit/runtime_message.rs`. The runtime.rs residency existed only in unpublished
   intermediate commits.

## Boundary evidence — quoted helper body from the packaged bytes

The body below is **byte-identical in all 40 affected versions** (single body hash
`e350f274…`; full-file diff of first/middle/last packaged copies returned identical), so it
serves simultaneously as the first-affected (0.4.0), middle (0.10.0), and last-affected
(0.18.1) quote:

```rust
pub(crate) extern "C" fn jit_send_message(
    process: *mut Process,
    dest_pid: u64,
    message: u64,
) -> u64 {
    let Some(process) = process_from_abi(process) else {
        return message;
    };
    let message_term = Term::from_raw(message);
    if let Some(PidRef::Local(pid)) = PidRef::new(Term::from_raw(dest_pid))
        && pid == process.pid()
    {
        process.mailbox_mut().push_owned(message_term);
        if process.status() == ProcessStatus::Waiting {
            let _ = process.transition_to(ProcessStatus::Running);
        }
    }
    message
}
```

The only `mailbox_mut().push_owned` sits inside the `PidRef::Local(pid)` +
`pid == process.pid()` guard; every other destination falls through silently and the fn
returns the raw `message` term — no error, no deopt. The fn is the live JIT send path: it
is registered as the `beamr_jit_send_message` symbol in `src/jit/compiler/dispatch.rs` and
invoked from `src/jit/compiler/ir_helpers.rs` in every sampled carrier (0.4.0, 0.10.0,
0.18.1).

Earliest-boundary check: 0.3.15 and every earlier version contain no `src/jit/` directory
(verified per-package, not inferred). Latest: 0.18.1 carries the defect.

## Cross-checks and notes

- **vcs sha vs local repo:** every one of the 58 `.cargo_vcs_info.json` shas resolves to a
  commit object in /Users/tom/Developer/ablative/stack/beamr (`git cat-file -t` = commit,
  58/58). Existence only — the packaged bytes remain the authority for every judgment above.
- **Registry vs tags skew (why tags were the wrong census):** tagged-but-never-published:
  v0.8.3, v0.12.1; published-but-untagged: 0.15.3, 0.15.4, 0.16.0, 0.16.1, 0.16.2
  (0.16.2's missing tag is the already-ruled task-#55 state, pinned by commit 67f89c4).
- **Failed rows: none.** 58/58 downloaded, extracted, and measured.

## Summary

58 published (2 yanked: 0.13.1, 0.13.2). **40 affected** — every published version from
**0.4.0** (first affected) through **0.18.1** (latest, still affected), contiguous over the
published sequence, one byte-identical defective body throughout. No version has `jit`
non-default; 0.4.0/0.4.1 are worse — defective code compiled unconditionally, no feature
gate. 18 unaffected (0.1.0–0.3.15, no jit code at all). Zero failed measurements.
