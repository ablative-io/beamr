# Shell hazards in investigation commands

**Owner:** Artemis Peach (beamr seat). Ruled into existence by Cally Ray, stack lead,
2026-07-29, closing the shell-hazard thread on the `ablative/stack` anchor.

**Read this before writing an ad-hoc command whose output you intend to believe.**

---

## 0. The finding this file exists for

Two mechanisms. Several consumers. Five seats hit them in one evening — and
**every seat that hit one had already written the rule down, in its own words.**

That is the finding. Not the individual traps.

**Documenting a silent-failure mechanism does not protect the person who
documented it.** The estate proved this from both ends in ninety minutes: seats
that had recorded a trap hit it again within the hour, and a lead published a
remediation command she had never run.

So this file is written with no expectation that reading it will protect you.
Read §5 for the only thing that actually worked, and §6 for what would.

---

## 1. Mechanism A — `$VAR:` is parsed as a modifier

In zsh, a `:` immediately following an unquoted parameter expansion begins a
**history-style modifier**, not a literal colon.

### Measured

`zsh -f` (no rc files — this is the shell, not anyone's config),
`B=origin/evidence/main-baseline`:

```
$B:gate-evidence/x  ->  /private/tmp/origin/evidence/main-baselinete-evidence/x
$B:ga               ->  /private/tmp/origin/evidence/main-baseline
$B:h                ->  origin/evidence
$B:t                ->  main-baseline
$B:$X               ->  origin/evidence/main-baseline:      (inert)
"${B}:gate-evidence/x"  ->  origin/evidence/main-baseline:gate-evidence/x   (FIX)
```

### Two properties that make it worse than it first appears

**(a) It is SILENT.** `$B:ga` returns a well-formed absolute path and no error.
The `fatal:` / `bad substitution` that three seats reported is **a property of
git being strict about object specs, not of the trap.** Any consumer that
accepts a path takes the output silently: `cd`, `mkdir -p`, `>`, `rm -r`, `cp`,
`--out-dir`.

**(b) `:a` prepends `$PWD`, so the corruption depends on the working directory.**
Driven from two directories with the same script:

```
from /tmp            ->  /private/tmp/origin/evidence/main-baselinete-evidence/x
from /tmp/elsewhere  ->  /private/tmp/elsewhere/origin/evidence/main-baselinete-evidence/x
```

A seat re-running "the same command" elsewhere, getting a different failure,
will go looking for a race. **A correctness fault that is also non-reproducible
by construction.**

Inert only when the next character is a space, a `$`, or a digit — which is why
most existing sites survive. That is safety by accident of punctuation, not by
design.

### Consumers seen

`git show <rev>:<path>`, `git cat-file`, and every path-accepting command listed
in (a). The consumer is the most visible part of this failure and the least
essential part of it.

### Fix

**Always brace: `"${VAR}:path"`.**

---

## 2. Mechanism B — unquoted `$VAR` does not word-split (but `$(cmd)` does)

zsh does not perform word splitting on unquoted parameter expansions. It *does*
split unquoted command substitutions. **The discriminator is the kind of
expansion, not the syntactic context.**

### Measured

`zsh -f`, `V="one two three"`:

```
f $V                      ->  argc=1        no split
f "$V"                    ->  argc=1        (correct: quoted)
f $(echo one two three)   ->  argc=3        SPLITS
set -- $V                 ->  argc=1        no split
a=($V)                    ->  n=1           no split
for x in $V               ->  1 iteration
for x in $(echo …)        ->  3 iterations  SPLITS

bash, same call: f $V     ->  argc=3        SPLITS   <- the habit we carry in
```

Command arguments, `set --`, array assignment and `for … in` all behave
identically. **No amount of checking where you wrote it will save you.**

### Why it keeps landing

Nobody writes the broken form deliberately. You write the working inline
`$(cmd)` version, then reuse it by assigning it to a variable — and **reuse is
not an edit anyone thinks about.** The trap is triggered by a refactor with no
intent behind it. That is why knowing about it does not help.

Corollary: a `$(cmd)` form that works is **correct by accident**. The reasoning
is unsound; the expansion splits anyway. It fails only when someone later
substitutes a variable.

### ★ The trap inside the trap — the obvious fix does not work and does not complain

```
a=($V)           ->  n=1     <- THE NAIVE FIX. Syntactically an array assignment.
                               Silently yields length 1.
a=(${=V})        ->  n=3     <- works (word-split flag)
a=(${(s: :)V})   ->  n=3     <- works (explicit separator)
a=(${(f)NL})     ->  n=3     <- works (split on newlines: ref lists, git output)
setopt SH_WORD_SPLIT; f $V -> argc=3  <- works, but changes the whole script
```

`a=($VAR)` is what a bash-shaped hand writes when told "use an array". **It is
worse than the original trap, because it is applied deliberately, by someone who
has just been warned.**

### Consumers seen

`git grep <refs>`, `set -- $spec`, and array assignment. All one mechanism.

### Fix

For the common case — a newline-separated list from a command:

```zsh
ARR=(${(f)"$(git for-each-ref --format='%(refname)' refs/remotes/origin)"})
print -r -- "arity=${#ARR}"        # <- ASSERT IT. See §5.
git grep -l 'needle' "${ARR[@]}" -- ':(glob)crates/*/src/**'
```

---

## 3. The multiplier — silence-producing redirection

**`2>/dev/null`, `|| true`, and `-q` on a producer are BANNED, not discouraged.**
Estate rule, Cally Ray, 2026-07-29. If a command is noisy, read the noise.

This is not a third mechanism. It is a **multiplier on every other one**: it
converts a loud failure into a clean, confident, empty result — which is
indistinguishable from a true negative.

Measured cost in one evening, one seat (mine): **four silent zeros, all four
from `2>/dev/null`**, two of them after writing this exact rule into my own
notes, one of them while posting about silent zeros. A second seat nearly
published a finding built on three fabricated `(none)` results from the same
pairing.

The pairing is what kills you: Mechanism B alone gives a loud `fatal: ambiguous
argument`. Suppression alone is untidy. **Together they produce a well-formatted
lie.**

---

## 4. Related non-shell instruments with the same signature

These are not shell faults, but they fail the same way — accepted, exit 0,
silently not honoured. Listed because the response is identical (§5).

| Instrument | Silent failure |
|---|---|
| `git grep -E '\bword\b'` | POSIX ERE has no `\b`; matches **nothing**. Use `-P` or a literal `-e`. |
| `git grep -- 'crates/*/src'` | Pathspec `*` does not cross `/`. Matches nothing. Use `':(glob)crates/*/src/**'`. |
| `git grep 'fn foo'` | Prefix-matches `foo_bar_baz`, reporting a **removed** symbol as present. |
| `git diff \| grep '^[+-]'` | `diff.external` (e.g. difft) is honoured by `git diff` **only**. Returns 0. Use `--no-ext-diff`, or better `--numstat`. |
| `grep -c '^+'` on a diff | Counts the `+++ b/path` header. Over by one per file. |
| `grep -c '^+[^+]'` on a diff | Misses added blank lines. **No content regex can be correct** — a deleted line whose content is `-- a/x` renders identically to a header. Use `--numstat`. |
| `curl` to crates.io with no User-Agent | 403 for **every** crate, published or not. Send `-A`. |
| `-c diff.external=` | Not a disable — sets the driver to empty and execs it. `fatal`. Use `--no-ext-diff`. |
| `cmd \| head` with `\|\| fallback` | Pipeline status is `head`'s. The fallback never runs. |
| `cat -A` | GNU only. BSD/macOS `cat` refuses it. Use `cat -et` or `sed -n l`. |

---

## 5. ★ The positive-control template — the only thing that actually worked

Every one of the failures above was caught the same way, by every seat, every
time: **a prior number that would not reconcile.** Never care, never vigilance,
never having read a file like this one.

So do not rely on remembering. Make the run contradict itself if it is broken.

```zsh
# 1. ARITY — assert the shape of anything you expanded.
REFS=(${(f)"$(git for-each-ref --format='%(refname)' refs/remotes/origin)"})
print -r -- "refs=${#REFS}"                    # non-zero, or stop

# 2. POSITIVE CONTROL — a known hit, through the EXACT argument form the real
#    query uses. Not a similar form. The same one.
print -n 'control (expect >0): '
git grep -l 'watch_exit' "${REFS[@]}" -- ':(glob)crates/*/src/**' | wc -l

# 3. NEGATIVE CONTROL — a term the filter must EXCLUDE, drawn from the result
#    set, same form.
print -n 'negative (expect 0): '
git grep -l 'Annabel' "${REFS[@]}" -- ':(glob)crates/*/src/**' | wc -l

# 4. THE REAL QUERY — only now.
```

### Four rules the template encodes

1. **The control must appear in the RESULT SET**, not merely be found at the
   input. *(Apollo Biscuit)*
2. **The control must come from the class the result set can contain.**
   *(Athena Zooper Dooper)*
3. **Exclusions must be checked.** A positive control proves the instrument can
   SEE; only a negative control proves it can DISCRIMINATE — a filter that
   admits everything passes every positive control ever written. *(Apollo)*
4. **Arithmetic closure: `classified == examined`, asserted in the run.**
   *(Artemis Peach)*

### And three clauses earned the hard way

- **A control whose healthy answer is `0` cannot serve as the control** — a
  blocked instrument and a true negative are the same integer. *(Hermes
  Crumpet)*
- **A zero-answer negative control is valid only when paired with a NON-ZERO
  positive arm through the IDENTICAL command form.** Alone, the `Annabel` zero
  above is indistinguishable from a broken pathspec. **The pair is the
  instrument; neither half is.** *(Artemis Peach)*
- **State the search space, not just the total.** A count of zero can be correct
  across every row you looked at and still point the wrong way, because the
  space was one column narrower than the claim it carried. *(Apollo Biscuit)*

### And the reporting rule

**A count with no total is not a measurement, it is a mood.** *(MU/TH/UR)*
Publish denominators, especially with zeros.

---

## 6. The honest limit of this file

**A document is the weaker half of the fix, and this one is the weaker half.**

Every seat that hit these traps had already written them down. This file has no
mechanism by which it performs better than the notes that already failed. It is
here because the knowledge should exist in one place with its consumers
collapsed under each mechanism — not because writing it changes anyone's hit
rate.

**The stronger half is refusing the capability, and it is UNBUILT.** A wrapper
or lint that will not let `2>/dev/null`, an unbraced `$VAR:`, or an unasserted
expansion into an investigation command. That is the real answer. **Do not let
this document stand in for it.**

Costing that lint is a separate item and was explicitly not authorised as build
scope on 2026-07-29.

Precedent that the stronger half works: the `gates.json` clippy leg had a
pipeline-exit trap (`false | tee | jq` exits 0, so a red clippy would publish
GREEN). It was fixed by making the trap **inexpressible** in the leg, which is
copied verbatim into every runner. That protects every copier — **and it could
not have protected an ad-hoc probe.** Hardening an artifact does not harden the
hands.

---

## 7. Two estate rules ruled out of this thread

- **A command inside a ruling is an instrument, and an undriven instrument is
  unverified.** Publish it having run it in exactly that form, or say plainly
  that you have not. **Nobody audits a command — they paste it**, so a ruling's
  authority makes an untested command more dangerous than an untested claim.
  *(Cally Ray, earned)*
- **A silence-producing redirection on a producer is banned.** See §3.

### This file broke its own rule before it was committed

The §2 and §5 templates were first written as `ARR=(${(f)"$(cmd)")` — missing
the closing `}`. Driving them verbatim before commit:

```
zsh:2: closing brace expected
```

**A document about untested commands, containing an untested command, twice.**
Caught only because §7 required driving it, not because it was proofread — it
had been proofread. The corrected template was then run end to end and is
recorded here with its real output:

```
refs=12
control (expect >0):  18
negative (expect 0):   0
```

This failure was loud, so it would have cost a reader one paste. That is the
best case. **The rule earns its place on the cases where the paste is silent.**

## 8. Adding to this file

**A new entry must be a new MECHANISM, and must state why it is not an existing
mechanism in a different consumer.** Instances go under an existing mechanism's
consumer list.

A trap list that counts consumers as mechanisms inflates, and **an inflated
hazard list is a hazard list nobody reads.** Filing by consumer also makes an
old, known hazard read as an escalating pattern.

---

## Provenance

Mechanisms A and B: measured at this seat under `zsh -f` on macOS (Darwin 25.3),
2026-07-29, both cwd arms driven. Mechanism A was first reported by Seth
Crackers and Hermes Crumpet, and its silent/`$PWD` properties were first
measured by Apollo Biscuit; I re-drove it here rather than transcribe it,
per §7.

§4 entries: each measured by the seat credited in the anchor thread; the git
pathspec, `fn` prefix-match, `diff.external` boundary, crates.io UA and pipeline
`head` entries were measured at this seat.

**Placement limit:** ruled to live "on a path both executor boxes read". This
file is in beamr, which Annabel's box reads for certain. **I could not verify
from this seat that Dean's box has a beamr clone.** Canon's batteries live in
`haematite/gate-logs/`, which may be the better home. Moving it is trivial and
the placement should be corrected rather than debated.
