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

`"0.16.0"` means `^0.16.0`, which **does not match 0.17.0**. **Both pins must be
edited to `"0.17.0"` in the same commit as the bump.**

**CORRECTED 2026-07-29, forward-only — an earlier revision of this section said
the `path` key masks this locally so "a green battery cannot detect it, only
`cargo publish` resolution can." THAT WAS WRONG, and the repo disproves it.**
Cargo validates a path dependency's actual version against the stated version
requirement, so a stale pin does not build green — **it makes the workspace
refuse to check at all.** This has already happened here once:

```
1b07d03 fix(release): carry the 0.14.0 bump into beamr-cli/beamr-wasm
        dependency requirements — the prior commit left the workspace uncheckable
```

**So the correct expectation for the executor is the opposite of what I first
wrote: if the battery explodes immediately after your version-bump commit with
a dependency-resolution error naming `beamr`, this is why, and the fix is the
two pin edits — not a broken toolchain and not a real defect in the tree.**

The trap is that it is easy to *forget*, not that it is invisible. Every minor
bump in this repo's history carried a manual pin edit — 0.9.0 through 0.16.0,
eight consecutive releases — and the one time it was missed it cost a follow-up
commit. Verify with:

```sh
git log -L16,16:crates/beamr-cli/Cargo.toml   # every bump, and the one miss
```

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
- **No battery has been run at `9294be0` or later.** This runbook stages the
  release; it does not certify the tree. Certification is the executor's, at the
  commit actually being cut.
