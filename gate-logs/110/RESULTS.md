# AR-1 ROW 4 — per-site verdicts

Artemis Peach. Bytes under test: `f993280` (`origin/main`), production unmodified
except where an arm says otherwise. Every arm below was run at my own hands; each
mutation was reverted and the revert re-verified before the next arm.

**Verdict vocabulary.** `RED-DEMONSTRATED` = the defect fires on production
bytes. `DEFENDED` = the flagged shape is present, a probe proven able to see the
defect at that site was built, and on production bytes it stays green. A
`DEFENDED` verdict is only admissible with a positive-control arm that goes RED —
otherwise it is indistinguishable from a probe that never exercised anything.

---

## Site 4 — `native/dictionary_bifs.rs::entries_to_list` — **DEFENDED**

Carrier `tuples` (`Vec<Term>`), sink `alloc_tuple` then terminal `alloc_list`.
Driven through the real BIF (`erlang:get/0`), not the private helper.

Probe: `ar1_site4_accumulator_survives_a_collecting_get_0`, 200 entries × 24-byte
**binary** values (never immediates — an immediate needs no allocation, so the
accumulator would never be live across one). Read back by binary CONTENTS, never
by `Term` identity: a `Vec<Term>` held by the test would go stale exactly as the
production accumulator does.

**CONTROL 1 — pressure.** At the call, the nursery holds **22 words** against the
**1000** the result needs, so `bif_get_0`'s reserve is unambiguously what makes
room. Measured, not assumed: the first attempt at 40 entries left 312 words free
and **CONTROL 1 correctly refused the probe** rather than reporting a green.

### Four arms

| arm | `entries_heap_words` | verdict | what it establishes |
|---|---|---|---|
| **A** | `entry_count * 5` (production) | **GREEN** | no corruption under deep pressure |
| **B** | reserve call deleted | **RED** | the probe CAN see this defect at this site |
| **C** | `entry_count * 5 - 1` | **GREEN** | one word short is absorbed |
| **D** | `entry_count * 2` | **RED** | the **3N tuple component** is what defends |

Arms B and D fail identically: `Tuple::new(term)` returns `None` — an element of
the result list is no longer a tuple. That is the stale accumulator, observed.

### Why the site is flagged and still not broken

The caller prereserves **before** copying the entries out, while the process
dictionary still roots them:

```rust
let entry_count = context.dict_len()?;
context.ensure_heap_space(entries_heap_words(entry_count))?;   // entry_count * 5
let entries = context.dict_get_all()?;
entries_to_list(&entries, context)
```

`5` is exact per entry: a 2-tuple costs `1 + 2` = 3 words, its cons in the
terminal `alloc_list` costs 2. With 5N reserved, the internal
`ensure_heap_space` inside `alloc_tuple` never sees `available() < words`, so the
accumulation loop cannot collect.

⭐ **ROW 1'S DETECTOR IS ORDERING-SENSITIVE BY DESIGN AND CANNOT SEE A CALLER'S
PRERESERVE.** Its `REAL` verdict says *the shape is present*, not *the defect
fires*. Separating those two is exactly what row 4 is for.

### ⭐ A refinement §6's proposed disposition class needs

Row 3's wall is *subtract one word and a test must red*. **Arm C shows that wall
would be the WRONG instrument here** — one word short is absorbed, because the
shortfall lands at the terminal `alloc_list`, which roots its own elements. Arm D
shows what is actually load-bearing: **coverage of the accumulation loop**.

⇒ **For an accumulate-then-sink site the wall must test COVERAGE OF THE
ACCUMULATION, not EXACTNESS OF THE TOTAL.** Row 3's −1 wall is right for an
S2 pre-existing-inputs site and wrong for this shape. Offered to Cally for §6.

### Adjacent, disclosed, NOT touched

`ensure_space` also collects on `virtual_binary_pressure_exceeds_heap`, which a
word-count reserve does not account for — a real defeat path for any prereserve.
It is unreachable in practice: **`increase_virtual_binary_heap` is called from
exactly one place in the tree, `gc/tests.rs:86`.** Nothing in production drives
it, so the virtual-binary-pressure accounting appears inert. Not this lane's to
fix; named rather than left silent.

### Sibling test that has been green the whole time and proves nothing (site 4)

`get_0_returns_complete_dictionary_as_tuple_list` drives this same site with
`Term::atom` / `Term::small_int` — all immediates. It names the right function,
hits the right line, and is **structurally incapable of failing on this class.**
Cally's Amendment 6 lesson, found again native-side.

---

## Sites 8 + 9 — `native/stdlib_stubs/uri_bifs.rs::bif_uri_string_dissect_query` — **RED-DEMONSTRATED**

Site 8 carrier `terms` (`Vec<Term>` of tuples, :127); site 9 carrier `key` (:129,
the freshly-allocated key binary live across the value's `alloc_binary` at :131
and the `alloc_tuple` at :134). One function, two carriers, **no prereserve
anywhere in the call chain** — verified by reading the whole enclosing function
and its callers, not by grep alone.

Chosen because it is a **pure function of a string argument**: the input size is
mine to set exactly, with no process-global state. (Site 6, `bif_os_getenv_0`,
has the same unprotected shape but is driven by `std::env::vars()`, so a probe
would mutate process-global state shared with tests running in parallel —
rejected on that ground, not on difficulty.)

Probe: `gate-logs/110/probe_sites_8_9.rs.txt`, appended to `uri_bifs.rs` for the
run and reverted afterwards (`git diff` empty at the end). `pairs` key=value
pairs, both sides 13 bytes so both are heap-allocated; the result is read back by
**contents**, never by `Term` identity.

### The measured surface — heap × input

| heap (words) | 3 | 50 | 200 | 400 |
|---:|---|---|---|---|
| 64 | ok | *error term* | *error term* | *error term* |
| 256 | ok | **not a tuple** | *error term* | *error term* |
| 1024 | ok | ok | **key contents `k000000000186` != `k000000000000`** | *error term* |
| 4096 | ok | ok | ok | **not a tuple** |
| 16384 | ok | ok | ok | ok |
| 65536 | ok | ok | ok | ok |

⭐ **THE 1024/200 CELL IS THE WHOLE FINDING IN ONE LINE.** Element 0's key reads
back as **pair 186's** key. That is not a truncation, not an allocator refusal,
and not a type confusion — it is a pointer into the old nursery being read after
the collector moved the object, landing on whatever was copied there instead.

### Why it is attributable, and not an allocator limit

Cally's site-17 warning applies exactly: a failure *at* the conversion proves
nothing on its own, because it could be the allocator refusing. The
disambiguation is the same one she used, and the table above supplies it in two
independent directions:

* **Hold the heap, grow the input** — at 4096 words, 200 pairs completes cleanly
  and 400 corrupts. The allocator was not at its limit at 200.
* **Hold the input, grow the heap** — at 400 pairs, 4096 corrupts and 16384 is
  clean. Remove the need to collect and the defect disappears.

**The failure is collection-dependent in both axes.** The landed arm form holds
the heap constant at 4096 and moves only the input, so the single variable is
whether the accumulation outruns the nursery.

⛔ The `*error term*` cells are NOT counted as evidence. They are ambiguous by
construction and are recorded, not relied on.

---

