# Sites 4 and 6 — RESULTS

The last two `PENDING` crossings. **One discharged, one parked with its reason
and its re-check trigger.** Ground at `gate-logs/114/GROUND.md`, park at
`gate-logs/114/SITE4-PARK.md`.

⭐ **They were never the same problem, and finding that out was most of the
work.** Site 6 is a single-carrier shape the accumulator fixes outright. Site 4
has **two** carriers, one of them minted by its caller, and the accumulator
cannot discharge it at all.

## SITE 6 — DISCHARGED

`bif_os_getenv_0`, `crates/beamr/src/native/otp_stubs/erlang_stubs.rs`.

Row 6 was `PENDING` / **NOT DEMONSTRATED**, deferred on a *named mechanism*:
`std::env::vars()` is process-global, so sizing the population meant mutating an
environment other parallel tests read.

**The extraction is what dissolves that.** Pulling the loop into
`env_pairs_to_list(pairs, context)` makes the population a **parameter** rather
than ambient global state — no environment access, no serialised leg, no
test-ordering coordination. Then `with_accumulator` roots the run. The deferral's
own mechanism is removed rather than worked around.

### ⭐ The cell was chosen BY MEASUREMENT, and the failure modes are not interchangeable

`gate-logs/114/site6-cell-sweep.log` sweeps heap × population. The first cell I
tried passed, and the naive response — "push harder until it goes red" — would
have produced a red for the wrong reason:

| cell | call | data |
|---|---|---|
| heap=1024 n=200 | ok | intact — one collection, too late to matter |
| heap=1024 n=400 | **refuses** | corrupt |
| heap=1536 n=800 | **refuses** | corrupt |
| ⭐ **heap=1536 n=400** | **ok** | ⛔ **CORRUPT** |

At every other corrupting cell the call **also refuses**. A refusal is a loud
failure and proves nothing about rooting. **heap=1536 n=400 is the one measured
cell where the call SUCCEEDS and returns silently wrong data** — which is the
AR-1 hazard itself.

### The red

```
element 0 survived the collection intact
  left:  KEY_0307=value_0307
  right: KEY_0000=value_0000
```

The stale `Vec` pointer aliasing a **later** binary written over the reset
nursery. Post-fix: green.

### The positive control, and it is the #112 observable's first outing elsewhere

`ar1_site6_probe_population_really_collects` asserts on **`gc_attempts`**, not
heap capacity, and **passed in both the red run and the green run**. So the red
was under real collection pressure and the green is not bought by an unpressed
cell. First use of the collection observable outside the lane that built it.

### Ledger

Row 6 `PENDING` → **`STRUCTURALLY-ELIMINATED`**, replacement construct
machine-verified present at `erlang_stubs.rs:81`. `--self-test` 10/10;
`--sign-off` now refuses on **`[4]`** alone, down from `[4, 6]`.

An `ADDR_NOTE` records that `bind_line: 66` is a **historical pin** to the
pre-fix inline loop and is deliberately not re-pointed — the `Vec` it named no
longer exists, which is the point.

## SITE 4 — PARKED, and the park is about a REMEDY, not about difficulty

Full record: `gate-logs/114/SITE4-PARK.md`. In short:

1. ⛔ **The ledger's named remedy is the wrong instrument.** `entries_to_list`
   has **two** unrooted carriers — `tuples`, and **`entries` itself**, a slice of
   heap terms read across an allocating call. `entries` is minted one call above
   by `ProcessContext::dict_get_all` / `dict_erase_all`, which copy the
   dictionary's rooted slice into an owned `Vec`. For `bif_erase_0` the entries
   are **no longer rooted by anything** after the drain. Rooting `tuples` alone
   would leave a *partially* fixed site whose remaining hazard is invisible —
   arguably worse than today's honest state.
2. **What defends it is the caller's prereserve**, proven load-bearing by #110's
   positive controls (arm B RED, arm D RED, arm C GREEN).
3. **The remedy is to fuse reserve + drain + build** so "drain without reserving"
   is unrepresentable — the #86/#94/#103/#112 move.
4. ⛔ **PARK REASON, a fact rather than a preference:** the full remedy removes
   `dict_get_all` / `dict_erase_all` from `ProcessContext`, which is the
   **embedder-facing surface** of a **published crate**. That is
   **semver-breaking**, so it belongs to a breaking release, not to a fix lane
   landing on main tonight.
5. **RE-CHECK TRIGGER: the 0.19.0 cut**, where the remedy joins the existing
   cutter's list. At that cut the park reason is **re-measured, not assumed** —
   including re-deriving that each method still has exactly one caller. **If a
   third caller has appeared, the park is void and the row escalates.**
6. ⭐ **A non-breaking interim exists and needs one word:** add the fused methods
   now, migrate both call sites, leave the raw pair for deletion at 0.19.0. That
   discharges the hazard at the real call sites with zero breakage. Not done
   unilaterally because it adds surface to a published embedder-facing type.

Row 4 stays `PENDING`, so `--sign-off` keeps refusing. **Silence about a site is
the failure — the ledger should go on saying so.**

## The battery

Pin `cf00e03`. Prediction committed **before** the runner, pin wrinkle declared
up front.

| leg | predicted | measured | |
|---|---|---|---|
| 4 `wasm-tests` | 2 / 86 / 0 / 0 | 2 / 86 / 0 / 0 | ✅ unchanged |
| 5 `tests` | 76 / 2150 / 0 / 0 | 76 / 2150 / 0 / 0 | ✅ +2 |
| 8 `tests-all-features` | 76 / 2160 / 0 / 0 | 76 / 2160 / 0 / 0 | ✅ +2 |

8/8 rc 0 · `SCORED == DECLARED == 8` · marker **COMPLETE** · pin identical at
open and close · `--untracked-files=no` **EMPTY**.

The `+2` was derived at the bytes (`^#[test]` in `otp_stubs/tests.rs` **15 →
17**, both named in advance), never off a diff.

Prediction pin `20b9245` vs battery pin `cf00e03`: one commit, `--numstat`
**63/0, zero Rust**.

✅ **The raw census held flat legitimately this time** — `tree pre: 21` →
`tree post: 21`, because the working notes went to scratchpad instead of into the
repo mid-run. That is the #113 disclosure corrected at the operator end rather
than re-explained.
