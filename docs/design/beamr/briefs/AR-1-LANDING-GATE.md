# NATIVE-BIF ROOTING LANE — my landing gate, WRITTEN BEFORE THE FIX EXISTS

**Cally Ray, 2026-08-07 — committed 08:48:43Z (`35dc5ae`).** Artemis has the ground pack; **no fix is designed, none applied.**

> ⚠️ **TIMESTAMP CORRECTION, 2026-08-07T11:00:45Z (`34552cc`).** The three headers above originally read `~08:40Z`,
> `~08:50Z` and `~08:55Z` — estimates taken from narrative flow, not from a clock. Measured against
> the commits that carry them (`git log --date=iso-strict-local`), **two were impossible**:
> amendment 1's `~08:50Z` sits *inside* `35dc5ae`, authored **08:48:43Z**, and amendment 2's
> `~08:55Z` inside `ab1f1f9`, authored **08:53:36Z**. A timestamp cannot post-date the commit that
> contains it. The `~08:40Z` on the gate itself is unrecoverable and has been replaced by its
> commit time rather than a better guess.
> ⇒ **Forward rule, and it binds me first: a header takes its time from the commit or carries none.
> A version-controlled artefact's date has an authoritative instrument inside the commit that
> carries it, so claiming it from memory is a carried count with extra steps.** This is the same
> defect I scored in the outward-check doctrine's F5 an hour after writing it here myself.
> ⚠️ **AND THIS NOTE REPRODUCED THE DEFECT IT CORRECTS — caught by Artemis Peach, push held.** It
> first read `11:05Z`; the commit carrying it, `34552cc`, was authored **11:00:45Z**. **+255 s: the
> correction post-dated its own commit, four lines below the sentence forbidding exactly that.** It
> now takes its time from that commit, as the rule requires.
> ⇒ **The rule resolves its own bootstrap:** a correction cannot know its future commit's time, so it
> **cites the commit that carries it** rather than predicting a clock. **A note that names its commit
> can never post-date it.**


**Nothing below has been informed by looking at a fix, deliberately.**

**Why pre-register, again.** Same reason as #79 INC-1: ⭐ **A CHECK AUTHORED AFTER THE ARTEFACT CAN BE
SHAPED BY IT, AND THE SHAPING IS INVISIBLE TO THE PERSON DOING IT.** With one addition specific to this
lane — **the author of the fix is the author of the instrument that will grade it.** Artemis wrote
`r6_stage3.py`; if she also sets the acceptance bar, the sweep, the fix and the verdict are one hand.
**That is not a comment on her work — she has caught more of her own errors tonight than I have of
mine.** It is a structural point: an author writing their own acceptance criteria is the one review
shape that cannot catch anything.

**Subject:** the 14 crossings verified at beamr `850cdc0` — 13 REAL + 1 REAL-OSIRIS — of which **11 are
one shape**: build a term in a loop → push into a `Vec<Term>` or thread through a `tail` → hand the
collection to `alloc_list`/`alloc_tuple`. Every allocation after the first can collect; nothing roots
what is already in the accumulator.

**Standing rule for every row: a reading is not a result.** Where the property can be broken, I break it
and require the check to go RED. A green never observed red is a regression test, not a wall.

---

## ROW STATUS AT HEAD — A POINTER TABLE, NOT A SECOND SOURCE

⚠️ **THIS TABLE MAY NAME WHERE A ROW'S CURRENT FORM LIVES. IT MAY NEVER RESTATE THE CRITERION.**
A criterion copied here becomes a second source that can disagree with the amendment, which is the
exact defect the `created`-field deletion cured this same night. **If a cell needs a second line, the
cell is wrong.** The amendments are authoritative; this is an index, and an index that grows into a
payload rots — measured, on my own memory index, an hour before this table was written.

| row | current state | last moved by |
|---|---|---|
| 1 | OPEN — ordering-sensitive detector, unbuilt | as written |
| 2 | OPEN — ledger **pre-registered**, `evidence/accumulator-rooting/{dispositions,ledger_check}.*` | A5 |
| 3 | OPEN — four `UNRULED-PRERESERVE` rows undischarged | as written |
| 4 | **PARTLY DISCHARGED** — leg priced (28.19 s, setup cost zero); **13 + 17 red-at-parent DEMONSTRATED**, 12 + 16 not; per-site commit granularity still required | A3, A6 |
| 5 | ✅ **MET** — ancestry checked | A2 |
| 6 | OPEN, **unfired** — my sighting was false and is retracted | A4 |
| 7 | ✅ **DISCHARGED** — S3e hunt, 14 → 17 | A1 |
| 8 | OPEN — Osiris' verdict, not this lane's to give | as written |
| 9 | ✅ **DISCHARGED** — 56/56 measured | A2 |
| R10 | ✅ **DISCHARGED** — 5 fixtures + 5 controls; fmt/clippy/test green, hunt finds five | A5 |

**Counts at `308b448`:** 39 raw hits · **27 production · 12 `cfg(test)`** · **the seventeen unchanged.**
Production is unchanged at 27; raw and the `cfg(test)` column moved when R10's five fixtures landed.
S3c is **0 but now controlled** — `ar1_shape_control.rs:152` holds it, so the zero is interpretable.
⭐ **A COUNT LABELLED "AT HEAD" CARRIES AN EXPIRY THAT A COUNT LABELLED WITH A SHA DOES NOT** — same
numbers, different lifetimes, and only one of them rots. These are pinned to a sha for that reason.

---

## THE ROWS

### 1. ⛔ SIGN-BLOCKING — THE DETECTOR IS ORDERING-SENSITIVE, NOT PRESENCE-SENSITIVE

**This is the row the whole lane turns on.** The terminal `alloc_list` **does** root its elements — it
is rooting values that are **already stale**, and rooting a stale pointer does not recover it. ⇒ **every
one of the eleven ALREADY CONTAINS A ROOTING CALL.**

- **Read:** the rule flags a carrier when a rooting call occurs **after** the first collecting call on
  that carrier's live range — never merely "a rooting call is absent from this function."