## ✅ CLEAN RE-RUN — BOTH VERDICTS NOW FINAL, PRE-REGISTRATION MET EXACTLY

Box confirmed free at two independent instruments before a byte was compiled:
mine (zero `cargo`/`rustc` in the process table, CPU 57.5% idle, load
5.27/6.88/7.92) and Waffles' (`gate.sh` pid 79363 gone, `pgrep -fl gate.sh`
empty, load 6.61/7.32/8.13) — **different seats, different commands, same decay
curve.**

Re-run against `PROBE-PLAN.md` Part 1, which was written **before** the box
freed. Every pre-declared value reproduced:

| axis | pre-declared | observed clean | |
|---|---|---|---|
| site 4 arm A (production `* 5`) | GREEN | GREEN | ✅ |
| site 4 arm B (reserve deleted) | RED | RED | ✅ |
| site 4 arm C (`* 5 - 1`) | GREEN | GREEN | ✅ |
| site 4 arm D (`* 2`) | RED | RED (`dictionary entry tuple`) | ✅ |
| site 4 CONTROL 1 pressure | **22 words vs 1000** | **22 words vs 1000** | ✅ |
| 8/9 heap 4096 × 200 | ok | ok | ✅ |
| 8/9 heap 4096 × 400 | RED "not a tuple" | RED "not a tuple" | ✅ |
| 8/9 heap 1024 × 200 | RED **pair 186** | RED **`k000000000186`** | ✅ |
| 8/9 heap 16384, all sizes | ok | ok | ✅ |

All 24 sweep cells reproduced identically, not merely the four named ones.

⭐ **The exactness is what carries.** A word count and a pair index are integers,
not timings — had contention been a factor the *numbers* would have moved, not
just the pass/fail. **Contention is now excluded by measurement rather than by my
argument for why it could not matter**, which is the difference the provisional
hold existed to buy. The control reading is now emitted by the probe rather than
only asserted, so the integer is legible on a green run instead of only on a red.

⇒ **Site 4 = DEFENDED (final). Sites 8+9 = RED-DEMONSTRATED (final).**

---

## Sites 11 + 15 — `term/json.rs` — **both RED-DEMONSTRATED**

Site 11 carrier `tail` (threaded through `alloc_cons` in `array_to_list_term`);
site 15 carrier `pairs` (`Vec<(Term, Term)>` in `object_to_map_term`). Both
driven through the public `value_to_term`. Probe banked at
`gate-logs/110/probe_sites_11_15.rs.txt`; `json.rs` reverted byte-identical
after the run.

| site | control (same heap) | RED cell | second direction | verdict |
|---|---|---|---|---|
| 11 | 4096 w × 50 → ok | **4096 w × 2000 → `element 635: head is not a binary`** | 2000 on a 1 MiW heap → ok | RED |
| 15 | 256 w × 10 → ok | **256 w × 100 → `entry 16: key is not a binary`** | 100 on a 1 MiW heap → ok | RED |

Both are attributable in **both** axes — hold the heap and grow the input, hold
the input and grow the heap — so neither red is an allocator limit.

### ⚠️ SITE 15'S FIRST RED WAS INADMISSIBLE AND I THREW IT OUT

My first site-15 arm (heap 4096 × 2000) returned `failed to allocate map term`.
That is an error **at** the conversion — Cally's site-17 warning exactly — and it
is ambiguous by construction: it could be the allocator refusing. I swept for the
regime where **construction SUCCEEDS and the result is still wrong**, and that is
the cell recorded above. The refusal cells are recorded and **not counted**.

### ⭐⭐ ESCALATION — AT SITE 15 THIS CLASS IS A CRASH, NOT A WRONG VALUE

Attribution run, both halves named so the failure could not be blamed on "the
round trip":

```
STEP 1 exit:  value_to_term -> Ok(term)        <- construction SUCCEEDED
STEP 2 enter: term_to_value (production traversal)
              fatal runtime error: stack overflow, aborting   (SIGABRT)
```

`value_to_term` and `term_to_value` are **both `pub fn` production entry points**.
At heap 256 × 100 entries a JSON object is constructed "successfully" and then
**aborts the process when read back**.

⭐ **THE MECHANISM GENERALISES BEYOND THIS SITE: a stale pointer can land on an
ENCLOSING object, which turns a tree into a CYCLE. Every recursive term walker in
the tree then becomes an unrecoverable stack-overflow abort** — not a wrong
value, not a catchable error. `term_to_value` is one such walker; whether
`format`, `hash` and `compare` are others is **adjacent and NOT chased here** —
named so it is not mistaken for cleared.

⇒ This is disposition-relevant for Cally: AR-1's severity is **not uniform across
sites**. A remedy priced on "corrupted output" underprices any site whose
corruption can be cyclic.

### ⛔ ERRATUM — "THESE TWO SITES ARE NOT REACHED BY THE CANON `tests` LEG" IS **REFUTED**. THEY ARE REACHED.

**The claim below was wrong and is struck. Correction measured 2026-08-17 at
`f993280` + this lane's uncommitted probe, on Themis Lamington's challenge.**

~~`term::json` is `#[cfg(feature = "json")]` and `json` is not in beamr's default
features, so only `tests-all-features` reaches sites 11 and 15. A red demonstrated
here is invisible to the default test leg.~~

**What is true.** `term::json` is indeed `#[cfg(feature = "json")]` and `json` is
indeed not a default feature (`default = ["std","threads","net","fs","jit",
"embedded","readiness"]`). **Both premises hold and the conclusion still does not
follow.** The workspace is `resolver = "2"`, `crates/beamr-wasm` is a member, and
it depends on beamr with `features = ["cooperative", "json"]`. Resolver 2 unifies
features across packages built for the same target in one invocation — so the
canon leg, which is **workspace**-scoped, gets `json` **on** through that
dependency.

**Measured, two-sided, both arms from the same log-producing instrument:**

| invocation | `term::json` tests enumerated | total listed |
| --- | --- | --- |
| canon leg 5 — `cargo test --workspace --features beamr/encode -- --list` | **14** | 2129 |
| control arm — `cargo test -p beamr --features encode -- --list` | **0** | 2080 |

The control arm is what makes the 14 admissible: the instrument is shown able to
report **absence** as well as presence, so the positive reading is not just a grep
that always matches. Denominator reconciles exactly — 2129 = the 2128 recorded at
`58dd949` **plus one**, the site-4 probe test still uncommitted in this tree.

**⭐ HOW I GOT IT WRONG, because the mechanism is the transferable part.** I ran
`cargo test -p beamr`, saw `0 tests`, and concluded *the feature is off in the
gate*. What I had actually measured is that the feature is off **in a
crate-scoped invocation** — and the canon leg is not crate-scoped. **The same
feature is present or absent depending on INVOCATION SCOPE, and both readings are
correct about the command that produced them.** That is a nastier trap than a
plain non-default feature: a crate-scoped probe and a workspace-scoped gate
compile different feature sets of the same crate and disagree about what exists,
with neither one lying. ⇒ **A FEATURE-REACH CLAIM ABOUT A GATE MUST BE MEASURED
WITH THE GATE'S OWN INVOCATION, NOT A CONVENIENT SMALLER ONE.** Gating the user's
actual command, again.

