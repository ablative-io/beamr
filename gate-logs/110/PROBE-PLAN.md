# AR-1 ROW 4 — probe plan for the remaining sites, and the re-run pre-registration

Artemis Peach. Written **while the box is held**, from the site bodies read at
`f993280`. Costs the machine nothing. Its purpose is that the clean run is
mechanical rather than exploratory — and that what I expect is on the record
**before** the measurement, so it can falsify instead of confirm.

---

## PART 1 — THE RE-RUN PRE-REGISTRATION (sites 4, 8, 9)

Waffles' instruction, and the right one: pre-declare before re-running, so a
match is a confirmation rather than a rescue.

**I predict the clean re-run reproduces the contended run EXACTLY, on every
named axis:**

| axis | pre-declared value |
|---|---|
| site 4 arm A (production `* 5`) | GREEN |
| site 4 arm B (reserve deleted) | RED — `Tuple::new` returns `None` |
| site 4 arm C (`* 5 - 1`) | GREEN |
| site 4 arm D (`* 2`) | RED — `Tuple::new` returns `None` |
| site 4 CONTROL 1 pressure reading | **22 words available vs 1000 needed**, same integers |
| sites 8/9 heap 4096 × 200 pairs | ok |
| sites 8/9 heap 4096 × 400 pairs | RED — "element 0: not a tuple" |
| sites 8/9 heap 1024 × 200 pairs | RED — **key contents `k000000000186` != `k000000000000`** |
| sites 8/9 heap 16384, all sizes | ok |

**The falsifiable part is the exactness.** These are integer quantities — a word
count, a pair index — not timings. If contention were a factor, the *numbers*
would move, not just the pass/fail. **A changed integer refutes my "no timing
dimension" argument outright**, and I would report the verdicts as
contention-sensitive rather than explain the difference away.

Specifically: if the 1024/200 cell still corrupts but names a pair OTHER than
186, that is NOT a confirmation. It would mean the collection point moved, which
would mean something outside my declared variables is in play.

⭐ **The contention measurement makes this a stronger test, not a weaker one:**
the battery alone drives ~8.07 and my compiles took it to 9.57, so the two runs
differ by a ~1.5 load delta. If a ~1.5 delta can move a word count, that is a far
more interesting finding than the site verdicts.

---

## PART 1B — FORM UPGRADE, BINDING ON ALL EIGHT REMAINING SITES

Added after sites 7 and 14 both nearly produced a false `DEFENDED`. Ratified by
Waffles, who added the forward instruction that the pre-registered traps on
sites 2 and 10 **name a hazard, not necessarily the knob**, and that the knob
must be re-derived from a pressure measurement before any sweep's clean cells
are trusted.

### ⛔ THE STANDARD IS NOW A TWO-SIDED BAND, NOT A TWO-ARMED CONTROL

A one-sided control (grow the heap until it is clean) cannot distinguish *the
site is safe* from *the instrument was asleep*. Both present as a clean read.

**A two-sided band can.** Site 7's surface at heap 1024 was:

| margin | 4 | 8 | 16 | 24 | 32 | 48 | 64 | 96 |
|---|---|---|---|---|---|---|---|---|
| | ok | RED | RED | RED | RED | RED | ok | ok |

The **upper** clean cells are the ordinary "enough room, nothing collects"
control. The **lower** clean cell is the one that proves the instrument was
awake: at margin 4 the first allocation fails or collects *before anything has
accumulated*, so there is no live carrier to go stale. Clean cells on both sides
can only be produced by actually traversing the pressure regime.