- **Break it:** construct a site with `with_rooted`/`rooted_push` present but placed **after** the
  collecting allocation. **The detector must flag it.** If it clears, the instrument is measuring
  presence and **passes all eleven while they are still broken.**
- **Control:** a correctly-ordered site must come back clean, or "flags everything" reads as a pass.

### 2. ⛔ SIGN-BLOCKING — THE POST-FIX COUNT MUST BE 14 RECLASSIFIED, NOT 14 VANISHED

**The most likely way this lane ships broken, and it will look like success.**

- **Read:** after the fix, re-running the sweep must still yield **population ≥ 69**, with the fourteen
  **present and classified `SAFE-ROOTED`**.
- ⛔ **A count that drops from 14 REAL to 0 REAL by the sites no longer MATCHING is a FAILING result,
  not a passing one.** ⭐ **A FIX THAT MAKES A DEFECT INVISIBLE TO THE INSTRUMENT IS INDISTINGUISHABLE
  FROM A FIX THAT REMOVES IT — and from an instrument that broke.** The honest shape is
  **14 REAL → 14 SAFE-ROOTED**, each nameable.
- **Break it:** narrow the binder so one site stops being recognised. **The population must fall and the
  fall must be reported**; if the run still says "0 REAL, clean", the acceptance is decorative.

### 3. ⛔ SIGN-BLOCKING — THE FOUR `UNRULED-PRERESERVE` ROWS ARE DISCHARGED OR PROMOTED

Relabelled from `SAFE-PRERESERVE` by ruling: they were cleared on **word-count arithmetic nobody has
audited** (`info_proplist_heap_words`, `value_heap_words`, `system_info_bifs.rs:186`).

- **Read:** each of the three calculations is audited and shown to cover **every allocation in the
  sequence it is claimed to cover**, or the row is **promoted to REAL and fixed**.
- **Break it:** subtract one word from a reservation. **A test must red.** If nothing does, the
  reservation is an assumption with no wall behind it and all four rows are REAL by default.
- ⭐ *A prereservation is a promise made by an arithmetic expression; clearing a site on it inherits
  that arithmetic.* **This row may not be closed by restating the assumption more carefully.**

### 4. RED-FIRST, PER SITE — not one test for the class

- **Read:** each fixed site has a test that is **red at its own parent commit** and green after.
- **Verify by running it at the parent**, not by reading the commit message. ⭐ *Landed is not running;
  a described wall is not an observed one.*
- ⚠️ **A single class-level test is NOT sufficient.** Eleven sites sharing a shape does not mean one
  test covers eleven code paths — **that is the same "two partial proofs compose" error**, and here the
  boundaries provably do not meet: the carriers differ (`Vec<Term>` vs threaded `tail`), and a test
  driving one exercises neither the other's accumulation nor its terminal call.

### 5. THE REMEDY'S BLAST RADIUS IS MEASURED BEFORE IT IS DESIGNED

- **Read:** a count of **every** `alloc_list`/`alloc_tuple` call site and how many already hold a rooted
  collection, **produced before any type-level change is written.**
- **Why:** the candidate deletion — make a bare `Vec<Term>` unable to reach the allocators — is the only
  shape that stops the fifteenth site being written next month. **But** ⭐ *a proposed remedy is a claim,
  and its blast radius is a measurement, not an inference from the finding's name.* 20 sites ⇒ the type
  change is right. 200 ⇒ it is not, and fourteen patches plus a lint is the honest answer.
- **This row fails if the design lands first and the count is produced to justify it.**

### 6. NO SILENT ARM — refusal, not a stale return

Mirrors the RF-006 fix shape already verified at `d5c3eee`.

- **Read:** where rooting cannot be established, the path **refuses** (named error / `return None` /
  `return 0` with a null-checked caller) rather than proceeding with an unrooted term.
- **⚠️ The specific defeat:** an `unwrap_or(term)` or a `.unwrap_or_default()` on the rooted handle
  re-admits exactly what this row forbids and will look like defensive programming.
- **Break it:** add a refusal path with no caller check. **Must fail to compile or must red.**

### 7. THE INSTRUMENT'S BLIND SPOT IS TESTED, NOT ASSERTED

C-vi proved the binder had **two** invisible carrier shapes. The lane's own honest claim is *complete
over what the instrument can see*.

- **Read:** a **third** carrier shape is actively hunted before sign-off — at minimum, carriers bound
  from a `match`/`if` arm, from a closure return, and from a struct field.
- ⇒ either a third shape is found (population grows again, verdicts re-run) **or** the search is
  recorded with its patterns so the blind spot is bounded rather than hoped. ⭐ *"Not found" and "not
  looked for" produce the same output and mean opposite things.*

### 8. OSIRIS' SITE IS NOT ABSORBED

- **Read:** `string_bifs::bif_split/terms` is fixed **by Osiris, in his lane, with his verdict**, or is
  explicitly still open. **It may not be quietly swept into this lane's patch set** — nor may this lane
  report "14/14 fixed" while one of them was somebody else's call to make.
- **His verdict may be "false positive."** The instrument's FP rate is **20/69 ≈ 29%**. If he refutes
  it, **the count drops to 13 and the advisory says 13** — a corrected number is worth more than a
  padded one.

### 9. THE ADVISORY AND THE `0.16.4` NOTES

- **Read:** `0.16.4`'s notes name this class **open and SIZED** — "14 sites, one shape, native BIFs, no
  JIT required, present in every released version." A bound beats a worry.
- ⛔ **`0.16.4` must NOT be announced as "the memory-safety fix" full stop** while this class is open —
  that is the ornamental-surface defect that opened the whole lane, committed by us, in the artefact
  that exists to correct it.
- The false *"RF-006 is an ABI-level GC-rooting change"* text ships in released `0.17.0` and wants a
  **forward-only appended correction** at landing.

---

## THREE THINGS I MUST NOT LET MYSELF DO

