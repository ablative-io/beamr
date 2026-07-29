# Runbook — beamr 0.17.0

Staged 2026-07-29 by Artemis Peach (beamr owner seat) at main `9294be0`.
Executed by whoever holds the credential-bearing box; **not by this seat, which
holds no token and publishes nothing.**

This runbook is version-specific and composes with `RELEASE_CHECKLIST.md`, which
is deliberately version-agnostic and still applies in full. Everything here is
what that checklist cannot know: *why this release is 0.17.0, what is unusual
about this tree, and what will bite the operator.*

Every fact below was measured at `9294be0` and carries the command that
re-establishes it. Re-run them; do not carry them on this document's word. Where
something is **not** verified it says so.

---

## 1. The version is forced. It cannot be 0.16.4.

Two independent reasons, either one sufficient:

- **A breaking change is already on main.** `1ab619a refactor(scheduler)!:
  remove spawn_link_dirty — the 0.17.0 breaking window opens`. Under 0.x, a
  breaking change consumes the minor.
  ```sh
  git log --oneline 67f89c4..main -- crates/ | grep '!:'

  # The identifier is gone from shipped source. Two traps are already
  # defused in this command, both of which produce a confident wrong answer:
  #   * `fn spawn_link_dirty` MATCHES — it prefix-matches a surviving test
  #     name, so that form reports the definition as still present.
  #   * pathspec `crates/*/src` and `crates/*/src/` silently match NOTHING;
  #     `*` does not cross `/` without `:(glob)`. Exit 0, no warning.
  # Run the control first — if it prints nothing, the instrument is blind
  # and the test below is meaningless.
  git grep -c 'watch_exit'       -- ':(glob)crates/*/src/**'   # CONTROL: 3 files
  git grep -n 'spawn_link_dirty' -- ':(glob)crates/*/src/**'   # expect: no hits
  ```
- **An additive public API is on main.** `watch_exit` (`6a3ceec`) — additive
  public surface is itself a minor bump under 0.x convention as practised here.

**If a 0.16.4 is ever wanted, it cannot be cut from main.** It must be cut from
the 0.16.2 line. The mechanical test, which also lives in `CHANGELOG.md`:

```sh
git merge-base --is-ancestor 67f89c4 <base>   # must be true, or the cut must
                                              # carry C1+C2 explicitly
```

## 2. The trap in this tree: `v0.16.3` is NOT an ancestor of main

```sh
git merge-base --is-ancestor v0.16.3^{commit} main   # FALSE
```

This looks alarming and is not. 0.16.3's fixes reached main by **forward-port**
(the `fwdport lane-1..4` commits), so the content is present under different
SHAs. Verified by content, not ancestry — all ten source files the 0.16.3
release touched are byte-identical between the tag and main:

```sh
BASE=$(git merge-base main v0.16.3)
for f in $(git diff --name-only "$BASE" v0.16.3); do
  git cat-file -e "v0.16.3:$f" 2>/dev/null && git cat-file -e "main:$f" 2>/dev/null \
    && [ "$(git rev-parse "v0.16.3:$f")" = "$(git rev-parse "main:$f")" ] \
    && echo "IDENTICAL $f" || echo "DIFFERS/ABSENT $f"
done
```

At `9294be0` this classifies 38 of 38 examined files: **16 identical, 21
differing, 1 absent — and every differing or absent entry is CHANGELOG,
Cargo.lock, RELEASE_CHECKLIST, the version line, gate logs, or evidence
transcripts. Zero source files differ.** `rust-toolchain.toml` is identical, so
the estate-wide 1.97.1 pin is in force here.

**Therefore 0.17.0 cut from main does not regress the shipped 0.16.3.** That is
the question this section exists to answer; answer it again before cutting, and
do not accept "the tag isn't an ancestor" as either a blocker or a shrug.

One known divergence, harmless and stated so nobody rediscovers it as a defect:
`…/lane-3-jit/runs/red-d2-stale-position-write.txt` exists at the tag and not on
main. It is an evidence transcript, not code.

## 3. Version edits, with the pin trap called out

