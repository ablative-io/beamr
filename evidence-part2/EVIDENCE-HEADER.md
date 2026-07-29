# Evidence header — beamr fix-wave part 2, RECORD/REPLAY REBUILD (task 3aecb622)

**MACHINE:** Dean's box — `/Users/deanwhiting/Developer/beamr-part2`
**OPERATOR:** Seth Crackers (sub-agent, beamr part 2)
**MEMBER_ID:** 5828bee2-6460-44a5-ba78-a0f82ce0f8f1
**LANE OWNER:** Artemis Peach (beamr owner) — she owns acceptance; nothing is landed here.
**REPO/BRANCH:** beamr @ `seth/replay-rebuild-3aecb622`, branched off **192e4a4** (never rebased).
**REMOTE:** `github.com/tomWhiting/beamr` — resolved on the first try, no 404, no org search needed.

Every line in this bundle names the machine and the operator above.

## DATABASE — STATED LOUDLY

**BEAMR NEEDS NO DATABASE.** `MERIDIAN_TEST_DATABASE_URL` is **NOT required and is NOT
assumed present**. No leg in this battery reads a database URL; beamr has no database
surface at all. A leg silently running against a wrong or absent database is the
silent-variant class the estate is closing — it cannot occur here, and this is recorded
rather than left implicit.

## LAUNCH ENV ACTUALLY USED

```
SEAT_NAME="Seth Crackers (sub-agent, beamr part 2)"
MEMBER_ID=5828bee2-6460-44a5-ba78-a0f82ce0f8f1
EVIDENCE_DIR=/Users/deanwhiting/Developer/beamr-part2/evidence-part2
```
`cd` to the worktree root before anything; invoked with **bash**, never zsh.

Note on this box: the agent tool's default shell **is zsh**, in which `PIPESTATUS` does not
exist (zsh spells it `pipestatus`, 1-indexed) and `--include=*.rs` globs fail. Every
compile/test invocation in this lane was therefore issued through an explicit `bash -c`.
This is exactly the failure canon r3's flag-5 refusal exists to prevent — the script dies
mid-run under zsh *after* the claim is taken.

## CANON IDENTITY

- stack-root entry **e269d2c9-0dfa-409e-ada5-b10303118225**
- sha256 **ff831516f8ff74de1f54023ff4af54d80cade6a285b6ff42f5a37dbfda6290e4** (11532 B, with-LF framing) — verified at the bytes on this box
- extracted runnable script sha256 **bf404f6a1d4d475442d078b185a993cd2d18c6d708589dd2e33d0ee29f259806** — verified

**KNOWN AND RULED, NOT RE-FLAGGED, NOT FIXED LOCALLY:** canon r3's line 2 self-describes as
"revision 2". The **label** is wrong; the **content** is r3; the bytes are **frozen** by
ruling. Canon identity is the entry-id + hash pair, never the artifact's self-description.

**Canon r3's bytes were NOT patched.** r4 is landing under another owner.

## DERIVATION PROVENANCE

Derived from the **byte-verified extracted canonical script** (`bf404f6a…`), re-verified at
derivation time. **Not** copied from any runner found lying in a scratchpad. Nothing was
read, resurrected, or executed from `QUARANTINE-do-not-execute/` (three superseded runners,
mode 000 — one carrying an unguarded `rm -f "$CLAIM"` under `trap … EXIT INT TERM` that
deletes the live foreign claim it was waiting on at acquisition timeout).

Runner: `evidence-part2/run-battery-beamr-seth.sh`
sha256 `d5f33d50a6eac5b499dc173f3c29778c516349c1a70a0e05ecc283afc86e2f05`

## THIS IS NOT A NULL DIFF FROM CANON r3

Per rider **77b2c212** / reads **a621f353**: canon r3 itself carries the defects, so a
null-diff claim would assert this runner **inherited** them. It does not. **When the
canonical artifact carries the defect, verbatim inheritance IS the debt.** Deltas are
disclosed below; the preamble is otherwise byte-verbatim from r3, with no tidying and no
opportunistic edits.

## DISCLOSED A5 DELTAS (citing a621f353)

| # | Rule | Delta | Where (post-delta lines) |
|---|------|-------|--------------------------|
| A5-1 | Write canon's dialect exactly (`key=value`) | **NONE** — r3 already complies; deliberately not "improved" | `write_claim` 252, `acquire` 316 |
| A5-2 | Read dialect-tolerantly (`key=value` **and** `key: value`) | **ADDED** `claim_field()`; replaces all three of r3's blind `sed -n 's/^pid=//p'` sites | 177; used at 182, 214, 228, 232, 265–266, 303–304, 349 |
| A5-3 | Unparseable/empty pid is **never** grounds for a rule-5 clear | **ADDED** — pid must be non-empty **and** all-digits before any liveness result is believed; otherwise reads **HELD** | 352 (`CANNOT DETERMINE HOLDER PID`), stale path at 356 |
| A5-4 | No phase flip without an ownership check | **ADDED** — **pre-flip** refusal + post-flip re-verification | `write_claim` 252, refusal 290, post-flip 307 |
| A5-5 | Release-time voiding on **foreign or absent** | **ADDED** — absent branch checked explicitly, scoped by `CLAIM_ACQUIRED` | `release_claim` 209, FOREIGN 228, ABSENT 236 |

### Re-pointed behavioural conformance cites (line numbers MOVED under the guards)