1. **Accept a green from an instrument I have not seen return red** — every row above has a break-it arm.
2. **Read any file without naming a commit.** `git -C <repo> show <rev>:<path>`, never bare, and
   `git status --short` on the graded tree before any claim. ⭐ *A working tree is not a commit* — my
   own error twice tonight, both times on trees that **agreed with the right answer.**
3. **Grade the fix on the box that produced it, oversubscribed.** Battery runs under the ruled
   precondition — **1-min load ≤ core count, both measured at run time, three consecutive samples**
   (Athena's debounce; the box oscillates, 71.58 → 27.00 → 43.21 inside 20 minutes).

## ALSO AT LANDING (standing)

`wc -l` every file in the diff — flag god-files before they grow. Strip workflow artefacts. Diff against
**current main**, not the branch's base: ⭐ *both ends move.*

## THE ONE I EXPECT TO BE ARGUED

**Row 2.** "The sweep reports zero REAL, so it's fixed" will be offered, and it will be offered by a
green instrument on a clean tree. **A zero is the same output whether the defects are gone, the binder
stopped recognising them, or the script errored and something ate the exit code** — all three have
happened on this lane tonight. **The fourteen must be nameable, present, and reclassified.** ⭐ **AN
ABSENCE IS ONLY EVIDENCE WHEN THE INSTRUMENT HAS BEEN SHOWN, THAT RUN, TO BE ABLE TO PRODUCE A PRESENCE.**

---

# ⚠️ POST-ARTEFACT AMENDMENT 1 — ROW 2 WAS DEFECTIVE. Cally Ray, 2026-08-07, 08:48:43Z (`35dc5ae`)

**Recorded here, in the gate, with its mechanical reason — because a ruling that lives only in a DM is
not pre-registered, it is remembered.**

## WHAT COMPELLED IT

Artemis ran row 7 before any fix existed and found a **third carrier shape in one pass** — `S3e`, a
`Vec<(Term, Term)>` accumulator whose push argument is a **tuple literal**, invisible to both binder
generations *by construction*. **14 → 17.** All three new crossings sit in **files that already hold a
known crossing** (`term/json.rs` = #11; `beamr-wasm/convert.rs` = #12, #13).

⇒ **ROW 2 AS WRITTEN — "14 REAL → 14 SAFE-ROOTED, each nameable" — WOULD HAVE GONE GREEN WITH THREE LIVE
DEFECTS IN THE VERY FILES BEING REPAIRED.** The fix would have touched those files, the count would have
reconciled **perfectly**, and the shape would have shipped.

⭐⭐ **A RECONCILIATION IS ONLY AS WIDE AS THE POPULATION IT RECONCILES. PINNING A COUNT MAKES THE COUNT
AUDITABLE AND MAKES THE DENOMINATOR INVISIBLE** — and a fix-time re-run of the instrument that *defined*
the denominator cannot widen it.

**Discriminator applied (compelled by external measurement, not by the artefact's shape):** *would the
amendment have been necessary had the fix been built differently?* **S3e exists in released code
regardless of any fix. ⇒ yes, compelled.**

## ROW 2, AMENDED

> **Read:** at fix time the population is **re-derived by a DIFFERENTLY-SHAPED instrument**, not by
> re-running `r6_stage3.py`. The seventeen must be **present and reclassified `SAFE-ROOTED`**, and the
> re-derivation must **carry its own known-positive control and exit non-zero if the control fails.**
> **A count that falls to zero by sites no longer MATCHING remains a FAILING result.**
> **Break-it:** narrow the binder so one site stops being recognised — the population must fall **and
> the fall must be reported.**

## 🔴 NEW ROW — R10 ⛔ SIGN-BLOCKING (added by this amendment, not optional)

> **THE SHAPE HUNT RE-RUNS AT FIX TIME AND MUST COME BACK EMPTY — with its known-positive control
> firing in the same run.**
> **Rationale:** S3e was found in one pass **after** the class was believed sized at 14. The lane may not
> sign off on "no fourth shape" while the only evidence is that nobody looked again after the fix moved
> the code. ⭐ *A blind spot disclosed is not a blind spot discharged; if naming it is cheaper than
> searching it, the naming buys silence* (Artemis, on her own caveat).
> **Break-it:** remove the control's target — the hunt must **exit non-zero**, not report clean.

## AMENDMENT 2 — THE CRITERION SURVIVES, AND S3e IS ITS PROOF

The pre-registered remedy criterion — *a remedy is IN only if the unrooted-accumulator shape CANNOT BE
WRITTEN* — **is strengthened, not threatened.** A lint, a doc, or a rooted-`Vec<Term>` handle would each
have covered the two known shapes and **left S3e writable, because nobody knew to name it.**
⇒ **any remedy must make the shape unwritable for `Vec<(Term, Term)>` and for tuple-literal pushes too.**
⭐ **A CRITERION THAT SURVIVES CONTACT WITH A CASE DISCOVERED AFTER IT WAS SET IS THE STRONGEST THING THAT
CAN BE SAID FOR A CRITERION** — and the site-count bracket it replaced would have passed S3e silently.

## ROUTING, NOT MINE TO RULE

`loader/decode/etf.rs:120` is the same shape and is **VESPER'S CLAIM** — untouched, unruled, routed to
her, not absorbed. Same rule as `string_bifs::bif_split` staying Osiris'.

## 🔴 MY ERROR, RECORDED WHERE IT HAPPENED

This gate was first committed **only** to `ablative/docs`, which is **local-only by ruling: no remote,
single copy.** Its subject **could not open it** — she said so rather than pretending, and fetched to
check. ⭐⭐ **A PRE-REGISTERED GATE THE SUBJECT CANNOT READ IS NOT PRE-REGISTRATION, IT IS A SECRET.**
Pre-registration's value is that criteria are fixed **and visible** before the work; if only the
gatekeeper can see them, nobody can verify they were not edited afterwards. **This is the second time
tonight I have pointed someone at an unreachable citation — I diagnosed the identical fault in a runner
citing this same repo an hour ago.** This copy is now **canonical for AR-1**; the `docs` copy is history.

---

# ⚠️ POST-ARTEFACT AMENDMENT 2 — TWO ROWS WERE UNMEETABLE. Cally Ray, 2026-08-07, 08:53:36Z (`ab1f1f9`)

**Both raised by Artemis at the bytes, both confirmed at mine before ruling. R10 was unmeetable BY
CONSTRUCTION — it could not pass after the fix it exists to grade.**

## ⛔ R10 WAS SELF-DEFEATING

`shape_hunt.py:65-69` keys its known-positive control on `code_management_bifs.rs:148`
(`list = context.alloc_cons(tuple, list)?`) — **verified live at my hands.** That is a defect **this
programme repairs.** ⇒ the moment it is fixed, the control's target ceases to exist, the hunt exits 2,
and **R10 reads "the instrument went blind" when what happened is "the lane did its job."**

⭐⭐ **A KNOWN-POSITIVE CONTROL KEYED ON A LIVE DEFECT IS DESTROYED BY THE REPAIR IT EXISTS TO SURVIVE.**
A control's positive must be **stable under the change being graded** — a fixture, never a finding.

**And "come back empty" was wrong independently:** the hunt currently returns **32 hits**
(S3a 8 · S3b 11 · S3d 4 · S3e 9), nearly all ruled not-defects. **Empty is unreachable, and the only way
to force it is to narrow the hunt until it sees nothing — which is exactly what row 2 forbids, committed
in order to pass row 10.** ⭐ *A success condition that can only be met by blinding the instrument is an
instruction to blind the instrument.*

## ⛔ ROW 2 CONTRADICTED THE REMEDY CRITERION

Criterion: a remedy is IN only if the shape **cannot be written.** Row 2: the seventeen must be
**present and reclassified `SAFE-ROOTED`.** **A remedy that satisfies the criterion necessarily makes
those sites stop matching** — no carrier, no bind, nothing for any binder to classify. ⇒ **the better the
remedy, the more certainly row 2 reds.** The two selected opposite outcomes.

⭐⭐ **ROW 2'S INTENT WAS RIGHT AND ITS IMPLEMENTATION WAS WRONG: it separated "defect gone" from
"instrument blind" by keying on POPULATION MEMBERSHIP — and membership is precisely what a
deletion-class remedy destroys.** My error, and the same shape as pinning a count: I graded the
numerator and trusted the divisor, then graded presence and trusted the shape.

## THE REPAIR — ONE LEDGER, TWO ROWS

**ROW 2, AMENDED AGAIN.** Each of the seventeen is accounted for **BY NAME** at fix time as exactly one
of:
- **(a) `SAFE-ROOTED`** — rooted in place, still recognisable to a re-derivation; or
- **(b) `STRUCTURALLY-ELIMINATED`** — the shape is gone, **and the replacement construct is NAMED and
  SHOWN PRESENT at that site**; or
- **(c) `FIXED-UNVERIFIED`** — repaired but not exercised (see row 4). **Sign-blocking, never silent.**

**⛔ SILENCE ABOUT A SITE IS THE FAILURE.** A count falling to zero with **seventeen named dispositions**
is a PASS; falling to zero with **seventeen silences** is the defect the row exists to catch.

**R10, AMENDED.** Control points at a **committed synthetic fixture** the lane will never repair.
Success is **not** "empty": **every hit is RULED, and the ruled set matches the pre-registered ledger.**
A new unruled hit fails. **A vanished hit fails unless it carries a disposition.**

## ⭐ TWO CONDITIONS I ADD, BECAUSE THE REPAIR OPENS THEM

1. **ONE LEDGER, NOT TWO.** Row 2 and R10 are keyed on the same population and **must share a single
   disposition ledger.** ⭐ **TWO ACCOUNTINGS OVER ONE POPULATION WILL DISAGREE, AND THE DISAGREEMENT
   RESOLVES TO WHICHEVER IS CHECKED SECOND** — a site marked eliminated in one and missing in the other
   is the seam this whole gate exists to close.
2. **THE DISPOSITION IS MACHINE-VERIFIED, NOT ASSERTED.** Every `STRUCTURALLY-ELIMINATED` row **cites
   file:line of the replacement construct, and a checker confirms it is present at that site.**
   ⭐ *Without positive evidence at the site, "structurally eliminated" is an unfalsifiable claim in a
   table* — the same defect as a caveat in prose beside a SAFE label.

## ROW 4 — the wasm leg is priced or the site is named

Sites 16/17 are in `beamr-wasm/src/convert.rs`; **17 (`value_to_term`, `:199`) takes `JsValue`/`Object`,
so a red-at-parent test needs wasm32 + `wasm-bindgen-test` + node** — a build leg nobody has costed.
**Price it, or the site's disposition is `FIXED-UNVERIFIED` and sign-off blocks until the leg lands or
the risk is explicitly ruled accepted.**

> ⛔ **THE CLAUSE ABOVE IS RETAINED AS WRITTEN, AND ITS PREMISE WAS FALSE.** The leg was already
> installed, pinned, and wired as `wasm-tests` in `gates.json:7` and in CI before this lane existed.
> ⭐ **AN ABSENCE CLAIMED WITHOUT A SEARCH IS NOT A FINDING, IT IS A DEFAULT.** See **Amendment 6**;
> the row's requirement survives intact, only its cost estimate was wrong.

⛔ **The one escape that stays forbidden is a class-level test
standing in for it.** ⭐ *Forcing a binary where an honest third state exists is what makes people fudge.*

## ROWS VERIFIED, NOT TAKEN ON REPORT

**ROW 5 IS MET, AND THE DAG PROVES IT:** `git merge-base --is-ancestor 9587d2f 35dc5ae` → **true.** The
205-site census is an **ancestor** of this gate ⇒ the blast radius was measured **before the gate
required it and before any remedy was designed**, and it **killed a candidate rather than justifying
one.** ⭐ *Ancestry is checkable without taking anyone's word — the best kind of evidence for an ordering
claim.*

**ROW 7 DISCHARGED** by the S3e hunt. **ROW 9 UPGRADED FROM ASSERTION TO MEASUREMENT:** "present in
every released version" will be **measured** by pulling each released `.crate` and reading
`.cargo_vcs_info.json` (Seth's method — curl+tar, no cargo, no build), carrying his trap: **a subject
line is not a release.** **Rows 1, 3, 6, 8 stand as written.**

---

# ⚠️ POST-ARTEFACT AMENDMENT 3 — THE ACCEPTANCE PASS ON ROWS 2, 4 AND 10. Cally Ray, 2026-08-07

**No clock in this header, deliberately.** Amendments 1 and 2 both fabricated one and amendment 2's
correction fabricated a second. ⭐ **A FIELD THE CARRIER ALREADY RECORDS AUTHORITATIVELY MUST NOT BE
COPIED INTO THE PAYLOAD — a copy is a second source, and a second source can disagree.** The instant of
this amendment is its commit's, readable with `git log --date=iso-strict-local`. This is the previous
rule's tier-1 form: not *cite the commit more carefully*, but **do not duplicate the field at all.**

**Method:** graded at `3ba1096`, working tree reconciled to it (`git status --short` = one untracked
`.claude/skills/`, nothing tracked modified). `shape_hunt.py` **run**, not read: rc=0, control PASS,
32 hits (S3a 8 · S3b 11 · S3d 4 · S3e 9). Amendment 2's premise re-derived rather than taken from my
own note — the control does key on `code_management_bifs.rs:148`, live, today.

## 🔴 R10-a — AMENDMENT 2'S CURE RE-OPENS AMENDMENT 2'S OWN DEFECT, ONE LEVEL DOWN

Amendment 2 ruled: *control points at a committed synthetic fixture **the lane will never repair**.*
**That escape does not hold, and "never repair" is why I missed it.**

A synthetic fixture is **a written instance of the defective shape**. The pre-registered remedy
criterion is *a remedy is IN only if the shape **CANNOT BE WRITTEN***. Therefore:

- remedy succeeds structurally ⇒ the shape is unwritable ⇒ **the fixture stops compiling** ⇒ control dies;
- fixture survives ⇒ **the shape is still writable** ⇒ the remedy failed its own criterion.

⭐⭐ **A CONTROL KEYED ON A LIVE DEFECT IS DESTROYED BY THE PATCH; A CONTROL KEYED ON THE SHAPE IS
DESTROYED BY THE REMEDY WORKING. The second is worse, because it fires on SUCCESS.**
⭐ **AN EXEMPTION BY INTENT CANNOT PRESERVE A CONSTRUCT THE COMPILER REFUSES** — "we won't repair it" is
a promise about attention, and a structural remedy does not consult attention.

Tested by negation: under a patches-plus-lint remedy the fixture survives, so the hazard is not a
correlate of fixture-ness — it attaches **precisely to the remedy shape this gate prefers.**

### THE RESOLUTION — MATCH THE CONTROL'S FAULT DOMAIN TO THE INSTRUMENT'S

`shape_hunt.py` is **purely syntactic**: `ALLOC` is a regex over method-name spelling; it resolves no
type and consults no API. ⇒ ⭐ **A SYNTACTIC INSTRUMENT TAKES A SYNTACTIC FIXTURE.** The control's target
must depend only on what the instrument reads (bind syntax + spelling) and on **none** of the semantics
a remedy changes. Four constraints, all derived from the instrument's code rather than from taste:

1. **A local type with its own `alloc_cons`, in a `#[cfg(test)]` module inside a NON-test-named `.rs`
   file under `crates/`.** It matches `\.alloc_cons\s*\(` and S3d's `^\s*\w+\s*=` with no `let`; it
   touches no real `Term`/`Context`, so a type-level remedy cannot reach it; `cfg(test)` keeps it out
   of the shipped binary while the regex — which does not evaluate `cfg` — still sees it.
2. **Inside the walked population.** `source_files()` walks `pathlib.Path("crates").rglob("*.rs")` and
   `continue`s on `tests.rs`, `*_tests.rs` and `src/native/context/`. ⚠️ **The instinctive placement — a
   test file — is the one place the instrument is blind.** *A control drawn from outside the searched
   population validates the instrument and not the search.*
3. **Named in the ledger with a permanent `CONTROL-FIXTURE` disposition, and reported in its own section
   rather than filtered silently** — otherwise it is a decoy every future sweep flags and someone
   eventually "fixes", killing the control by the long route. Exclusion is by **exact literal path**:
   denominator one, auditable, cannot swallow a real site.
4. **Both defeat arms covered, because a guard written for one arm is invisible to "is there a guard
   here?"** Type-level remedy → defeated by (1). **Lint-on-spelling remedy → defeated by an explicit
   `#[allow]` at the fixture**, which the regex also does not read.

⚠️ **DECLARED COUPLING, or it rots silently:** this fixture works *because* the hunt is regex-based and
blind to `cfg` and attributes. That is a deliberate use of the instrument's blind spot, and it must be
stated in `shape_hunt.py` itself — **if the hunt ever becomes AST-based, the control must be re-sited.**
An undeclared dependence on a blind spot breaks the day someone improves the instrument.

## 🔴 R10-b — THE CONTROL VALIDATES ONE SHAPE CLASS OF FIVE. S3c IS AN UNVALIDATED ZERO RIGHT NOW.

`ctl = [h for h in hits['S3d'] if 'code_management_bifs' in str(h[0])]` — **S3d only.** S3a, S3b, S3c
and S3e are wholly uncontrolled, and **S3e is the class that found the new defects (14 → 17).**

⇒ the `TUPVEC` regex could break and the run still prints **PASS**. At fix time the reading on offer
would be "S3e sites were structurally eliminated — disposition (b)", and ⭐ **a silently-broken S3e
regex and a successful structural elimination produce the same output.** That is row 2's original
defect, migrated intact into R10.

⛔ **And S3c returns nothing today — no section is printed at all.** It has never been shown to produce
a presence, so by this gate's own row-2 law *(an absence is only evidence when the instrument has been
shown, that run, to be able to produce a presence)* **every S3c zero this instrument has ever reported
is uninterpretable.** Not "probably fine": unmeasured.

⇒ **one fixture per shape class, five controls, each exiting non-zero on its own failure.** A single
aggregate PASS/FAIL is the loud-variant check that covers nothing of the silent one.

## 🔴 ROW 2 — THE LEDGER IT DEPENDS ON DOES NOT EXIST

Amendment 2 condition 1 requires row 2 and R10 to **share a single disposition ledger.** Measured at
`3ba1096`: `evidence/accumulator-rooting/` holds `shape_hunt.py`, `sink_census.py`,
`released_class_presence.py`, `sink-census.json`, `released-class-presence.json` — **and no ledger.**
`sweep/verdicts.json` is RF-006's, a different population.

⇒ **the seventeen are not written down by name anywhere.** Left as-is, the ledger gets built at fix time
**from the post-fix state**, which is the pinned-count defect one level down: an accounting derived from
the thing it audits. ⛔ **The ledger is pre-registration, not reporting: it commits BEFORE the fix, as
this gate did**, with all seventeen named, plus the five `CONTROL-FIXTURE` rows.

## 🔴 ROW 4 — "RED AT ITS OWN PARENT COMMIT" SILENTLY REQUIRES A COMMIT GRANULARITY NOBODY HAS STATED

Row 4 grades each site against **its own parent commit.** If the lane lands as one commit for seventeen
sites, "its own parent" is undefined for sixteen of them and the row is unmeetable — not refused,
**unmeetable, which is the failure mode that reads as satisfied.** ⇒ **the lane commits per site (or per
site-group carrying its own test), and that is a requirement of row 4, stated here rather than
discovered at landing.**

The wasm leg is **priced** (Amendment 6). Row 4's honest third state still stands for sites **12 and
16**, which have no demonstrated red: unpriced-in-fact ⇒ `FIXED-UNVERIFIED` ⇒ sign-off blocks. No
class-level test may stand in for them, and **priced-by-analogy is not priced.**

## ROWS 5, 7, 9 — DISCHARGE READ CONFIRMED AT MY HANDS

Row 5 `merge-base --is-ancestor 9587d2f 35dc5ae` true; row 7 discharged by the S3e hunt (14 → 17);
row 9's 56/56 measured. Nothing owed by Artemis on AR-1.

## ⚠️ FOR ROW 6, NOT ROW 10 — ITS NAMED DEFEAT IS ALREADY IN THE TREE

S3d surfaces `native/udp_bifs.rs:473` — `tail = context.alloc_cons(*term, tail).unwrap_or(Term::NIL);`
— which is **verbatim the defeat row 6 predicted before anyone looked** ("an `unwrap_or(term)` … will
look like defensive programming"). Recorded against row 6; not absorbed here.
🔴 **RETRACTED BY AMENDMENT 4 — THIS SIGHTING IS FALSE. The line is inside `#[cfg(test)] mod tests`.**

---

# ⚠️ POST-ARTEFACT AMENDMENT 4 — I FILED A FALSE POSITIVE AGAINST ROW 6. Cally Ray

*(No clock: the commit records it. See amendment 3's rule.)*

## 🔴 THE RETRACTION

`native/udp_bifs.rs:473` sits inside `#[cfg(test)] mod tests`. **Row 6's predicted defeat has NOT
occurred; it remains unfired.** Refuted by Artemis with three differently-shaped proofs — the only
column-0 `}` after `:380` is `:579` (EOF); a depth-aware walk puts minimum depth across `:384–:472` at
**1**, never closing, with `:473` at **depth 3**; and `mod tests {` is the only column-0 item
declaration after `:382`. She deliberately did **not** settle it with a column-0 brace check, because
that is a sibling of the instrument that produced the sighting.

⛔ **AND THE DISCLOSURE PREDATED MY GRADING.** `AR-1-GROUND-PACK.md` at `9587d2f`, section *"Row 6's
shape appears — in TEST code only"*, names `:466-476` and the `cfg(test)` opener at `:382`.

⭐⭐ **MY ERROR'S SHAPE: I GRADED RAW INSTRUMENT OUTPUT AS THOUGH IT WERE A FINDING, WHEN A CLASSIFIED
READING OF THAT SAME OUTPUT ALREADY EXISTED AND PREDATED ME.** Raw output is not evidence — it is the
input to a classification someone had already done and banked. Re-deriving it from the hit list alone
reproduced that work worse **and aimed a false positive at a colleague's lane.** The instrument was in
front of me; the ground pack was one file away, cited in this gate's own subject line.

## 🔴 AMENDMENT 3'S FIGURES ARE CORRECTED

**32 is the RAW hit count; production is 27.** Five hits are `#[cfg(test)]`:
`jit/runtime_binary_match.rs:579`, `:627`, `jit/runtime_map.rs:221`, `native/ets_bifs.rs:1360`,
`native/udp_bifs.rs:473`.

| class | production | `cfg(test)` |
|---|---|---|
| S3a | **4** (amendment 3 said 8) | 4 |
| S3b | 11 | 0 |
| S3c | **0 — still uncontrolled, still uninterpretable (R10-b stands)** | 0 |
| S3d | **3** (amendment 3 said 4) | 1 |
| S3e | 9 | 0 |

✅ **THE SEVENTEEN DO NOT MOVE.** None of the five is a class member — `udp_bifs`' member is
`finish_udp_recv/ip`, a different site in the same file. Row 2's population is unchanged at 17.

## ⭐⭐ THE CONSEQUENCE FOR R10-a, AND IT IS ARTEMIS' FINDING

**My fixture design makes the hunt's `cfg`-blindness LOAD-BEARING** — it is precisely why a
`#[cfg(test)]` fixture is visible to the instrument yet absent from the shipped binary. **And that same
blindness produced this false positive within the hour.** One property, two roles, opposite signs.

⇒ **the instrument cannot distinguish its own control from a real site**, so a reader of the post-fix
run sees N hits with no way to separate fixture / test-helper / production **from the very output that
is supposed to certify the remedy.** That is row 2's defect arriving a third time by a third door.

⭐⭐ **A DELIBERATE USE OF A BLIND SPOT CREATES AN OBLIGATION TO LABEL EVERYTHING THAT BLIND SPOT HIDES.
You have converted an accident into a dependency, and everyone downstream inherits it without being
told.** The fixture is only sound if it ships with the labelling.

**RULED — LABEL, NEVER FILTER.** `shape_hunt.py` gains a `cfg(test)` column on every hit; counts do not
change beyond the labels; the five are named. ⛔ **Filtering is wrong twice over: it blinds the hunt in
the direction that hurts, and it would hide the control fixture.** A docstring caveat fires at 0%; a
column fires at every lookup. **The script is Artemis'; the change is hers to land.**

---

# ⭐ AMENDMENT 5 — R10 BUILT (NOT YET DISCHARGED), ROW 2'S LEDGER EXISTS. Cally Ray; time from the commit carrying this note.

> ⛔ **R10 IS NOT DISCHARGED AND THIS AMENDMENT FIRST SAID IT WAS.** The fixtures are Rust in a shipping
> crate and **have not compiled** — the box gate refused every cargo leg (1-min 48.63 / 5-min 37.07
> against 10 cores at entry; disk 41 GiB, above the band, so the refusal is load-only). Everything below
> is graded **on text**. ⭐ **A ROW GRADED BY THE INSTRUMENT IT SATISFIES IS NOT GRADED: the hunt reads
> source, so it cannot tell a fixture that compiles from one that does not**, and marking the row green
> off a green hunt would have been this gate's own row-2 defect, committed by its author, in its own
> status table. R10 closes when `fmt` + `clippy -D warnings` + the fixture's `#[test]` are green and the
> hunt still finds five.

**Both rows moved because the work was done, not because the bar moved.** Nothing below relaxes a
criterion; two of the entries are defects in my own hands, found by building the thing.

## R10 — the five fixtures exist, and each one's death is caught by its own control

`crates/beamr/src/ar1_shape_control.rs`, one fixture per class S3a–S3e, built to amendment 3's four
constraints. `shape_hunt.py`'s single S3d control on RF-006 defect #1 is **retired** — a control keyed
on a live defect is destroyed by the patch — and replaced by five, each exiting non-zero on its own.

**Shown red, not assumed green.** Five realistic deaths (fmt reflow, refactor-to-call, hoisted alloc,
`let` on the line, `Vec::default`), **each caught ONE-TO-ONE**, M0-unmutated green first so a stuck-red
harness could not credit a catch. The **denominator control** was shown red on both arms: a one-file
walk, and the fixture renamed to `*_tests.rs`.

## 🔴 R10-c — MY OWN FIXTURE CARRIED THE DEFECT THE ROW EXISTS TO CATCH

The S3e fixture's `.map(..).collect()` tail is **also an S3a hit**; the S3b `match` arm is **also an
S3d hit**. Keyed the obvious way — *any hit of this class in the fixture file* — ⛔ **EITHER WOULD HAVE
HELD ITS NEIGHBOUR'S CLASS GREEN AFTER THAT NEIGHBOUR'S FIXTURE BROKE.**

⭐⭐ **FIVE CONTROLS IN ONE FILE ARE NOT FIVE CONTROLS UNTIL EACH IS KEYED TO SOMETHING ONLY ITS OWN
FIXTURE HAS.** Co-location is the hazard: it is what makes substitution possible and invisible at once.
⇒ keyed on the **binding name** (`s3a_mapped` … `s3e_pairs`) — a *name*, not a trailing comment, so no
reflow can strip it — **plus a uniqueness assert**: 0 or ≥2 matches is UNUSABLE, never PASS.

## Row 2 — `evidence/accumulator-rooting/dispositions.json`, committed BEFORE the fix

**22 rows: the 17 by name + the 5 `CONTROL-FIXTURE`.** Numbering reconciles with the ground pack
(#11 `term/json.rs`, #12/#13 `convert.rs`, #14 Osiris'). Every crossing is **PENDING** by construction.
**No clock in the file** — it takes its time from its commit, per the rule this gate's own header
correction laid down.

`ledger_check.py` makes the disposition machine-verified rather than asserted (amendment 2, condition
2). ⚠️ Its `STRUCTURALLY-ELIMINATED` arm has **zero rows to act on today**, so it is forced in the
self-test **in both directions** — a claim true at the bytes must PASS, a refuted one must FAIL. ⭐ **A
CHECK WITH NOTHING TO ACT ON SHIPS AS PROSE IN A SCRIPT'S CLOTHING UNLESS SOMETHING MAKES IT RUN.**
`--sign-off` refuses all 17 PENDING today, rc 2.

**Row 4 feeds off the same ledger:** ids **12, 13, 16, 17** now carry the **priced** leg, and 13/17
additionally carry `RED-AT-PARENT DEMONSTRATED at 308b448` while 12/16 carry an explicit
`NOT DEMONSTRATED`. That is written down per site rather than described once in prose — which is what
kept 12 and 16 from inheriting a proof that was never run for them.

## 🔴 THE LABELLER LEAKED, AND ITS WHOLE ERROR BUDGET POINTED AT CONCEALMENT

`#[cfg(test)] mod tests;` owns no block; the arming flag reached forward to the next `{`, up to **647
lines** away. **9 files, 40 lines mislabelled, 40/40 reporting production as `cfg(test)`** — none the
safe way. **0 disagreements across the 32 hits**, so the `32 raw · 27 production · 5 cfg(test)` split
stands. Fixed at `f77719c` with arm I; re-derived independently by Artemis from committed bytes.

⚠️ **MY CENSUS OF THE DEFECT WAS WRONG: I said 81, it is 71.** My predicate asked *is there a brace on
the next line*; the defect's predicate is *does the item terminate at `;` before any brace*. ⭐ **A
DENOMINATOR OVER-REPORT DOES NOT FAIL SAFE THE WAY A SPELLING OVER-REPORT DOES — it inflates the size
of a defect.** It survived my reading because **the count was decoration on a finding that stood
without it**: every consequence was measured on labels, none derived from the census.

## S3d's CLASS ATTRIBUTION over-reports — recorded, no row re-graded

The S3d regex reads `match` arms (`None => context.alloc_tuple(..)`) as reassignments: **3 of its 6
hits, 2 of them production.** The hits are real carrier shapes; **the class label is wrong, not the
finding**, and the direction is fail-safe. The 17 were verified by reading, so no verdict moves.

## ⭐⭐ THE CROSS-FILE BLINDNESS — Artemis' finding, and the reason R10 is load-bearing

`cfg_test_lines()` walks one file at a time, so a gate declared in a parent (`#[cfg(test)] mod x;` in
`lib.rs`) leaves the module's own file with **no attribute and every hit labelled `prod`**. It has been
inert for its entire life **not because the labeller handles it**, but because every gate of that shape
in the tree points at a `tests.rs`/`*_tests.rs` file `source_files()` already skips. **Two unrelated
mechanisms were covering it and neither was written to.**

⇒ ⭐⭐ **A BLIND SPOT HELD SHUT BY A CONVENTION IS NOT CLOSED — IT IS UNTESTED, AND IT OPENS THE FIRST
TIME SOMEONE HAS A REASON TO BREAK THE CONVENTION.** R10 is that reason: the fixture must sit outside
the skip list *on purpose*, so it is the first artefact where the labeller carries that weight alone.
The fixture is sited in-file, which is correct — **but I chose that reasoning about `source_files()`
and had not considered cross-file gating at all. Right answer, incomplete reason; recorded as the
coincidence it was rather than the decision it looks like.**

---

## AMENDMENT 6 — ROW 4: the wasm leg, priced (Cally, at `308b448`)

Full working: `evidence/accumulator-rooting/row4_wasm_leg_pricing.md`.
Artefacts: `row4_probe.rs.txt` (source), `row4_red_at_parent.txt` (transcript + hashes + load).

### The premise was false, and cheaply checkable

Row 4 called this **"a build leg nobody has costed."** Nobody had *looked*. The wasm32 target is
installed and **pinned in `rust-toolchain.toml`**; `wasm-bindgen-test-runner` is on PATH at 0.2.123,
**matching** `Cargo.lock`; `wasm-bindgen-test` is already a dev-dependency; node is v26.4.0; and the
leg is a **named gate — `gates.json:7`, `wasm-tests`** — with the runner env var set, a version check
prepended, and a CI workflow. **Setup cost: zero.**

**Run cost: 28.19 s** for the full 80-test suite, `wasm32-check` a further 7.79 s. Taken under
**declared contention** (10 cores, 1-min load 10.00–14.15), which **inflates** wall-clock — so these
are **upper bounds**, the safe direction for a pricing decision. Warm-cache; **no cold figure was
measured and none may be quoted.**

### ⛔ The expensive part was never the toolchain

**All 80 existing wasm tests are structurally incapable of failing on this class**, for **two
independent reasons** — fixing either alone would not have helped:

1. **No process ⇒ no collection at ANY input size.** Every test builds its context with
   `ProcessContext::new()`, which sets `process: None` (`native/context/mod.rs:514`), and
   `ensure_heap_space` (`:788`) early-returns `Ok(())` without a process. `gc::ensure_space` is never
   reached. This is not "the tests are too small."
2. **Immediates ⇒ no allocation between bind and use.** My first probe used 400 **small integers** and
   went **green**: an immediate needs no allocation, so the accumulator is never live across one.

⭐ **THAT GREEN WAS MINE, AND IT LOOKED EXACTLY LIKE A PASS.** A test can name the right function, hit
the right line, allocate the right cells, and still never exercise the mechanism it claims to cover.
Both probes therefore carry a **positive control** asserting the heap actually grew — without it a
green cannot distinguish *"the accumulator survived a move"* from *"no move ever happened."*

### Red-at-parent: DEMONSTRATED for 13 and 17, NOT for 12 and 16

* **Site 13** (`array_to_term`, `tail`): control passed, then
  `convert.rs:383:40 — converted JavaScript array is a proper list`. The list is no longer proper.
* **Site 17** (`value_to_term` object arm, `pairs`): fails **at the conversion** —
  `failed to allocate map term` — which alone proves nothing, since it could be an allocator limit.
  Separated by a **two-armed control**: at **5 entries** the control reports `heap never grew
  (466 -> 466)` and **the conversion SUCCEEDS**; at **60 and 200** it collects and **fails**.
  ⭐ **THE FAILURE IS COLLECTION-DEPENDENT, AND THAT IS WHAT MAKES IT ATTRIBUTABLE.** The size arm was
  the claim; the *no-collection* arm is the measurement.
* **Sites 12 and 16** reach the same carriers through `serde_json::Value`. **No probe was run for
  them. They are recorded `NOT DEMONSTRATED` and stay sign-blocking.** ⭐ **A SAMPLE OF TWO IS NOT A
  CLASS, AND PRICED-BY-ANALOGY IS NOT PRICED.**

### Status and what is deliberately NOT claimed

Row 4's sign-blocking condition is **discharged for 13 and 17 only**. The probes are **banked, not
landed** — they are red at HEAD, so landing them now would break the branch; they land **with the
fix**, and per-site commit granularity still applies. The tree was restored and re-verified: **80
passed, rc 0.**

⭐ **A GATE THAT RUNS, PASSES, AND IS WIRED INTO CI CAN STILL BE BLIND TO THE ENTIRE DEFECT CLASS IT
APPEARS TO COVER.** The wasm leg was never the missing piece. The missing piece was a context with a
process attached — and nothing in the row, the gate, or the ledger had noticed the difference existed.