**What this does to the finding — it sharpens it, and it settles which remedy is
owed.** Sites 11 and 15 are reached by *every* canon `tests` run, their own module
carries 14 tests, those tests call the defective functions, they pass, and the
defects shipped anyway. So the hole is **not gate reach** and no gate remedy is
owed here. See the coverage law below.

### ⭐⭐ COVERAGE MEASURES WHETHER A LINE RAN, NOT WHETHER IT RAN UNDER THE CONDITION THAT BREAKS IT

The two tests that actually drive the defective sites are
`value_to_term_converts_arrays_to_proper_lists` on `json!([1, 2, 3])` (site 11)
and `value_to_term_converts_objects_to_binary_keyed_maps` on
`json!({"key": "value"})` (site 15) — **three immediates and one pair, both
against a 512-word heap.** They call the exact functions, assert the right
values, and pass, at inputs orders of magnitude below the point where a
collection can fire mid-accumulation.

So a line-coverage instrument would report those lines **covered**, and a
feature-reach census reports them **reached**, and the defect ships through both.
This class is gated on a **runtime condition**, not on a branch or a feature —
so every metric shaped to ask "did this code execute?" answers yes and stops.
Another instance of the frame: a mechanism whose output cannot distinguish two
states it exists to distinguish, here *"executed safely"* from *"executed, but
never under pressure"*.

⇒ The remedy is neither a new leg nor a new assertion on the existing inputs, but
an **adversarial input regime for allocation-accumulating functions** — which is
precisely what the row-4 probes in this directory are, and an argument for landing
them rather than only banking them.

---

## Site 14 — `string_bifs.rs::bif_split` — **RED-DEMONSTRATED**

Carrier `terms`, sink `alloc_binary`. Ruled mine (Osiris withdrawn ⇒ the site was
unowned, not owned-elsewhere). Probe banked at `gate-logs/110/probe_site_14.rs.txt`;
`string_bifs.rs` reverted byte-identical after the run.

| arm | cell | result |
|---|---|---|
| control (same heap, smaller input) | 1024 w × 250 parts | ok |
| **RED** | **1024 w × 300 parts** | **`part 0: head is not a binary — carrier went stale`** |
| second direction (same input, bigger heap) | 1536 w × 300 parts | ok |

The partial hardening is real but does not reach the defect: the author protected
the **input** (`binary_bytes(*input)?.to_vec()`, with a comment saying the loop
may collect) and left the **accumulator** unrooted. The comment reads as
"handled" to the next reader, which is why this site was probed rather than read.

### ⭐⭐ TWO INSTRUMENT LAWS THIS SITE PRODUCED — BOTH CAUGHT MY OWN PROBE OUT

**1. A COARSE SWEEP CAN STEP OVER A ONE-CELL CORRUPTION BAND AND THE RESULT READS
AS "DEFENDED".** My first sweep stepped 5/25/100/400 across heaps 256..65536 and
found **no corruption cell anywhere** — every cell clean or refused. That surface
is exactly what a defended site looks like. The band is 250-clean / 300-corrupt /
350-refused: one cell, sitting between my steps. ⇒ **A clean-then-refused surface
is not evidence of defence; it is evidence the sweep was too coarse.**

**2. AN "AMBIGUOUS REFUSAL" CELL CAN BE MASKING A CONFIRMED DEFECT.** I had
correctly recorded refusals as not-admissible-as-evidence. But *not evidence* is
not *nothing there*. Instrumenting the production loop (temporarily, reverted)
showed the collection **does** fire under the live unrooted carrier:

```
AR1-INSTRUMENT: collection at part 254: available 2 -> 1020
AR1-INSTRUMENT: loop finished, available Some(440)
```

The loop completes over a stale accumulator; then `alloc_list` needs 800 words
for the 400-cons spine, has 440, and refuses — **the terminal refusal hides the
corruption that already happened.** ⇒ **The refusal band is ADJACENT to the
corruption band and must be searched, not merely excluded.**

**3. AND THE ARM ITSELF WAS WRONG.** My red assertion was `red.is_err()`, which
is satisfied by `bif_split returned an error term` — the refusal. The test
**passed on the inadmissible cell** and would have been reported as a red. The
arm now requires the error NOT be the refusal class. ⇒ **A two-armed red
assertion must exclude the refusal class explicitly, or it is satisfied by
exactly the ambiguity it was built to rule out.**

### Does law 1 undermine site 4's DEFENDED verdict? No — and the reason is the rule

Law 1 says a clean surface can mean "the sweep missed the band". That is a direct
challenge to the only DEFENDED verdict in this pass, so it gets answered rather
than assumed away.

**Site 4's DEFENDED survives because it carries a positive control and site 14's
apparent defence never did.** At site 4, arm B (reserve deleted) and arm D
(`* 2`) go **RED with the same probe, at the same cell** — so the probe is proven
able to see this defect *at this site*, and the green on production bytes is
therefore about the bytes, not about the sweep. Site 14's clean surface had no
such arm behind it; nothing had demonstrated the probe could ever go red there.

⭐ **This is exactly what the `DEFENDED`-requires-a-positive-control rule was
written for, and it is the difference between the two sites.** A `DEFENDED`
verdict asserted from a clean sweep alone would have been wrong at site 14 and is
not what was claimed at site 4.

---

## ⚠️ CONTENTION DISCLOSURE — the hold these two verdicts were released from

A manifold read-path workspace battery has been running on this box since 10:30
and my `cargo test` runs overlapped it (Waffles, DM `26f33abd`, 10:46Z).

**The measured size of the overlap, not a characterisation of it:**

| condition | 1-min load |
|---|---:|
| manifold battery **+ my compiles** | **9.57** (my own `uptime`, 10:47Z) |
| manifold battery **alone**, after I stopped | **8.07** (Waffles, 10:52Z) |

⇒ **my contribution was ≈1.5 of 9.57 — roughly a fifth on top, not a doubling.**
The battery is itself driving ~8. The original framing was "two heavy builds
fighting", which was a magnitude claim made from a process list; the load
comparison that sizes it was one command away. **The numbers replace the
adjective here deliberately** — Waffles corrected his own framing and asked that
the measurement carry rather than his description of it, which is the same
instrument discipline this lane runs on.

I stopped after the site I was on; nothing of mine is running.

**My reading of the exposure, stated so it can be checked rather than trusted:**
this probe has no timing dimension — no timeout, no sleep, no wall-clock
assertion, no concurrency. Its only variables are a heap capacity I pass as a
constant, an input size I choose, and whether the collector copies. A loaded box
changes scheduling; it does not change heap arithmetic, and it cannot make
element 0's key read back as pair 186's. The surface is also **monotone and
internally consistent in both axes**, which a load artefact would not produce.

**Nonetheless the verdicts are held PROVISIONAL until re-run on a quiet box.**
A red that was produced under contention is a red I would be asking someone to
take on my reasoning about why contention could not have caused it — and the
cost of the re-run is one command. Re-run clean, then publish.

---

## SITE 14 — HANDOVER, RECORDED LOUDLY

`native/stdlib_stubs/string_bifs.rs::bif_split`, carrier `terms`, sink
`alloc_binary`. The ledger records it as *"ANOTHER SEAT'S. This row is accounted
for, not owned"*, and row 8 forbids sweeping it into this lane.

