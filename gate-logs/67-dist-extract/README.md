# Lane #67 DIST-EXTRACT — evidence

Base `227267e1b9d69ab78a3bd4dee935497f5e2421dd`, branch `artemis/dist-extract`,
worktree `/Users/tom/Developer/ablative/stack/beamr-67`.
Split commit `f86ffe3`. Authority: sha-frozen brief `d8de285e…` on Cally Ray's
named GO `4abd2919`. Nothing pushed, nothing tagged.

`crates/beamr/src/distribution/connection.rs` (3,449 lines) became the child
module directory `connection/`. Every external path and every behavior is
preserved.

## Declared-edit-set form

This is a MOVE WITH A DECLARED EDIT SET, not a refactor. Every byte of the
result is either

- **(a)** part of a byte-intact block move, or
- **(b)** a member of the enumerated edit set in `EDITS.txt`.

`census.sh` is the counter that proves there is no third category. It is not a
prose claim: its own exit code is the verdict, recorded in `census.rc`.

### The block manifest — `BLOCKS.txt`

82 blocks, columns `id file OLDa OLDb NEWa NEWb transform`. 3,329 of the base
file's 3,449 lines travel inside a block. Each block is diffed old-vs-new by
`census.sh` into `block-NN.diff` with its own `block-NN.rc`; **82/82 are rc 0**.

`transform` is `verbatim` for every block except the `tests.rs` blocks, which
carry `dedent4`.

### `dedent4` — the one mechanical transform, declared

