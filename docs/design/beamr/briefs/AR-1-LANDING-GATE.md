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
the risk is explicitly ruled accepted.** ⛔ **The one escape that stays forbidden is a class-level test
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