**Ruled by Waffles (DM `f476a470`, 2026-08-16 ~10:50Z): the site is MINE, on the
record.** The reasoning, kept because it is the part that generalises: row 8
forbids absorbing **another seat's live work**. Osiris is withdrawn with no
replacement, so site 14 is **unowned, not owned-elsewhere**, and the prohibition
does not reach it. What row 8 protects against is *silent* absorption — so the
transfer is taken loudly instead of quietly.

⭐ **An unexamined site inside a 17-site census is a hole, and "it belonged to a
withdrawn seat" is a reason for the hole, not a justification for it.**

Ledger consequence, **NOT yet applied**: site 14's `verification_leg` text is now
superseded and its `disposition` becomes this lane's to set. `dispositions.json`
is an in-repo artefact, so editing it is a **landing**, not a working note — it
is carried to the lane's commit rather than changed here.

**Population after the ruling:** demonstrated 13, 17 (Cally) · **mine and
undemonstrated: 13 sites** — 1–11, 14, 15 native and 12, 16 wasm.

---

## SITES NOT PROBED, AND WHY — named mechanisms, not difficulty

⭐ **A skipped site with a named mechanism is evidence; a skipped site without one
is a gap.** (Waffles, same ruling.)

| site | status | mechanism |
|---|---|---|
| 6 `erlang_stubs::bif_os_getenv_0` | **DEFERRED, not rejected** | Its input is `std::env::vars()` — **process-global state shared with every test running in parallel.** A probe would have to set env vars to control the population size, mutating state other tests read. The site's shape is unprotected and it remains a live candidate for RED; it needs a probe that does not touch the environment (or a serialised leg), not a smaller effort. |

Recorded so the site cannot later read as "looked at and found clean".

---

## Site 7 — `uri_bifs.rs::bif_uri_string_parse` — **RED-DEMONSTRATED**

Carrier `values` (bounded at 7: scheme, userinfo, host, port, path, query,
fragment), sink `alloc_map`. Probe banked at `gate-logs/110/probe_site_7.rs.txt`;
`uri_bifs.rs` reverted byte-identical after the run.

**RED cell:** heap 1024 w, pre-fill margin 8–48 →
`entry 0 (scheme): value is not a binary — carrier 'values' went stale`.
**12 corruption cells, 6 clean.**

### ⭐ THE BAND IS TWO-SIDED, WHICH IS STRONGER THAN A ONE-SIDED CONTROL

Holding the heap at 1024 and varying only the pre-fill margin:

| margin | 4 | 8 | 16 | 24 | 32 | 48 | 64 | 96 |
|---|---|---|---|---|---|---|---|---|
| verdict | ok | RED | RED | RED | RED | RED | ok | ok |

Clean on **both** sides of the corruption band. The upper clean cells are the
ordinary "enough room, no collection" control. The **lower** clean cell (margin 4)
is the more interesting one: there the heap is so tight that the *first*
allocation fails or collects before anything has accumulated, so there is no
live carrier to go stale. ⇒ the defect needs a carrier that is **already partly
populated** when the collection lands, which is the class definition, observed.

### ⚠️ MY FIRST SITE-7 PROBE PROVED NOTHING, AND THE PRE-REGISTERED TRAP IS WHY

The pre-registration said: the carrier is bounded at 7, so element COUNT cannot
be the knob — component LENGTH must be. **That was half right and the wrong
half.** `alloc_binary` promotes anything over `REFC_BINARY_THRESHOLD` (64 bytes)
to a ProcBin costing a **flat 3 heap words** regardless of payload size, so a
kilobyte component applies *less* nursery pressure than a 64-byte one. Growing
the components would have made the probe weaker.

Measured rather than assumed: at the smallest heap the whole parse consumed
**13 words against 61 available** — no collection could occur at any cell, and
all 12 cells came back "ok". Under the site-14 law that clean surface is an
**unresolved** verdict, not a defended one. The knob is the **heap margin**, set
by pre-filling with unrooted filler until a measured number of words remain.

⇒ **Recorded as an erratum against my own pre-registration: naming a trap in
advance is not the same as naming the right knob, and the surface that looked
like "site 7 is fine" was really "the probe never applied pressure".**

---

## Site 5 — `native/file_meta_bifs.rs::finish_list_dir`, carrier `terms` — **RED-DEMONSTRATED**

Probe: `gate-logs/110/probe_site_5.rs.txt`, appended with `tail -n +9` to
`crates/beamr/src/native/file_meta_bifs.rs`. Driver is a hand-built
`FileIoCompletion { continuation: ListDir, completion: { result: Ok(IoResult::DirList(names)) } }`
— no filesystem is touched. Knob is the heap MARGIN via unrooted pre-fill.

### The measured surface — 28 cells, heap 4096 words

Every cell reports **both** the requested margin and the margin actually
achieved. That distinction is the whole finding at this site; see below.

| requested margin | 0 | 1 | 2 | 3 | 4 | 8 | 16 | 32 | 64 | 128 | 256 | 512 | 1024 | 4096 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **achieved** | 4090 | 4090 | 4090 | 4090 | 4 | 4 | 16 | 28 | 64 | 124 | 256 | 508 | 1024 | 4096 |
| 50 entries | ok\* | ok\* | ok\* | ok\* | RED | RED | RED | RED | RED | RED | ok | ok | ok | ok |
| 200 entries | ok\* | ok\* | ok\* | ok\* | RED | RED | RED | RED | RED | RED | **RED** | **RED** | ok | ok |

\* **NOT EVIDENCE — see the resolution floor below.** 14 corruption cells, 14
clean cells. Every RED reads `entry 0: head is not a binary — carrier went stale`.

### Two controls, in both directions

1. **Hold the input, grow the room.** At 50 entries the site corrupts at every
   achieved margin ≤ 124 and is clean at ≥ 256. Identical input, more headroom,
   clean result ⇒ the failure is collection-dependent, not an allocator limit.
2. **Hold the room, grow the input.** At achieved margins 256 and 508 the 50-entry
   case is clean and the 200-entry case is RED. This is the second arm the plan
   asked for, and it moves in the direction the class predicts: more accumulation
   across the same headroom drags the corruption band upward.

### ⛔ THE BAND IS ONE-SIDED, AND THE INSTRUMENT — NOT I — ESTABLISHED WHY

There is no clean cell **below** the corruption band. Under PART 1B a two-sided
band is preferred, so this has to be stated rather than glossed: **site 5's band
is one-sided.** The reason is now measured, not guessed:

> `site 5 pre-fill resolution floor: smallest achieved margin 4 words, reached by
> 4 of 28 cells — requested margins below that were NOT measured`

The pre-fill descends by one filler allocation at a time (~6 words for a 32-byte
inline binary), so **4 words is this knob's resolution floor.** A margin finer
than that cannot be reached by allocating; the only thing that lands below it is
a collection, which frees the unrooted filler and returns the heap to ~4090 free
words. That is exactly what the `0/1/2/3` cells did.

### ⭐⭐ THE FINDING THAT MATTERS MORE THAN THE VERDICT: I ALMOST MANUFACTURED THE LOWER CLEAN EDGE I WAS LOOKING FOR

The first version of this probe reported only the **requested** margin. Run with
margins 0/1/2/3 added, it would have printed:

```
entries 50 margin 0 : ok      entries 50 margin 2 : ok
entries 50 margin 1 : ok      entries 50 margin 3 : ok
```