⇒ **Every remaining site's sweep must exhibit a two-sided band, or state
explicitly that it could not and why.** A clean surface with no RED band is
recorded as **UNRESOLVED**, never as DEFENDED — `DEFENDED` still additionally
requires a positive-control arm that goes RED (site 4's arms B and D).

### The three checks each remaining probe must pass before its verdict counts

1. **PRESSURE** — the sweep must contain at least one RED cell, or the probe is
   reported as unable to apply pressure. Where the band is not found by size
   alone, pre-fill the heap with **unrooted** filler to a measured margin (site
   7's method) rather than growing the input.
2. **NOT-A-REFUSAL** — the RED assertion must exclude the refusal class
   explicitly. `is_err()` is satisfied by the allocator declining, which is the
   exact ambiguity the arm exists to rule out (site 14's third law).
3. **RESOLUTION** — step the sweep finely enough to catch a one-cell band, and
   search the cells *adjacent to the refusal band*, since a refusal can arrive
   after the corruption and hide it (site 14's first and second laws).

### ⭐ THE LAW BEHIND ALL OF IT

**NAMING A TRAP IN ADVANCE IS NOT THE SAME AS NAMING THE RIGHT KNOB.**
Pre-registration protects against moving the goalposts after seeing the data. It
does **not** protect against having built an instrument that cannot fire — and a
*confidently* pre-registered wrong knob is more dangerous than an unexamined one,
because the declaration itself reads as diligence. Site 7 was pre-registered
with the wrong knob and the declaration would have carried a false clean.

⇒ **Knobs below are HYPOTHESES to be re-derived against a measured pressure
reading, not instructions.**

---

## PART 2 — PER-SITE PROBE PLAN

Ordered by tractability, which is also the order the reds get cheapest. Every
probe carries the two obligations established at sites 4 and 8/9: **non-immediate
carrier contents** and a **control arm that isolates collection as the variable**
(hold the heap and grow the input; or hold the input and grow the heap).

### Tier 1 — pure functions of an argument, no facility, no global state

**Site 7 — `uri_bifs::bif_uri_string_parse`, carrier `values`.**
Driver: one binary argument. `values` accumulates up to **7** `alloc_binary`
results (scheme, userinfo, host, port, path, query, fragment) before the terminal
`alloc_map`, which roots them. ⚠️ **The carrier is bounded at 7, so element COUNT
cannot be the pressure knob — component LENGTH must be.** Build a URI whose
components are kilobytes each; the collection must fire between two pushes.
Expect: **RED**, same family as 8/9. If it stays green at every length, that is a
real result — 7 elements may simply not span a collection — and it is reported as
DEFENDED-BY-SIZE with the lengths tried named.

**Site 11 — `term/json.rs::array_to_list_term`, carrier `tail` (threaded).**
S3b shape: `tail` threaded through `alloc_cons`, with `value_to_term` allocating
between iterations. Driver: a JSON array of many string elements through the
crate's json entry point. Expect: **RED**.

**Site 15 — `term/json.rs::object_to_map_term`, carrier `pairs`.**
S3e shape (`Vec<(Term, Term)>`). Driver: a JSON object of many string-valued
keys. Expect: **RED**.
⚠️ **NOTE, NOT THIS LANE'S TO CHASE:** this site does
`pairs.sort_by_key(|(key, _)| *key)` — it sorts by the **raw `Term` bit pattern**,
i.e. by heap address for boxed keys. Even with rooting fixed, that makes flatmap
key order depend on allocation addresses rather than term value. Recorded as an
adjacent observation for the lane that owns map ordering.

### Tier 2 — constructible input data, no live facility

**Site 5 — `file_meta_bifs::finish_list_dir`, carrier `terms`.**
Driver: hand-construct a `FileIoCompletion` carrying `IoResult::DirList(entries)`
with many long filenames. No filesystem needed if the type is reachable from the
test. Expect: **RED** — unprotected `alloc_binary` accumulation, the plainest
shape in the set.

**Site 10 — `udp_bifs::finish_udp_recv`, carrier `ip`.**
Driver: hand-construct `IoResult::DatagramReceived { bytes, data, addr }`.
⚠️ **The carrier is a SINGLE term** (`ip`, from `ipv4_tuple`) live across
`alloc_binary(datagram)` and then `alloc_tuple`. Expect: **RED**, and it is the
cheapest demonstration in the set that the class does not need a loop at all.

⛔ **KNOB RE-DERIVATION REQUIRED — the obvious knob is probably wrong here, for
the same reason it was wrong at site 7.** "One large payload" is the natural
guess, but `alloc_binary` promotes anything over 64 bytes to a ProcBin costing a
**flat 3 heap words**, so a *large* datagram applies almost no nursery pressure.
A datagram just under the threshold applies more than one a thousand times its
size. **Pre-fill to a measured margin and sweep that**, and confirm a two-sided
band before trusting any clean cell.

**Site 14 — `string_bifs::bif_split`, carrier `terms`. ⭐ READ THIS ONE CLOSELY.**
Driver: three binary arguments; part count is set by the input. **This site is
ALREADY PARTIALLY HARDENED and carries a comment saying so:**

> `// Own the input up front: the per-part allocation loop below may collect,`
> `// so every part must borrow this owned buffer, never the process heap.`

Someone saw that the loop can collect and protected the **input**
(`binary_bytes(*input)?.to_vec()`) — and left the **accumulator** `terms`
unrooted across `alloc_binary`. ⭐ **A site that has been thought about and
partially fixed is more dangerous than one nobody looked at: the comment reads as
"handled" to the next reader.** Expect: **RED**. This is now my site by ruling,
and the partial-hardening is exactly why it must not be taken on the comment's
word.

### Tier 3 — needs a facility

**Site 3 — `code_management_bifs::all_loaded`, carrier `list` (threaded).**
Needs a code-management facility reporting many loaded modules. Both
`alloc_tuple` and `alloc_cons` allocate per iteration. Expect: **RED**.

**Site 2 — `distribution/pg.rs::members`, carrier `terms`.**
Needs a pg facility with **remote** members: only `alloc_external_pid` allocates
(`Term::try_pid` for locals is an immediate). ⚠️ **A probe with local members
only is structurally incapable of failing** — the same trap as the immediates.
Expect: **RED** with remote members, and the local-only arm is the natural
negative control.

⛔ **THAT TRAP IS A HAZARD, NOT THE KNOB.** Remote members are what makes the
probe *capable* of failing at all; they are not what makes it *fail*. The knob is
still heap pressure across the accumulation, and an external-pid box is a fixed
small cost per member, so member COUNT is the plausible knob here — but it gets
re-derived from a pressure reading, and the sweep must show a two-sided band
before any clean cell counts. **Satisfying the pre-registered hazard is not the
same as having applied pressure.**

**Site 1 — `distribution/control.rs::alloc_spawn_request`, carrier `mfa`.**
Needs a `SpawnRequest`. `mfa` is live across `spawn_options_to_list`, which ends
in `alloc_list`. Knob: long `mfa.args` and/or many spawn options. Expect: **RED**.

### Tier 4 — deferred, mechanism named

**Site 6 — `erlang_stubs::bif_os_getenv_0`.** DEFERRED, not rejected: its input
is `std::env::vars()`, process-global state shared with parallel tests. Needs an
env-free probe or a serialised leg. Shape is unprotected; still a live RED
candidate.

### Tier 5 — wasm leg

**Sites 12 + 16 — `beamr-wasm/src/convert.rs::json_value_to_term`** (carriers
`tail` and `pairs`), reached via `serde_json::Value` rather than `JsValue`.
Cally's leg is priced at zero setup, 28.19 s run. Her sites 13/17 probes are the
template; **priced-by-analogy is not priced**, so these get their own arms.

---

## Expected shape of the finished pass

On this reading, **most of the remaining sites are unprotected and should go
RED**; site 4 is so far the only DEFENDED verdict, and it was defended by a
caller's prereserve rather than by anything at the site. That is a prediction
recorded before the work, not a conclusion — and if the DEFENDED count comes in
higher, the disposition question Cally is ruling on moves further than anyone has
yet assumed.
