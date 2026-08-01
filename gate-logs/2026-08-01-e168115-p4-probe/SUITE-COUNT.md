# Registered denominator: 2052 workspace tests, UNCHANGED

The lane registered "zero suite tests added by either probe; expectation for any
following battery UNCHANGED at 2052". Established two ways, neither of which
required a cargo-class run (see the disk note at the end).

## 1. The number at this tree, from the closest committed battery

The nearest ancestor battery is `gate-logs/2026-08-01-cfbe0e0-60`, whose
`TREE` is `cfbe0e010050b726d1e2e6268ec6048291ecc97a`.

    $ git merge-base --is-ancestor cfbe0e0 e168115 ; echo $?
    0
    $ git rev-list --count cfbe0e0..e168115
    2

Counting its `cargo test --workspace` leg — summing per-binary results rather
than counting a substring, since "test result:" appears once per test binary
and a passed-count is not an occurrence count:

    $ grep -c '^test result:' gate-logs/2026-08-01-cfbe0e0-60/tests.log
    72
    $ grep '^test result:' gate-logs/2026-08-01-cfbe0e0-60/tests.log | grep -v '^test result: ok\.'
    (no output — every one of the 72 binaries reported ok)
    $ grep '^test result:' gate-logs/2026-08-01-cfbe0e0-60/tests.log \
        | sed -E 's/^test result: [a-zA-Z]+\. ([0-9]+) passed.*/\1/' | paste -sd+ - | bc
    2052
    $ grep '^test result:' gate-logs/2026-08-01-cfbe0e0-60/tests.log \
        | sed -E 's/.* ([0-9]+) ignored.*/\1/' | paste -sd+ - | bc
    0
    $ cat gate-logs/2026-08-01-cfbe0e0-60/tests.exit
    0

72 binaries, 2052 passed, 0 ignored, rc 0.

The two commits from there to `e168115` touch NO Rust source:

    $ git --no-pager diff --stat cfbe0e0 e168115
    ... README.md, docs/design/beamr/RETIREMENT-LIST.md, two briefs/*.json,
        and gate-logs/2026-08-01-cfbe0e0-60/* only — zero .rs files.

So the count at `e168115` is 2052 by construction.

## 2. This branch adds nothing the workspace suite can see

The probe crate `probes/lane-62/` declares its own `[workspace]` table, so it is
its own workspace root and is NOT a member of the beamr workspace. The root
`Cargo.toml` `members` list is untouched:

    members = ["crates/beamr", "crates/beamr-cli", "crates/beamr-wasm", "crates/gleam-types"]

`cargo test --workspace` therefore neither builds nor counts it. The crate also
contains no `#[test]` at all, and nothing was added under any existing crate's
`src/` or `tests/`. The commit's file list is the two `gate-logs/` artifact
directories plus `probes/lane-62/` — no production file was modified
(`p4-production.patch` is 0 bytes).

## Disk note — why no live re-run

The bar for this lane is 40 GiB (41,943,040 KiB) free on
`/System/Volumes/Data`. The reading at dispatch was 45,108,284 KiB (~43.0 GiB),
i.e. ~3.0 GiB of headroom. This worktree has no `target/`, so a
`cargo test --workspace` here is a COLD full build; the equivalent warm target
directory in the main checkout at the same commit measures 11 GB. Starting that
run would have driven free space to roughly 32 GiB, well under the bar. Per the
disk-courtesy rule the run was not started, and the count was established from
the committed evidence above instead. See `disk-boundary.log`.