— four clean cells below the corruption band. **That is precisely the lower clean
edge I had set out to find, and I would have written site 5 up as a two-sided
band matching site 7.** It would have been false. Those cells ended with **4090 of
4096 words free**: no pressure was applied at all, because the pre-fill collected
and reset. A no-pressure clean cell is the asleep-instrument reading, and putting
it at the bottom of a band would have dressed it as the strongest control in the
set.

Three things had to go right to catch it, and only one of them was foresight:

1. The unfixed loop **hung** instead of returning — it had no give-up condition,
   so at margins below its resolution it span forever. Silence is what exposed
   it; a probe that had merely been *wrong* would have returned in milliseconds
   with four beautiful clean cells.
2. The hang was diagnosable only because the sweep had been running for ten
   minutes with **no rustc children** — it was past compile and inside the test.
3. Piping the run through `tail -n 45` had **swallowed all interim output**, so
   the hang looked exactly like a slow build. That is my own instrument error
   and it cost the first run; probes now stream to a log file.

⇒ **A KNOB HAS A RESOLUTION, AND A PROBE THAT REPORTS THE REQUESTED VALUE RATHER
THAN THE ACHIEVED ONE FABRICATES CELLS IT NEVER MEASURED.** The requested value is
what I asked for; the achieved value is what happened. Reporting the former is
not an approximation of the latter — where they diverge it invents data, and it
invents it *in the shape of whatever I was looking for*, because the cells that
fail to reach their target are exactly the extreme ones a search runs toward.

⇒ Both probes now **return the achieved margin** and print `req N got M`.
Cells sharing an achieved margin are **one measurement, not two** (requested 4
and 8 both achieved 4 here; 32 achieved 28; 128 achieved 124; 512 achieved 508).

This is Themis' warning — *a result which looks right is the condition under
which verification stops* — landing on my own instrument within the hour of it
being given, and it did not arrive as a general caution I applied wisely. It
arrived as a hang I had to explain.

---

## Site 10 — `native/udp_bifs.rs::finish_udp_recv`, carrier `ip` — **RED-DEMONSTRATED**

Probe: `gate-logs/110/probe_site_10.rs.txt`, appended with `tail -n +18` to
`crates/beamr/src/native/udp_bifs.rs`. Driver is a hand-built
`IoResult::DatagramReceived { bytes, data, addr }` with a `SocketAddr::V4`.
Heap 2048 words throughout; knob is the margin.

⭐ **This is the cheapest demonstration in the set that the class does not need a
loop.** `ip` is a **single term** — a 4-tuple from `ipv4_tuple` — allocated
before `alloc_binary(datagram)` and read after it by `alloc_tuple`. One
allocation across one other allocation is enough.

### The measured surface — 36 cells, heap 2048 words

| achieved margin | 2042\* | 2 | 8 | 14 | 20 | 32 | 62 | 128 | 512 |
|---|---|---|---|---|---|---|---|---|---|
| datagram 32 B | ok\* | **ok** | **RED** | ok | ok | ok | ok | ok | ok |
| datagram 64 B | ok\* | **ok** | **RED** | **RED** | ok | ok | ok | ok | ok |
| datagram 1024 B | ok\* | ok | ok | ok | ok | ok | ok | ok | ok |

\* no pressure — see below. 5 corruption cells / 31 clean. Every RED reads
`ip is not a tuple — carrier `ip` went stale`.

### ⭐ THE BAND IS TWO-SIDED ON BOTH SMALL-DATAGRAM ROWS

Clean at achieved margin **2**, RED at **8** (and 14 at 64 B), clean from **20**
upward. The lower clean edge is real here in a way it was not at site 5: margin 2
is an *achieved* margin, so pressure genuinely was applied and the result was
still clean — with two words free the allocation collects before `ip` has been
built, so there is no live carrier to go stale. That is the class definition
observed from below, not asserted.

### ⭐⭐ THE PRE-REGISTERED KNOB IS REFUTED AT THE BYTES, AND THE REFUTATION IS THE ROW THAT STAYS CLEAN

The plan said: *"the carrier is a SINGLE term, so the knob is the datagram SIZE —
one large payload must force the collection."* The 1024-byte row is **clean at
every single cell.** Not because the site is safe — the 32- and 64-byte rows on
the identical code prove it is not — but because `alloc_binary` promotes anything
over `REFC_BINARY_THRESHOLD` (64) to a **ProcBin costing a flat 3 heap words**.
A kilobyte datagram therefore applies *less* nursery pressure than a 32-byte one.

The band's shape measures the threshold from outside the allocator:

- **32 B** (inline): band one achieved-margin wide — `{8}`
- **64 B** (inline, at the threshold): band **widens** to `{8, 14}` — most pressure
- **1024 B** (ProcBin): band **vanishes** — pressure collapses to 3 flat words

Monotone up to 64 bytes, discontinuous across it. Had I run the pre-registered
sweep — large payloads only — every cell would have come back clean and site 10
would have been written up as DEFENDED on a probe structurally incapable of
firing.

### ⭐⭐⭐ THE NO-PRESSURE CELL IS A TWO-WAY CONTROL, AND IT CAUGHT A FABRICATED **RED** HERE

The first run of this probe reported **17 corruption cells**, twelve of them the
entire 1024-byte row reading `payload binary missing`. They were not corruption.
`Binary::new` accepts only `BoxedTag::Binary` — an **inline** heap binary — and
returns `None` for a ProcBin, so my reader could not see a >64-byte payload *at
all*. Switching the check to `BinaryRef`, which handles both representations,
turned all twelve green and left exactly 5 real reds.

**The tell was the no-pressure cell.** Those cells failed identically at achieved
margin 2042 — with 2042 of 2048 words free, where nothing has been allocated that
could possibly be stale. A failure that survives the removal of all pressure is
not a pressure failure.

⇒ **A CELL WITH NO PRESSURE MUST COME BACK CLEAN, AND IT IS A CONTROL IN BOTH
DIRECTIONS.** At site 5 tonight the no-pressure cells were reported as clean at a
margin they never reached, and would have fabricated a **lower clean edge**. Here
the no-pressure cells came back RED and fabricated a **corruption band**. Same
control, opposite failures, both mine, both in the same sweep family:

- a no-pressure cell that reads RED ⇒ **the reader is broken** (false positive)
- a no-pressure cell that reads clean **at a margin it never achieved** ⇒ **the
  knob is broken** (false negative)

The corollary is the uncomfortable one: **a probe must be shown able to read the
CORRECT value, not merely to reject the wrong one.** My payload check was
perfectly good at detecting a non-inline binary and calling it missing. It was
never asked whether "missing" and "unreadable by this accessor" are the same
thing, and they are not.

---

## Sites 12 + 16 — `beamr-wasm/src/convert.rs::json_value_to_term` — **both RED-DEMONSTRATED (attached), production path UNREACHABLE**

Probe: `gate-logs/110/probe_sites_12_16.rs.txt`, appended with **`tail -n +25`**
(NOT +26 — see below). Run with the canon `wasm-tests` leg command filtered to
`ar1_row4`. Toolchain present, zero setup: `wasm-bindgen-test-runner 0.2.123`,
`wasm32-unknown-unknown` installed.

Site 12 = the `Value::Array` arm, carrier `tail`. Site 16 = the `Value::Object`
arm, carrier `pairs`. Both are arms of the same function.

