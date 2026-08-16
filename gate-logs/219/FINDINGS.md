# #219 feature-gate sweep — findings at d4a82e5 (2026-08-16/17)

## HEADLINE — the no-std waiver is DECAYING, and the mechanism that hid it

**1019 → 1039 errors (+20)** on `cargo check -p beamr --no-default-features`
between `ec5d7f8` (2026-07-07, the recorded baseline) and `d4a82e5` (HEAD).

| pin | rustc's own tally | E0425 | E0433 | E0432 | uncoded |
|---|---|---|---|---|---|
| ec5d7f8 | **1019** | 408 | 403 | 29 | 179 |
| d4a82e5 | **1039** | 420 | 412 | 29 | 178 |

Arithmetic closes exactly: coded +21, uncoded −1 ⇒ **+20**.

### The record and my instrument are RECONCILED, not in conflict

`CHANGELOG.md:1524` and `docs/DIST-CONTROL-WIRE-SPEC.md:941` record **1020**,
naming the identical command. rustc says 1019 at that same pin. The doc counted
`error: could not compile …` as an error line — **the exact off-by-one my own
first count made** (I read 1040 where rustc said 1039). Both instruments behave
the same way; neither was wrong about the world. State this whenever the two
numbers are shown together, or the next reader will think one seat miscounted.

⚠️ THE ORIGINAL AUTHOR IS NOT CONTRADICTED. "Byte-count-identical from baseline
ec5d7f8 through this release" was TRUE for the 0.13.0 series. What lapsed is the
period AFTER — five minor versions with nothing holding the invariant.

### Comparability was checked, not assumed
Same error composition at both pins (E0433/E0425-dominated), no edition or
toolchain failure at the old commit. A count drift with a DIFFERENT error mix
would have been a toolchain artifact, not drift.

## WHY IT WENT UNSEEN — two independent causes, both structural

### F1 · `--no-default-features` is INERT under `cargo test`
`crates/beamr/Cargo.toml:49`
```toml
[dev-dependencies]
beamr = { path = ".", features = ["test-support"] }
```
A self-dependency with no `default-features = false`. The flag applies to the
package named on the command line; this is a SEPARATE graph edge carrying
`default = true`, and cargo unifies features across edges ⇒ the defaults return.

Confirmed at TWO instruments:
1. the compile log — `cargo test -p beamr --no-default-features --features
   cooperative,json` compiled cranelift/tokio/zstd/num_cpus, all excluded;
2. two-arm `cargo tree` with a PRE-REGISTERED prediction —
   `-e normal` ⇒ crossbeam-queue + crossbeam-utils ONLY;
   `-e normal,dev` ⇒ cranelift-*, tokio, mio, num_cpus, crossbeam-channel/deque, zstd.

⇒ **No `cargo test` invocation can build beamr with defaults off.** The config
that would expose the debt is unreachable from the test harness by construction.
⚠️ PRECISION: opentelemetry is NOT from this edge — it is its own declared
dev-dep. The self-edge accounts for the other six.

### F2 · the waiver lives in PROSE, not in a gate
No leg in `gates.json` runs `cargo check -p beamr --no-default-features`. The
waiver was recorded in a CHANGELOG and a spec addendum. **A waiver without a
ratchet is a number that only ever grows**, and nothing was watching.

⭐ THE CLASS: A WAIVED GATE WITH NO RATCHET, PLUS A HARNESS THAT CANNOT SELECT
THE CONFIG, IS DEBT THAT GROWS INVISIBLY. Neither half alone would have hidden
it — the prose waiver was honest, and the harness gap was invisible. It is the
SEAM between them that let 20 errors accumulate unremarked.

## OTHER FINDINGS

### F3 · legs 5 and 8 build `cooperative` AND `threads` together
`beamr-wasm` is a workspace member (root `Cargo.toml:4`) depending on beamr with
`default-features = false, features = ["cooperative","json"]`. Under `--workspace`
those unify INTO the host build.
`cargo tree --workspace --features beamr/encode -e features` lists cooperative,
default, embedded, fs, jit, json, net, readiness, std, test-support, threads.
CONTROL: `-p beamr --features encode` gives the same list MINUS cooperative ⇒ the
workspace edge causes it, not the query.
⇒ canon's host suite runs a hybrid nobody ships. My earlier ground-pack table
calling config A "default + encode" was WRONG; this supersedes it.
⚠️ INSTRUMENT LIMIT: `-e features` shows only EDGE-INDUCED features, so `encode`
is absent from both lists. Expected, not evidence.

### F4 · `--features std` alone is also red — but small
rc=101, 21 errors, E0432 `unresolved import crate::timer` / `crate::replay`.
Module gating, not std-vs-no_std. Tractable; a plausible first rung.

### F5 · the browser feature set IS target-portable (clean result)
`cargo check -p beamr --no-default-features --features cooperative,json` ⇒ **rc=0**.
Also retroactively confirms the earlier "cooperative doesn't compile" red was
PURE CONTAMINATION from a concurrent clean, as attributed.

### F6 · a docs claim that is false at the bytes
`docs/stack-review/beamr-architecture.md:12` — "**`beamr`** — the VM (~100K
lines). `no_std`-capable." It has not compiled `no_std` since at least
2026-07-07. Existing brief `docs/design/beamr/briefs/B-144.json` ("no_std
compatibility audit") is the natural home for the evidence.

## APPARATUS DISCIPLINE THIS ROUND

- Positive control C0 (default `cargo check`) GREEN — without it the three reds
  would be uninterpretable.
- Predictions PRE-REGISTERED in `check-219.sh` before any arm ran. C1 held
  (predicted RED); **C2 was predicted UNKNOWN and came back GREEN** — recorded
  as-predicted-unknown, not retro-fitted into a success.
- Contamination sampled at BOTH ends of every run, per Waffles' amended protocol.
  Disclosed: aion's clippy/check appeared mid-window, rooted in
  `stack/aion/.worktrees/recovery-ordering` — a separate target dir. The failure
  signature here is deterministic `cannot find module std`, categorically unlike
  the earlier shrapnel (`.rmeta does not exist`, `could not execute process`).
- Baseline worktree used a self-imposed 20 GiB disk guard (floor is 15, another
  seat's build was live) and was torn down immediately; free 28 GiB unchanged.

## ROUTED, NOT ACTED ON — a finding does not authorize its own follow-up

(a) `default-features = false` on the dev self-edge — one word; makes the config
    selectable. Safe for canon: `--workspace` runs still get defaults via
    beamr-cli's edge, so only explicit `--no-default-features` runs change.
(b) **The real fix for the class** — a ratchet leg asserting the no-std error
    count does not exceed N, so a waived gate stops decaying silently.
(c) Correct `beamr-architecture.md:12`'s "no_std-capable".
(d) Feed all of it to B-144.

⛔ PHASE 4 (THE SEAL) STILL PREEMPTS the moment Cally answers §8.2/§8.4.