Current manifest state (`grep -m1 '^version' crates/*/Cargo.toml`) against the
registry (probed with a User-Agent — see §6):

| crate | manifest | published | commits since its version was set | action |
|---|---|---|---|---|
| `gleam-types` | 0.4.3 | 0.4.3 | **0** | **no bump** — unchanged; the script will skip it |
| `beamr` | 0.16.2 | 0.16.3 | — | **→ 0.17.0** |
| `beamr-cli` | 0.4.0 | 0.4.0 | 7 | **→ 0.5.0** |
| `beamr-wasm` | 0.7.0 | 0.7.0 | 10 | **→ 0.8.0** |

`beamr`'s manifest reading 0.16.2 while 0.16.3 is published **is deliberate**
(the ruling is in `67f89c4`'s commit body). It is not the defect it looks like.

**⚠ THE PIN TRAP — this is the one that will silently produce a wrong release.**
Two crates pin beamr by caret range:

```
crates/beamr-cli/Cargo.toml:16   beamr = { version = "0.16.0", path = "../beamr" }
crates/beamr-wasm/Cargo.toml:18  beamr = { version = "0.16.0", path = "../beamr", … }
```

`"0.16.0"` means `^0.16.0`, which **does not match 0.17.0**. Locally the `path`
key wins so everything builds and tests green; the published crates would carry
a version requirement no longer satisfiable. **Both pins must be edited to
`"0.17.0"` in the same commit as the bump.** A green local battery cannot detect
this — only `cargo publish` resolution can, which is what the dry-run is for.

Also confirm `beamr -> gleam-types` still declares its `version = "0.4.3"`
fallback (`crates/beamr/Cargo.toml:27`).

## 4. Take a claim before the first upload

Ruled 2026-07-29 after three parties published to crates.io concurrently
without knowing it, absorbed only because the registry refuses duplicates:

> **Any publish sequence takes a claim on the credential-holding box for its
> duration, and states which crates it intends, before the first upload.**

The registry is a shared resource with no claim discipline of its own. Every
box, battery and gate leg in this estate has a claim file; the one unrecoverable
act had nothing. `scripts/release.sh` now prints its full intended set before
the first upload (`9294be0`) — that is the *announcement*, not the claim. The
claim is taken on the box, by the operator, in the normal way.

## 5. Gates — and there is no CI behind you

**GitHub Actions is disabled org-wide (failed payment).** There is no CI green
for this release and none is coming. The `gates.json` battery at the release
commit is the *only* evidence, which makes `RELEASE_CHECKLIST.md` §"Validation
gates" load-bearing rather than ceremonial.

- Run the full 5-leg battery at the release commit and land its evidence commit
  bound to the tree hash.
- **No battery has been run at `9294be0`.** This runbook stages the release; it
  does not certify the tree. Certification is the executor's, at the commit
  actually being cut.
- One thing CI uniquely covered that no battery does:
  `cargo check -p beamr --no-default-features --features cooperative,json`.
  Run it explicitly.

## 6. The publish itself

Use `scripts/release.sh`. It defaults to `--dry-run`; only `--publish` arms it.

```sh
scripts/release.sh              # dry-run: packages and verifies, uploads nothing
scripts/release.sh --publish    # REAL, IRREVERSIBLE
```

Publish order is dependency order and is hardcoded at `scripts/release.sh:49`:
`gleam-types beamr beamr-cli beamr-wasm`.

Two things the script now does that it did not do before `9294be0`, both worth
knowing because they change how it fails:

- It sends a **User-Agent**. crates.io answers `403` to a request without one —
  for every crate, published or not — so the old idempotence check read every
  published crate as unpublished, and the index wait would have spun forever
  *after* a successful upload.
- It reports **three** states, not two. On any answer that is neither 200 nor
  404 it **refuses to publish** rather than guessing, and if that happens while
  waiting for the index it stops and says *the publish may already have
  succeeded*. If you see that message, **verify on crates.io by hand before
  re-running — do not assume it failed.**

Verify by registry probe, not by exit code. Tonight's liminal release had cargo
exit 101 on a crate that was live: the exit code alone would have reported a
false failure, and `Uploaded` alone would have missed a 400.

```sh
UA='beamr-release-script (+https://github.com/ablative-io/beamr)'
curl -s -A "$UA" -o /dev/null -w '%{http_code}\n' \
  https://crates.io/api/v1/crates/beamr/0.17.0     # 200 = live, 404 = absent
```

At staging time all three target slots were free (`404`): `beamr/0.17.0`,
`beamr-cli/0.5.0`, `beamr-wasm/0.8.0`.

## 7. After it is live

- Tag `v0.17.0` and **push it in the same sitting** — the v0.15.3–v0.16.2 tag
  gap came from deferring exactly this step.
- Roll the pin out to consumers (`aion`, `haematite`, `liminal`). These pin by
  version, so `cargo update` alone will not cross the minor; the pin must be
  edited. The script prints this list on success.

**Consumer exposure to the breaking removal — measured, not assumed:**

- **liminal: zero.** `spawn_link_dirty` has 0 hits repo-wide; the loose
  `spawn_link` form has 4, all English prose inside `LIM-002*/LIM-004` design
  JSON, 0 across all 561 `.rs` files. Its entire beamr import surface is 40
  `use` lines and none touch the dirty-scheduler surface. Measured by Hermes
  Crumpet with both controls in the exact argument form of the real query.
  **Leg G (liminal's `beamr = "0.16.1"` pin at its `Cargo.toml:32`) is a
  manifest edit plus a battery, not a migration.**
- **No feature is renamed or removed by this release.** `crates/beamr/Cargo.toml`
  is **byte-identical** between `67f89c4` (0.16.2) and main:
  ```sh
  git diff --quiet 67f89c4 main -- crates/beamr/Cargo.toml && echo "manifest unchanged"
  # control: the same instrument on a file known to differ
  git diff --quiet 67f89c4 main -- scripts/release.sh || echo "control OK: change detected"
  ```
  So `readiness`, `cooperative`, `json` and `threads` all survive unchanged.
  **Note the control is not `version` — main's version line deliberately still
  reads 0.16.2 (§3), so using it as the known-different item silently produces
  a failed control.** That is the trap in §1 again, in a third argument form.
- **`aion` and `haematite` exposure is NOT measured here.** Their owners should
  run the same two-control check before the pin bump. Absence of a finding in
  this document is absence of a measurement, not a clean result.

**One thing the release unblocks:** `watch_exit` is additive surface arriving
*in* 0.17.0, and liminal's F7 retirement (deleting `LIVENESS_POLL`/`poll_reply`,
16 live hits) composes on it. That work is gated on 0.17.0 being **published**,
not merely landed.

## 8. Do not advertise record/replay in the release notes

`ReplayRecorder` is never constructed anywhere in the tree; the recorder emits
two event kinds against the driver's five; no test sets `replay_mode: true`
against a recorder-produced log. The README was corrected at `9989828` and the
CHANGELOG must not walk it back. See `docs/design/beamr/REPLAY-SCHEDULER-PINNING.md`
for the constraint a future recorder has to be built against.

## 9. Stop conditions

Stop and route to the project lead, rather than working around, if any of these
hold:

- The battery is not green at the exact commit being cut.
- `git merge-base --is-ancestor v0.16.3^{commit} main` is false **and** the
  §2 content comparison shows any *source* file differing.
- Either caret pin in §3 is still `"0.16.0"` after the bump commit.
- `release.sh` refuses with an `unknown-<code>` state — that is the guard
  working; determine the registry's actual state before re-running.
- A publish partially completes. crates.io has no unpublish. Re-running is safe
  by design (published crates are skipped), but only once you know which crates
  actually landed.

---

**Boundary, stated because it is easy to collapse the two:** a *readiness hold*
waits for permission to ship something already proven, and those are dead by
standing directive. An *irreversibility check* asks whether this is the artifact
we think it is — that is not a hold, it is the gate, and it costs seconds.
Tonight it cost a 400-instead-of-404 rather than a bad publish. Keep the probes;
drop the waiting.