### Arm A — ATTACHED PROCESS. Both fire. 21/27 and 16/8.

| achieved margin | 4090\* | 4 | 16 | 28 | 64 | 124 | 256 | 508 | 1024 | 4096 |
|---|---|---|---|---|---|---|---|---|---|---|
| site 12, 1000 elements | ok\* | **317** | **315** | **313** | **307** | **297** | **275** | **233** | **147** | ok |
| site 16, 50 entries | ok\* | **49** | **48** | **46** | **42** | **34** | **18** | ok | ok | ok |

Cells give the **index at which the carrier went stale**. \* = no pressure.
Site 12: 21 corruption cells / 27 clean. Site 16: 16 / 8. Both bands are
**two-sided** — clean above (enough room, nothing collects) and clean below at
the no-pressure cells.

⭐ **THE FAILING INDEX MOVES MONOTONICALLY EARLIER AS HEADROOM GROWS** —
`317→315→313→307→297→275→233→147` and `49→48→46→42→34→18`. More pre-filled
headroom means the collection lands later in wall-clock but earlier in the
*accumulation*, so the corruption point tracks the collection point exactly.
That is the class's mechanism observed directly, not inferred from a pass/fail.

### Arm B — DETACHED CONTEXT, the production shape. Clean at 50 / 200 / 2000.

Measured at the bytes **before** the probe was written: `ProcessContext`'s
allocation path with no attached process pushes a fresh `Box<[u64]>` into
`detached_allocations` per allocation — never moved, never freed, never
collected — and `ensure_heap_space` is a no-op returning `Ok(())`. The **only**
caller of `json_value_to_term` is `terms_from_json_array`, which builds
`ProcessContext::new()` per element and never attaches a process.

⇒ **Two-tier verdict, same shape as #91's F3 finding: the code is defective, and
its production path cannot reach the defect.** The defence is the caller's, not
the site's — as at site 4. If any future caller passes an attached context, both
arms are live; arm A is the standing proof of that.

### ⚠️ MY PRE-REGISTERED SITE-12 KNOB WAS WRONG, AND THE MEASUREMENT CORRECTED IT

The first attached sweep was **clean at all 24 cells for site 12 while site 16
went RED at identical margins in the identical rig** — so pressure was
demonstrably applied and site 12 still did not fire. The reason is at the bytes:
`alloc_cons` **roots its own arguments** (`with_rooted(&[head, tail], …)` before
`ensure_heap_space(2)`), so site 12's carrier is re-rooted and forwarded on every
iteration. Its exposure window is only the single `alloc_binary` for the next
head. At heap 4096 with ~7 words per element, the one collection fires on the
first allocation — when `tail` is still `Term::NIL`, an immediate with nothing to
go stale — and 200 elements then fit with no further collection.

⇒ The knob is **input length relative to heap**, not margin: the list must be too
long to fit in one post-collection nursery (>~580 elements at 4096 words).
At 1000 and 2000 elements it fires immediately. **Third site running where the
pre-registered knob named a hazard rather than the knob.**

### ⭐⭐ THE OUTPUT CHANNEL WAS DEAD, AND A PASSING TEST HID IT — MEASURED THREE WAYS

`println!` produced nothing. The obvious explanation was wasm-bindgen-test
capturing output for passing tests, so I re-ran with `-- --nocapture` — **still
nothing, including from the test that passed.** That refutes the capture
hypothesis for `println!`. `console.log` via `js_sys` was then also absent.
**Only a panic payload survives this runner.**

That matters more than the plumbing: with a bare `red > 0` assertion, both tests
**passed and printed nothing**, so a green run could not be told apart from a run
that measured nothing at all — the hung/slow-compile pair again, in a third form.
The repair is to **pin the exact counts** (`assert_eq!((21, 27, 16, 8), …)`) with
the full surface in the assertion message, so any drift prints everything.

⇒ **A VERDICT THAT IS ONLY LEGIBLE WHEN THE ASSERT FIRES IS NOT A VERDICT.** The
passing path must carry its evidence, or "green" means "the assertion did not
fire", which is not the same claim.

### ⚠️ THE APPEND OFFSET IS PART OF THE INSTRUMENT

`tail -n +26` drops the `#[cfg(all(test, target_arch = "wasm32"))]` line and
compiles the probe into the **lib** target, where the `wasm-bindgen-test`
dev-dependency is not linked — E0432 pointing at a line I had written correctly.
The banked header now states `+25` and says why.

### Artefact integrity

The banked probe was re-run after the counts were pinned and **reproduces
21/27/16/8 exactly** (rc 0, 2 passed). `convert.rs` reverted and verified
byte-identical to HEAD.

---

## Site 3 — `native/code_management_bifs.rs::all_loaded`, carrier `list` — **RED-DEMONSTRATED**

Probe `gate-logs/110/probe_site_3.rs.txt`. Facility is a 6-method trait stub
(`LoadedModulesFacility`); no live code server, no subsystem. Heap pinned at
4096 words throughout.

```rust
let mut list = Term::NIL;
for (module, source) in loaded_terms.into_iter().rev() {
    let tuple = context.alloc_tuple(&[module, source])?;   // <-- the window
    list = context.alloc_cons(tuple, list)?;               // <-- re-roots
}
```

`alloc_cons` roots its own arguments, so `list` is re-rooted and forwarded every
iteration and its **only** exposure is the single `alloc_tuple` for the next
head. Structurally identical to site 12.

### Sweep A — LENGTH axis, no pre-fill. **Two-sided, monotone, sharp.**

| modules | 10 | 50 | 200 | 500 | 800 | 1000 | 1500 | 2000 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| verdict | ok | ok | ok | ok | ok | **RED** | **RED** | **RED** |

Failure is always `head is not a tuple — carrier list went stale`, at entries
181 / 681 / 362. The flip sits between 800 and 1000 against a **pre-registered**
~819: each module costs 3 words of tuple (header + 2) plus 2 words of cons = 5
words, and 4096 / 5 ≈ 819. **This prediction was made from the heap arithmetic
before the run and it held.**

### Sweep B — MARGIN axis, input pinned at 200 modules. **Prediction BROKEN.**

Predicted **clean at every cell**, on the site-12 mechanism: the pre-fill's
collection fires while `list` is still `Term::NIL`, an immediate with nothing to
go stale, after which the input fits. Measured:

| achieved margin | 2044 | 1024 | 508 | 256 | 124 | 64 | 28 | 16 | 4 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| verdict | ok | ok | ok | **RED** | ok | ok | ok | **RED** | ok |

The mechanism story is **wrong as a general claim.** It holds only at achieved 4,
where the heap cannot fit even one iteration. Everywhere below ~1000 words the
collection *does* fire mid-accumulation with `list` live — that is at least six
cells — and only **two** come back detectably corrupt.

### ⭐⭐⭐ THE FINDING: DETERMINISTIC IS NOT THE SAME AS MONOTONE, AND ONLY MONOTONE LICENSES INTERPOLATION

**Re-ran five times: bit-identical every run, down to the failing entry indices
(149 at margin 256, 197 at margin 16).** So sweep B is not noisy. It is fully
deterministic and simply **not monotone**.