Lifting the body of `#[cfg(test)] mod tests { … }` to the top level of
`connection/tests.rs` removes exactly four leading spaces from every non-empty
line. This was verified safe before it was applied: every non-empty line in base
`1892..3448` starts with at least four spaces (checked, 0 exceptions), and the
only multi-line string literals in the module use `\`-continuations, which eat
the newline and the following indentation, so no string's content depends on
those four columns. `census.sh` applies the same `sed 's/^    //'` when it
verifies a `dedent4` block, so the byte-intactness claim is checked at the
dedented bytes, not asserted.

### `E-f` — a category the brief did not name, disclosed

The brief's edit set is `E-a` / `E-b` / `E-c`, and `(E-d) NOTHING ELSE`. The
landed tree contains one more category, **`E-f`: 6 landed lines across 4 sites
in `tests.rs`, replacing 8 base lines.**

Ground: the four-column dedent frees four columns of width, and four expressions
that rustfmt had wrapped at the deeper indentation now fit on one line, so
rustfmt re-joins them. Leg 1 is `cargo fmt --all --check` and is mandatory, so
there is no version of this move that both dedents the test body and leaves
those four sites untouched. **Zero tokens change** at all four sites — they are
`let manager = …`, `tokio::time::timeout(…).await`, and two
`HandshakeNode::with_default_flags(…).expect(…)` calls, re-wrapped and nothing
else. Every one of the 14 lines involved is enumerated in `EDITS.txt` with its
old and new text, and the census counts them like any other edit. Reported, not
absorbed.

### `EDITS.txt` — 325 enumerated lines

Tab-separated: `category`, `side`, `file`, `line`, `text`.
`side OLD` = a base `connection.rs` line consumed by an edit;
`side NEW` = a landed line not produced by any block move.

| category | NEW | OLD | what |
|---|---|---|---|
| `E-a` | 145 | 77 | child `mod` declarations, re-exports, per-file `use` blocks, the Design Sentence, structural blank separators, and the `mod tests { … }` wrapper the child-file declaration replaces |
| `E-b` | 27 | 27 | `pub(super)` visibility widenings — 1:1, the token is the only difference |
| `E-c` | 15 | 8 | deictic re-pointings and intra-doc-link re-pathing (includes the 5 `etf.rs` lines and the 1 permitted `sender.rs` line) |
| `E-f` | 6 | 8 | the rustfmt re-wrap described above |

The 27 `E-b` widenings, in full:

- `link.rs` — `DistConnection` fields `node`, `down`, `shutdown`, `direction`,
  `down_reason`; methods `new`, `note_inbound_activity`, `inbound_idle_for`,
  `mark_down_heartbeat_timeout`, `mark_down`; `struct PreparedSocket` and
  `PreparedSocket::prepare`; `AcceptHandle` fields `local_addr`, `shutdown`,
  `task`. (16)
- `residency.rs` — `INBOUND_RESIDENCY_ENVELOPE_BYTES`,
  `INBOUND_RESIDENCY_PER_PEER_BYTES`, `struct InboundResidency` with fields
  `charged` and `refused`, `InboundResidency::new`,
  `InboundResidency::try_admit`, `struct InboundAdmissionPermit`. (8)
- `frame.rs` — `KEEPALIVE_FRAME`, `frame_buffer_for_header`. (2)
- `lifecycle.rs` — `spawn_read_lifecycle`, `accept_loop`. (2 — `spawn_heartbeat`
  and `handle_accepted` are called only from inside `lifecycle.rs` and stay
  private.)

Q3 is honoured: these are FIELDS, not reader functions. Nothing gained a `pub`
it did not have, and nothing crossed the crate boundary — `pub(super)` reaches
`connection` and its descendants and stops there.

### Positional words — checked, none falsified

The nine `above`/`below` sites the brief pinned (base `:552`, `:1348`, `:1410`,
`:1440`, `:1457`, `:1502`, `:1558`, `:1650`, `:1660`) were each read against
their new file. All nine are intra-item deixis — they point at code inside the
same function or the same doc-plus-body pair, and that relationship is unchanged
by the move. **None was falsified, so none was touched.** They appear in no edit
category because there is no edit to enumerate.

## NO RED LEG — ruled out loud

There is no red leg in this battery, and that is a ruling, not an omission.

A split has no behavior to demonstrate failing. A red leg earns its place when
it shows that the instrument can see the defect the change removes — you break
the thing, the gate goes red, you fix it, the gate goes green, and the green now
means something. Here nothing is removed and nothing is added: the production
bytes at the end are the production bytes at the start, redistributed across
files. Any "red" would have to be manufactured — comment out a re-export, watch
the compiler complain — which demonstrates that `rustc` resolves paths, not that
this move is correct. That is theatre. The instrument that actually carries the
claim is `census.sh`, and it is a counter with its own rc.

## Reconciliation-carry — no baseline run

No baseline was run; the carry is ruled.

**EXPECTED-PRIOR, certified at `227267e`, axes named:**

| axis | expected |
|---|---|
| passed | 2064 |
| failed | 0 |
| result-lines | 72 |
| ignored | 0 |

**named-new = THE EMPTY SET**, declared. This lane adds no test.

**Leg 5 as landed: 2064 passed / 0 failed / 72 result-lines / 0 ignored.** All
four axes match. Nothing is named against main.

## Battery — six legs

Commands verbatim from the dispatching brief, run from the worktree root after
the split compiled clean. Each leg: `df -k /System/Volumes/Data` captured to
`leg-N.df` BEFORE the leg; stdout redirected to `leg-N.log`, stderr to
`leg-N.err`; rc via `echo $?` into `leg-N.rc`. No tee, no `2>/dev/null`, no
`|| true`, no `-q`.

| leg | command | rc |
|---|---|---|
| 1 | `cargo fmt --all --check` | `0` |
| 2 | `cargo clippy --workspace --all-targets --message-format=json --keep-going -- -D warnings` | `0` |
| 3 | `cargo check -p beamr-wasm --target wasm32-unknown-unknown --locked` | `0` |
| 4 | `wasm-bindgen-test-runner --version && CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test --manifest-path crates/beamr-wasm/Cargo.toml --target wasm32-unknown-unknown --locked` | `0` |
| 5 | `cargo test --workspace` | `0` |
| 6 | `ast-grep scan -r .ast-grep/rules/blocking-call-in-native-bif.yml crates/beamr/src/native/ --json` | `0` |

Every rc in this table is quoted from its `leg-N.rc` artifact; no instrument was
re-run to fill the table in. Leg 6 was never piped; its log is `[]` — exit 0 with
no findings.

Boundary discipline: threshold and basis in `THRESHOLD.txt`. Available never
approached the 44,564,480 KiB floor — the lowest reading across the six legs was
92,085,504 KiB at leg 6, better than 2× the threshold. No leg was withheld.

`du-final.txt` (rc in `du-final.rc`): the worktree is 6,181,420 KiB after the
battery.

## Bar

- **Zero new `#[allow]`.** Base `connection.rs` had 0; the landed `connection/`
  tree has 0 across all seven files. Counted, not assumed.
- **No new `unwrap`/`expect`/`panic!`**: 121 in the base file, 121 across the
  landed tree — the same call sites, moved.
- **No `_ =>` arms**: 0 before, 0 after.
- Every `git diff` in this evidence used `--no-ext-diff` (the repo sets
  `diff.external=difft`, which would otherwise corrupt raw diffs).

## Layout

```
README.md            this file
BLOCKS.txt           82-block manifest, OLD range -> NEW file:range, transform
EDITS.txt            325 enumerated edit lines, category/side/file/line/text
census.sh            the counter
census.log           its run
census.err           empty
census.rc            0  <- remainder EMPTY
block-01..82.diff    per-block old-vs-new diff
block-01..82.rc      per-block rc (all 0)
THRESHOLD.txt        boundary-disk threshold, noun, instrument, basis
leg-1..6.{log,err,rc,df}
du-final.txt / .rc
```