- **⚠ THE CLIPPY LEG ITSELF IS UNEXERCISED.** `gates.json`'s clippy leg was
  rewritten to harness R2 at `c6043d2`, and the newest evidence under
  `gate-logs/` is `74c7d3c`, which **predates it**:
  ```sh
  git merge-base --is-ancestor \
    "$(git log -1 --format=%H -- gate-logs/)" \
    "$(git log -1 --format=%H -- gates.json)" \
    && echo "evidence predates the gate change — clippy leg never run in this form"
  ```
  **THE LEG HAS TWO OUTPUTS AND ONLY ONE OF THEM IS WALLED** (Athena Zooper
  Dooper's sharpening of my own weaker bound, which covered the verdict only):
  - `verdict: exit-code-of-cmd` guards the leg's **PASS/FAIL**. A malformed
    `extract` cannot turn a red leg green. That much is bounded.
  - **Nothing guards the `extract` jq filter itself.** A silent no-match yields
    an **empty findings list, and an empty findings list reads as "no clippy
    problems."** So the failure mode is not a false green leg — it is a red leg
    with zero listed causes, or an evidence file asserting a cleanliness it
    never measured. Same family as `grep -q`, the 403, and the narrow marker.

  **✅ DONE — THE LEG IS NOW PROVEN, AND THE EVIDENCE IS IN-TREE.** Driven to RED
  by Diana Plum at `929f4fc`, landed at `442b90c`, evidence under
  `docs/design/beamr/briefs/evidence/r2gates-929f4fc/battery/`:
  - **(a) the leg fails** — `red-demo-clippy-leg.txt`, exit **101**. This also
    settles the `--keep-going` assumption for free: cargo still exits non-zero
    when a crate fails, so `exit-code-of-cmd` holds.
  - **(b) the extract POPULATES** — `red-demo-findings.jsonl` names the real
    lint, file and line, rather than the confident empty nothing was guarding
    against.
  - **Legs verbatim, verified not asserted** — the sidecar `clippy-leg-cmd.txt`
    and `clippy-extract.jq` are byte-identical to `gates.json`'s clippy leg, and
    that comparison was itself mutation-controlled (a one-character change *is*
    detected), because a byte-identity check that cannot fail proves nothing.
  - **Clean-tree baseline** — `clean-baseline-*`, exit 0 with findings empty
    through the same pipeline. **So an empty clippy findings list now attests
    cleanliness instead of a possibly-dead filter.** That is the whole difference
    this demonstration bought.

  **⚠ AND THE MOST INSTRUCTIVE ARTIFACT IN THAT DIRECTORY IS A FAILURE.** Attempt
  1 of the harness wrote its scratch file to a nonexistent directory, silently
  ran the **clean** tree, and reported exit 0 / findings 0 — **the dead-producer
  class, enacted by the very instrument built to test for it.** It was caught
  only because the artifact had to *name* the scratch lint and could not. It is
  kept, disclosed, as `clean-baseline-*`. **A reader re-running this
  demonstration must assert the scratch file exists before driving the leg.**
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
- **`haematite`: zero.** At `db21b2c` — 324 Rust files, positive control
  (`beamr`) 55 files, negative control 0, **`spawn_link_dirty` 0 in `.rs` and 0
  anywhere in the repo**, loose `spawn_link` 0.
- **`aion`: zero.** At `1d53c193` — 1172 Rust files, positive control 109,
  negative control 0, **`spawn_link_dirty` 0 in `.rs`**. It has exactly one hit
  repo-wide and it is prose: `CHANGELOG.md:26`, *"The last `spawn_link_dirty`
  call is…"*. The 8 loose `spawn_link` hits are real calls to **different,
  surviving** APIs — `spawn_link/3`, `spawn_link_closure`,
  `spawn_linked_test_process`, `bif_spawn_link_4`.
- **Stronger than absence, and the check actually worth running: the APIs
  consumers DO call still exist.** Verified at `929f4fc`, with the removed
  symbol as the negative control through the identical command form:
  ```sh
  for sym in spawn_link spawn_link_closure spawn_linked_test_process bif_spawn_link_4; do
    git grep -c -e "fn $sym" -- ':(glob)crates/*/src/**'
  done
  git grep -c -e 'fn spawn_link_dirty' -- ':(glob)crates/*/src/**'   # must be empty
  ```
  All four survive; `spawn_link_dirty` is correctly absent. **"The removed
  symbol is not called" is a weaker claim than "the symbols they call survive" —
  the first is satisfied by a consumer that calls nothing at all.**
**⚠⚠ THE CONSUMER PINS FAIL THE OPPOSITE WAY FROM THE INTERNAL ONES, AND THIS IS
THE MOST DANGEROUS FACT IN THIS DOCUMENT.**

§3's correction — a stale pin makes the workspace *refuse to check* — is true of
**`beamr-cli` and `beamr-wasm`, because those pins carry `path`**, so cargo
validates the local crate's version against the requirement and fails loudly.

**Every consumer pin is a bare registry dependency with NO `path` key:**

```
liminal    Cargo.toml:32                    beamr = { version = "0.16.1", … }
haematite  crates/haematite/Cargo.toml:46   beamr = "0.16.0"
haematite  crates/haematite/Cargo.toml:114  beamr = { version = "0.16.0", … }
aion       Cargo.toml:89                    beamr = { version = "0.16.2", … }
```

**With no `path` there is nothing local to contradict the requirement, so a
stale pin does not fail at all — it SILENTLY KEEPS RESOLVING TO THE OLD
PUBLISHED CRATE.** The consumer builds green, tests green, ships, and runs
beamr 0.16.x forever. Nothing anywhere reports a problem.

**This has already caused a production incident, recorded in this repo at
`crates/beamr/src/scheduler/tests.rs:4224-4231`:** on 2026-07-06/07 a starvation
bug was observed in production. It was not a defect on main. `aion`'s `[patch]`
pointed beamr at the fixed local checkout for aion's own dep, while
**`liminal-server`'s `beamr = "0.11.0"` requirement silently resolved to
crates.io beamr 0.11.0, whose `RunQueue` is LIFO.** The connection scheduler is
constructed inside liminal-server, **so production ran the unfixed copy** — with
the fix sitting in the tree, and every build green.

**⇒ The internal pins cannot hurt you: they stop the build. The consumer pins
can, precisely because they do not.** The rollout step in this section is not
bookkeeping; **it is the only thing standing between a published 0.17.0 and
three consumers that quietly never receive it.**

**⇒ And note what this does to the ordering rule above: the battery certifies
resolution health for the beamr WORKSPACE only. It never looks at a consumer
tree, so it cannot detect a stale consumer pin in either direction.** After the
rollout, verify per consumer that the resolved version actually moved — read the
consumer's `Cargo.lock` for the `beamr` entry rather than trusting that a green
build means it took the new version. A green build is exactly what the 2026-07
incident produced.

- **Consumer pins that must be edited (all caret, none matches 0.17.0):**
  `haematite/Cargo.toml:46` `beamr = "0.16.0"` and `:114` the native-only
  dev-dep `{ version = "0.16.0", features = ["cooperative"] }`; `aion`'s
  workspace pin `Cargo.toml:89` `{ version = "0.16.2", features = ["json",
  "encode"] }`. *(`aion`'s `.meridian/workflows/stacked-dev/worker/…` scaffold
  pins `0.6.1`; that is a template, not a live edge — do not "fix" it as part of
  this rollout without asking its owner.)*

**⚠ ORDERING — THE BATTERY AND THE VERSION BUMP (Athena Zooper Dooper, and it
follows from the §3 correction).** If a stale pin makes the workspace *refuse to
check* rather than build green, then **resolution health is a property of the
tree at the moment of the run**, and the battery certifies it only for the tree
it ran on. **A version bump landing AFTER the battery silently invalidates the
resolution half of its evidence while leaving the report green and quotable.**

**⇒ Either the battery runs on the tree that will be published, or it re-runs
after the bump. Do not let the bump land between the battery and the publish.**

Note the polarity, because "I had this backwards" should not be misread as worse
news: **the corrected mechanism is the SAFER one.** A stale pin cannot reach the
registry through a green battery — the workspace refuses at the first compiling
leg, loudly. Under the original (wrong) belief the bump was invisible to the
battery and ordering did not matter; under the corrected mechanism the battery
is a genuine detector, which is exactly why it must be pointed at the right tree.

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