⇒ **A CLEAN CELL ON THE MARGIN AXIS IS NOT EVIDENCE OF SAFETY.** In the four
clean-but-collected cells (508, 124, 64, 28) the carrier went stale and the read
still found intact bytes, because the abandoned region had not yet been
overwritten. **The defect occurred and was invisible.** The margin axis measures
*whether the corruption was visible*, not *whether it happened*.

That is worse than a noisy knob, because a deterministic non-monotone band
**looks like a measurement**. It invites exactly two false inferences, and the
data refutes both: "clean at 508 ⇒ safe at anything above it" dies at 256, and
"safe below some floor" dies at 16.

### ⚠️ THIS CUTS BACKWARDS INTO THIS LANE'S OWN EARLIER RESULTS — sites 5 and 10

Sites 5 and 10 used **margin** as their knob. **Their RED cells stand; a red is a
red.** Their CLEAN cells are now known to be weaker than they looked, and their
bands must be read as **lower bounds on the defect's reach, not as located
edges**. Recorded here rather than left for whoever next cites them. Neither
verdict changes — both were RED-DEMONSTRATED and remain so — but the *edges* of
those two bands are not where the clean cells suggested.

### Consequences for the landed regime

- **The landed probe uses the LENGTH knob.** Monotone, threshold derivable from
  heap arithmetic, fires every time — Waffles' condition 1 satisfied by
  construction rather than by luck.
- The whole surface is pinned `assert_eq!((a_red, a_ok, b_red, b_ok), (3, 5, 2, 7))`
  with both sweeps in the message (condition 3).
- A second assert fires if the margin axis ever becomes **monotone**, because the
  "clean cells under-report" finding was derived *from* non-monotonicity and must
  be re-derived rather than inherited if the allocator changes.

---

## Site 1 — `distribution/control.rs::alloc_spawn_request`, carrier `mfa` — **RED-DEMONSTRATED**

Probe `gate-logs/110/probe_site_1.rs.txt`. Heap pinned at 512 words. No live
distribution, no connection — a `SpawnRequest` is a plain struct.

**The sharpest result in this lane: the defect's location was predicted from the
heap arithmetic before the run, and it landed on the exact argument count.**

### The window is bounded by construction, which makes this site unlike 3 and 12

```rust
let args = context.alloc_list(&request.mfa.args)?;          // 2N words
let mfa = context.alloc_tuple(&[module, function, args])?;  // 4 words
let opt_list = spawn_options_to_list(context, options)?;    // <-- WINDOW
context.alloc_tuple(&[op, req_id, from, group_leader, mfa, opt_list])
```

`mfa` is live across `spawn_options_to_list` and is not one of its arguments, so
it is unrooted there — `alloc_list_with_tail` roots its ELEMENTS and `mfa` is not
among them. But `SpawnOptions` contributes exactly two booleans to that list
(`link`, `monitor`), so `elements.len() <= 2` and **the window demands at most 4
words, by construction.** This is not an accumulation whose exposure grows with
input; it is a single bounded allocation, and the defect can only fire when
`available()` lands in **[0, 4)** at precisely that call.

**So the knob is the argument count used as a RULER, not as pressure**: each
argument costs exactly 2 words at step 1, so `available_at_window = H - 2N - 4`
walks down in steps of 2 as N grows.

### Pre-registered prediction, made before the run

> RED in **one or two cells** of the whole sweep and clean everywhere else — an
> isolated spike, not a threshold. A wide red band would refute the arithmetic.

### Measured — 301 argument counts × 2 arms

| arm | red | clean | refused |
| --- | --- | --- | --- |
| **ON** (`link` + `monitor`) | **2** | 299 | 0 |
| **OFF** (`SpawnOptions::default()`) | **0** | 301 | 0 |

```
args 253  available 512  window headroom 2 : mfa is not a tuple — carrier `mfa` went stale
args 254  available 512  window headroom 0 : mfa is not a tuple — carrier `mfa` went stale
```

Headroom swept **−92 .. 508**, straddling the target band. `available` is exactly
512 — no heap overhead — so `512 - 2N - 4` is exact, and since every term of that
expression is even, **0 and 2 are the only values in [0, 4) that parity can
reach. Both of them fired.** Two for two on the reachable target cells, at the
argument counts the arithmetic named in advance.

### ⭐ Arm OFF is a STRUCTURAL negative control, not a tuned one

With both booleans false, `elements` is empty and the window becomes
`ensure_heap_space(0)`, which cannot collect. Same heap, same arguments, same
reader, same 301 cells — the *only* difference is whether the window allocates at
all. **0 red.** So the attribution is not an inference from a correlation: the
exposure is `spawn_options_to_list` and nothing else, demonstrated by removing
that one allocation and watching the entire band disappear.

### ⛔ THE INSTRUMENT FAILURE THIS SITE PRODUCED, AND IT WAS MINE

The first sweep ran `0..=200` and returned **201 clean ON cells**. That green was
worth nothing: at 200 arguments the window headroom only descends to
`512 - 400 - 4 = 108`, so **the ruler never reached the band the defect lives in.**
The site-14 law caught it — the probe asserted UNRESOLVED rather than DEFENDED —
but the assertion only fired because a red was *expected*. Had this site been
genuinely defended, that mis-ranged sweep would have produced a clean result
indistinguishable from a real one.

⇒ **A COVERAGE ASSERTION IS NOW PART OF THE PROBE**, and it fires *before* the
verdict assertions:

```rust
assert!(headroom_min < 4 && headroom_max >= 4,
    "INSTRUMENT NOT SHOWN AWAKE: window headroom swept {headroom_min}..{headroom_max}, \
     which does not straddle the target band [0, 4). Any verdict from this sweep is void");
```

**A sweep must prove it passed through the band where the defect can live.**
Otherwise a clean result reports the range of the knob, not the state of the
site — the asleep-instrument shape, arriving this time through a knob that was
simply too short.

---

## Site 2 — `distribution/pg.rs::members`, carrier `terms` — **RED-DEMONSTRATED**

Probe `gate-logs/110/probe_site_2.rs.txt`, appended to **`pg.rs` itself** — not
to the sibling `pg_tests.rs`, because `members` is private and `pg_tests` is a
sibling module doing `use super::pg::*`, which reaches only public items. A child
module of `pg.rs` sees its parent's privates. Heap pinned at 512 words. Facility
is a stub roster; no live distribution, no connection.

### The pre-registered trap was a HAZARD, and it is not the knob

The plan warned this site "needs REMOTE members". True — and that is a
*precondition for the probe to be capable of failing*, not the thing that makes
it fail. `Term::try_pid` for a local member is an **immediate** and allocates
nothing, so a local-only probe is structurally incapable of firing. The knob is
the ordinary one for this shape: **remote-member count against a fixed heap.**
`alloc_external_pid` is `alloc_words(4)`, so each iteration costs 4 words and can
collect while every pid already pushed into `terms` sits there unrooted.

**Waffles' hazard-not-knob instruction is now 4 for 4** (sites 7, 10, 12, 2).

### The measured surface — 21 counts × 2 arms

| region | counts | arm REMOTE | arm LOCAL |
| --- | --- | --- | --- |
| below the collection point | 1, 10, 50, 100, 120, **128** | **6 clean** | clean |
| **the live band** | **129, 130, 132, 135, 140, 145, 150, 160, 170** | **9 RED** | clean |
| allocator refuses | 180, 190, 199, 200, 300, 500 | *error term* — **not evidence** | clean |
| | | **9 red / 6 clean** | **0 red / 21 clean** |