| Behaviour | Line(s) |
|---|---|
| Acquire **before** drain-wait | acquire loop `PHASE 1` **341**, `CLAIM_ACQUIRED=1` **370**, drain `PHASE 2` **373** |
| Release on **every** exit path | `trap release_claim EXIT INT TERM HUP` **250** (flag 3: HUP included) |
| Phase flip guarded | pre-flip refusal **290**, post-flip re-verify **307** |
| Quiet-floor proof is the **census**, not the claim | **388** |
| Legs, exits captured via `PIPESTATUS` | fmt **398**, clippy **405**, wasm32-check **412**, wasm-tests **421**, tests **428** |
| Coverage statements | nextest N/A **432**, doc-tests COVERED **435** |

## PRE-EMPTION — EXCEEDS THE RULED FLOOR, AND WHY THAT MATTERS

The ruling ("a wrapper around frozen canon cannot refuse before canon's internal `mv`, only
detect immediately after") describes a wrapper invoking canon as a **black box**. This is a
**derivation**, so `write_claim` is our own code: it **refuses before the mv**, leaving a
foreign claim intact, and re-verifies after.

**FINDING WORTH CARRYING TO WHOEVER LANDS r4: a post-flip-only check is VACUOUS, not merely
limited.** `write_claim` writes our own content, so reading the claim back after the mv
confirms our own write and passes **even when a live foreign claim was just clobbered**.
The first cut of this guard did exactly that. The proof harness caught it: `write_claim`
exit **0**, `flip_determination=OURS_CONFIRMED`, foreign claim silently overwritten. It was
corrected to check pre-flip. A post-flip-only guard should be treated as **insufficient**.

## WRAPPED-NESS IS VERIFIABLE FROM THIS BUNDLE, NOT ASSUMED

The guards bind only runs that go through this runner, so an unwrapped launch would yield a
green indistinguishable from a guarded one. This bundle carries records an unwrapped run
**could not** produce:

- this header + the runner's disclosure header, citing **a621f353**;
- `dialect-preflight.txt` — the dialect-tolerant reader exercised on **both** dialects
  before the claim is touched, showing canon's `sed` returning **empty** on the colon form;
- `flip-determination.txt` — the flip-time ownership determination (pre- and post-flip);
- `release-determination.txt` — the release-time foreign-or-absent determination, written
  on **every** exit path;
- `guard-proof.txt` + `guard-proof-harness.sh` — all five guards exercised against the
  runner's **real bytes** (functions extracted from the committed file), with the claim path
  redirected to a temp file so `/tmp/ablative-gate-battery.claim` is never touched.

### Guard proof result (see `guard-proof.txt`)

| Guard | Result |
|---|---|
| A5-2 dialect-tolerant read | colon form → wrapper `515151`, **canon sed `''`** — defect shown and closed |
| A5-3 unparseable pid | `''`, `not-a-pid`, `12x4` → **HELD**, no rule-5 clear; `424242` → proceeds |
| A5-4 flip with foreign claim | **exit 6**, `REFUSED_NOT_OURS`, VOID marker written, foreign claim intact |
| A5-5 absent at release after acquisition | `ABSENT_VOID`, `detector=release-time-absent` |
| control: absent **before** acquisition | `NOT_ACQUIRED`, **no** false void |

## THREE DETECTORS — ANY FIRING VOIDS THE RUN AS EVIDENCE

1. pre/post-flip ownership check fails → our claim was replaced mid-run;
2. release-time **foreign** claim → we were the **victim** of a clobber, not the perpetrator;
3. release-time **absent** claim after acquisition → a thief flipped over our claim and
   released its own first (canon exits clean here — that is the hole a621f353 closes).

Any firing ⇒ quiet-floor premise **VOID** ⇒ the run is **VOID AS EVIDENCE**, not a green.
Reported, never silently re-run over the top of. A `VOID.marker` is written.

## QUIET-FLOOR BASIS

Cited **alongside** the census, never instead of it: every battery on this box tonight is
**sequenced serially by the coordinator** — one authorized battery at a time, no concurrent
claim holders — and all live lanes here run the same r3 derivation, so they speak one
dialect. The acquisition race is therefore **structurally absent at this venue** rather than
merely unobserved. `census-at-start.txt` / `census-at-end.txt` remain the actual proof.

## BOX CLAIM CONVENTION v2 CONFORMANCE (pre-battery phase)

This lane is an **ORDINARY COMPILE LANE** for build/test iteration and **creates no claim**.
`/tmp/ablative-gate-battery.claim` was checked before **every** compile invocation:

| Checked at (UTC) | Result |
|---|---|
| 2026-07-29T18:02:26Z | no claim file |
| 2026-07-29T18:08:48Z | no claim file — proceeded |
| before fmt/clippy verification run | no claim file — proceeded |

No claim was ever taken, no dead-pid clear was ever performed, and no claim file was
removed by this lane.

## THE FIVE LEGS (beamr's, from `gates.json`, verbatim and in order)

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo check -p beamr-wasm --target wasm32-unknown-unknown --locked`
4. `wasm-bindgen-test-runner --version && CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test --manifest-path crates/beamr-wasm/Cargo.toml --target wasm32-unknown-unknown --locked`
5. `cargo test --workspace`

**NEXTEST: DECLARED N/A.** beamr has **no** nextest stage. Canon r3's nextest leg and its A6
Summary-line extraction do not apply here. Declared, never silently dropped.

**DOC TESTS: COVERED by leg 5.** `cargo test --workspace` runs doc tests for library
targets. Canon r3's A7 line reads "DOC TESTS: NOT COVERED" — written for a runner whose only
test leg was nextest, which does not run doc tests. **That line is deliberately NOT copied,
because for beamr it would be false.** Loud and true beats loud and copied; the runner
extracts the `Doc-tests` sections from leg 5's log as proof, and shouts if none match.

Per leg: stderr kept in committed logs, exit captured via `PIPESTATUS`, nothing masked, no
`|| true`, load disclosed per leg via `uptime`.
