# #116 — battery-log residue binned, and the finding that chose the mechanism

Waffles gave the word at seq=32 to run this as its own lane, **mechanism at my
call (bin or commit)**. The measurement overturned the call I was heading for.

## What was wrong

`tree pre` / `tree post` is the runner's tamper check. Its absolute reading had
been climbing lane over lane — **21 at #114, 26 at #115** — and read like drift
in an unrelated lane.

It was not drift. Established form committed only `leg4/5/8` + `.tsv` +
`BATTERY.log`; the other five per-leg logs were left untracked. So **five files
accumulated every lane, forever.**

⚠️ The residue is **invisible until the moment you commit**, which is what made
it confusing: `git status --porcelain` collapses a **wholly untracked directory
to one line**, so a fresh battery dir reads as `1` regardless of contents. The
moment `leg4/5/8` are committed the directory becomes tracked and its five
siblings begin listing individually. The +5 per lane is that transition, not a
change in the tree.

## ⭐ The measurement that overturned the mechanism call

I was heading for **"commit them all"** — uniform rule, nothing hidden, ~470 KB
a lane against a 64 MB repo. Then I looked at what is actually in them.

**Legs 2 and 7 are `clippy` under `--message-format=json`** — machine-event
streams, not human logs. They carry **absolute operator paths**:

```
/Users/tom/Developer   17,279 occurrences
/Users/tom/.cargo      12,958 occurrences
```

| | files | of | carry `/Users/tom` |
|---|---|---|---|
| untracked residue | 14 | 30 | ⛔ yes |
| already tracked under `gate-logs` | **220** | 1436 | ⛔ yes (pre-existing) |

So committing the residue would have added **14 more path-carrying files to a
standing 220** — making a pre-existing condition measurably worse **inside a lane
whose entire purpose was hygiene.** ⇒ **BIN.**

The 220 was re-measured with a **second instrument** before being written down —
`git grep` against the committed objects at `HEAD` rather than grepping the
filesystem — with both controls firing: a string that must be absent returned
**0**, a string that must be present returned **88**.

### Severity is bounded, and the bound was measured not assumed

`cargo package --list` for **all three** crates returns **0** `gate-logs`
entries (beamr's listing is 514 files). `gate-logs/` sits at the repo root while
the packages sit under `crates/`, so **none of this has ever reached crates.io.**
The exposure is repo-only, and its content is one operator's checkout path and
cargo registry path — not a credential.

⛔ **The 220 tracked files are NOT touched by this lane.** Rewriting 220
committed evidence artefacts is a decision with its own blast radius and is not
mine to take quietly inside a hygiene lane. **Recorded and routed, not acted on.**

## What was binned

30 files, 2.82 MB, listed in the commit message. Every affected pin
(`8e6943c`, `ea7211a`, `c918936`, `4a66232`, `cf00e03`, `d6c82ae`) was verified
**before** deletion to have a committed `.tsv` carrying **all 8 legs' rc**, so no
verdict was lost — only the transcript of legs that carry no axes.

⛔ **The fix is deletion, not `.gitignore`.** Hiding files from `git status` to
make the tree count read clean would corrupt the very instrument the count
belongs to. There is nothing hidden, because there is nothing left.

## What keeps it from coming back

The rule is written **at the runner** (`gate-logs/111/battery-RUNNER.sh`), where
the next operator meets it, rather than in a lane record nobody re-reads. The
change is **comment-only**, proven rather than asserted: `bash -n` rc 0, and
stripping comments and blank lines from both sides gives **byte-identical
executable text** — with a negative control confirming the stripper detects a
one-token change (`set -u` → `set -eu`).

## Result

Tracked **and** untracked census now **EMPTY** — the first fully clean tree
across this run of lanes. `tree pre == tree post` still asserts exactly what it
always did; now the absolute number is comparable across lanes too.

No battery: zero Rust delta, and the only executable file touched was proven
unchanged below its comments.