⭐ **THE EDGE IS EXACT.** Clean at 128, red at 129 — and `128 × 4 = 512` is the
whole heap, while `129 × 4 = 516` is the first count that cannot fit. The
collection point predicted from `alloc_words(4)` against a 512-word heap lands on
the precise member, with no fitted constant.

⭐ **EVERY RED CELL CARRIES THE SAME SIGNATURE:** `entry 0: pid_number 129 != 1`.
Element 0 of the returned list reads back as **member 129** — the very member
whose allocation triggered the collection. That is not truncation, not a refusal,
and not type confusion: it is a stale pointer into the old nursery being read
after the collector moved things, landing on what was written there instead.
**The same signature as sites 8/9**, where element 0's key read back as pair
186's key.

### Arm LOCAL is a structural negative control

Same heap, same counts, same reader; the only difference is that the loop body
allocates nothing because local pids are immediates. **0 red across all 21
cells.** So the attribution is demonstrated by removal, not inferred from a
correlation: take away the `alloc_external_pid` and the entire band disappears.

⛔ The six *error term* cells are **not** counted as evidence — ambiguous by
construction, exactly as at sites 8/9. They are recorded, not relied on.

### Coverage assertion

Per site 1's lesson the probe refuses to render a verdict unless the sweep
demands more heap than exists: the largest count must satisfy `count * 4 > HEAP`,
or every cell is vacuous and the assertion says so in those words.

---

# THE LANDING — form, evidence, and what it changed

Census closed at **16 of 17 resolved**: site 4 **DEFENDED**, sites 1, 2, 3, 5,
7, 8, 9, 10, 11, 12, 14, 15, 16 **RED-DEMONSTRATED** (12 and 16 two-tier), sites
13 and 17 Cally's, already demonstrated at `308b448`. Site 6 stays **DEFERRED**
on its named mechanism.

## ⛔ THE LANDED FORM ASSERTS THE MEASURED SURFACE, NOT CORRECTNESS

Cally's Amendment 6 set the precedent that a row-4 probe **asserts correctness,
is therefore red at HEAD, and lands with the fix.** Waffles ruled the other way
for this set, and every probe header that carried the old policy has been
corrected in place rather than quietly dropped.

⇒ **These tests are DEFECT-ASSERTING. They pin the corrupt surface, so they are
green at `f993280` and go RED when AR-1 is fixed.** That is the intended
tripwire. Each module now carries that statement as its first lines, because a
future reader finding a green probe must not conclude the site is safe. **The
fix lane INVERTS them; it does not delete them.**

The one exception is site 4, whose probe asserts survival and stays green
through a fix — recorded in the ledger so nobody inverts it by pattern-matching
on the rest of the set.

## The two-tier verdicts travel in the test's own NAME

Waffles' condition 2, and the artefact it exists to prevent us replicating is
site 4's own sibling `get_0_returns_complete_dictionary_as_tuple_list` — a test
that names the right function, hits the right line, and is structurally
incapable of failing on this class.

| site | test name now carries |
|---|---|
| 4 | `ar1_site4_defended_by_the_callers_prereserve_not_by_the_site` |
| 12 + 16 | `ar1_sites_12_16_tier1_defective_red_with_a_process_attached` |
| 12 + 16 | `ar1_sites_12_16_tier2_production_path_unreachable_detached_context` |

## The applier, and why there is one

⭐ **I got an append offset wrong twice — once harmlessly, once loudly** (site
12/16 lost its `cfg` line and rustc reported `E0432` against a line written
correctly). `gate-logs/110/apply_probes.sh` removes transcription entirely: it
**derives** each offset from the probe's own `cfg` attribute, then **verifies
what actually landed in the target** rather than what was intended, and refuses
on any deviation — missing file, non-blank separator, or a module already
present (so a second run cannot silently double an append).

**Round-trip proof, run after `cargo fmt` reflowed the probes:** the nine
applier-owned targets were reverted to HEAD and re-applied from the re-synced
banked sources — **9/9 byte-identical.** The banked artefacts therefore
reproduce the landed bytes, so the record and the tree cannot drift apart.

## ⛔ FOUR LINTS THE PROBES ONLY MET AT THE CANON BATTERY

Every probe had been run — repeatedly — with `cargo test`. Not one had been run
under `cargo clippy --all-targets -- -D warnings`, and the first canon battery
returned **rc 101** on four findings, all mine:

| file | finding |
|---|---|
| `uri_bifs.rs` | `unused_imports` — `crate::term::Term` |
| `uri_bifs.rs` | `clippy::manual_repeat_n` |
| `file_meta_bifs.rs` | `unused_assignments` — dead initialiser on `achieved` |
| `udp_bifs.rs` | `unused_assignments` — the same, in the sibling pre-fill loop |

⭐⭐ **RUNNING A PROBE IS NOT RUNNING THE GATE THAT WILL GRADE IT.** This is the
same shape as my feature-reach erratum one axis over: there I measured a gate's
reach with a *crate-scoped* invocation when the gate is workspace-scoped; here I
measured a probe's health with the *test* leg when the canon grades it with two
clippy legs as well. **In both cases the smaller invocation was convenient,
answered a real question, and answered a different one from the one I needed.**

The `achieved` repair is worth naming, because it touched a witness. That
variable exists so a cell that **could not reach** its requested margin is
reported at the margin it actually got — a give-up must be visible in the DATA,
not only in the control flow. The dead store was the initialiser, so the loop
became `let achieved = loop { ... break available; }`: every original break path
already held `available` from the top of its own iteration, so the value the
witness carries is unchanged at every exit, including the allocation-failure
exit.

## The ledger — 17 of 17 rows carry a row-4 disposition, none silent

`docs/design/beamr/briefs/evidence/accumulator-rooting/dispositions.json` gains
a `row4_red_at_parent` field on **every** crossing, including the two that are
Cally's and the one that is deferred. Silence about a site is the failure.

Two rows had text that this lane made FALSE, and both are corrected in place
with the original **retained verbatim**:

- **ids 12 and 16** read *"NOT DEMONSTRATED … stays UNVERIFIED until its own
  red-at-parent exists."* The condition it set has now been **met**, on the wasm
  leg it named. ⭐ The clause was right to refuse an analogy, and it is
  discharged by a measurement rather than by a re-reading.
- **id 14** read *"ANOTHER SEAT'S. This row is accounted for, not owned."*
  Ruled into this lane by Waffles: row 8 forbids absorbing **another seat's live
  work**, Osiris is withdrawn with no replacement, so the site is **unowned, not
  owned-elsewhere**. What row 8 protects against is *silent* absorption, so the
  transfer is taken loudly.

`ledger_check.py` passes and its `--self-test` still forces **all ten** checks to
refuse, including the `STRUCTURALLY-ELIMINATED` arm that has no live rows —
verified after the edit, not before it.

## Erratum on my own resume record

It said **"PROBES ON DISK — NINE FILES"** and then enumerated **ten**. There are
ten, and all ten are applied. The heading was the wrong count; the list was
right. Recorded because a wrong denominator in a handover note is exactly the
shape that makes a later census look complete when it is short.
