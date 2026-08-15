# AR-1 REMEDY PROBE — §7.1/§7.2 measurements, run BEFORE the ruling, deciding nothing

Artemis Peach. Ground: beamr main `a4e8802` (the proposal's own commit), probed
in a scratch worktree, worktree removed after; **zero production bytes landed
from this work.** Pre-registered in AR-1-REMEDY-PROPOSAL.md §7 — the probe's
output is a measurement, not a patch, and it does not pre-empt the §8 rulings.

## Method

Four arms, one sink each, restore-to-git between arms. Each arm: add the
sealed trait (`TermSource`, impl'd ONLY for `&[Term; N]`), flip exactly one
sink's parameter from `&[Term]` to `impl TermSource`, then
`cargo check -p beamr --features encode --all-targets` and read every E0277.
An error site is a call passing anything that is not a fixed-size array —
the compiler enumerates the migration set; a clean site is a literal that
compiled **textually unchanged**. Logs: `remedy_probe_arm{1..4}_*.log`.
Not probed: `list_from_vec` (takes a `Vec` by design — its 11 callers migrate
by definition) and the `beamr-wasm` crate (separate crate, not compiled by
`-p beamr`; its sites counted from the census + ledger, not the compiler).

## HEADLINE 1 — the zero-change claim is CONFIRMED AT SCALE

**Zero literal sites broke in any arm.** The census's 140 literal sites
(99 tuple · 39 list · 2 map) all pass `&[expr, …]`, which is `&[Term; N]`
before slice coercion; under the sealed trait every one compiled without a
character of change. The proposal's largest cost uncertainty is measured away.

## HEADLINE 2 — the compiler-measured migration set is SMALLER than the census's

| arm | sink | E0277 sites | note |
|---|---|---|---|
| 1 | `alloc_map` | **3** | all `&Vec<Term>`: `etf/decode.rs:269` · `json_bifs.rs:380` · `uri_bifs.rs:93` (each ×2 params = 6 errors) |
| 2 | `alloc_tuple` | **3** | `etf/decode.rs:409` · `ets/match_spec.rs:117` (`&[Term]` slice) · `type_conversion_bifs.rs:119` |
| 3 | `alloc_list` | **33** | list below |
| 4 | `alloc_list_with_tail` | **2** external | `etf/decode.rs:197` · `gate3_bifs/mod.rs:846`; +1 internal = `alloc_list`'s own delegation (`alloc.rs`), remedy plumbing not a site |

**41 external call sites in the beamr crate** (tests and examples included —
the census's 65 excluded test targets and included `beamr-wasm`, so the two
instruments bracket the true cost rather than contradict: **41 measured
(beamr, all targets) + 11 `list_from_vec` callers + the wasm-crate sites
(census/ledger: `convert.rs`, sites 12/13/16/17 among them).**
Notably arm 2: the census graded 7 tuple sites "variable" but only 3 fail —
several census-"variable" sites actually pass fixed-size arrays. ⭐ *The
syntactic census over-counts against the remedy; the compiler is the better
instrument, and it can only be consulted by building the probe.*

Arm 3's 33 (all `&Vec<Term>` except `ets/match_spec.rs:121`, a slice):
`distribution/control.rs:517,642` · `distribution/pg.rs:493` ·
`etf/decode.rs:175` · `ets/match_spec.rs:121` · `dictionary_bifs.rs:116,132` ·
`etf_bifs.rs:142` · `ets_bifs.rs:372,392,419,566,902,967,1297` ·
`file_meta_bifs.rs:170` · `gate3_bifs/mod.rs:322,770` ·
`gate3_bifs/type_conversion.rs:108,259` · `inet_bifs.rs:93` ·
`erlang_stubs.rs:70,302` · `process_info_bifs.rs:206,303,319` ·
`misc_bifs.rs:110` · `string_bifs.rs:93,136` ·
`type_conversion_bifs.rs:131,205` · `uri_bifs.rs:136` ·
`system_info_bifs.rs:191`.

## ⚠️ A migration site is NOT a defect site — say it before someone counts it

The error list contains **correctly-rooted S1 sites** (they still hand a
`&Vec` to the sink at the end — e.g. the four ets S1 uses), the **row-3-walled
prereserve sites** (`process_info_bifs.rs:206,303,319`,
`system_info_bifs.rs:191`, `ets_bifs.rs:566`), **ledger defect sites**
(`control.rs:517` = site 1 · `pg.rs:493` = site 2 · `dictionary_bifs` = site 4
· `file_meta_bifs` = site 5 · `erlang_stubs` = site 6 · `uri_bifs` = sites
7–9 · `string_bifs` = site 14, **Osiris', not absorbed by this measurement**),
and neutral sites. Migration cost ≈ 41+11+wasm; defect count stays 17. The
two numbers answer different questions and must not be traded.

## What this changes for the §8 rulings — information, not advocacy

The bracket question (§8.2) now reads: **~52 one-line-to-small call-site
migrations + 5 signatures**, with the 140-literal population measured at
**zero** text change. The criterion question (§8.1) is untouched — that is
still the ruling's to make. Sequencing (§8.3) and the S2+wall class (§8.4)
likewise untouched.

## Falsifier note

The probe's own negative control is built into its method: each arm's E0277
list is non-empty, so the instrument demonstrably CAN produce a presence —
the zero on literal sites is a measured zero, not a silent one.
