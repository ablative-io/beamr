# #227b beamr-wasm 0.9.0 — the cut is BLOCKED ON AN UNPUBLISHED beamr API, measured

## Verdict: battery GREEN at the pin, publish dry-run RED — different instances, and the red governs

| instrument | resolves beamr from | verdict |
|---|---|---|
| 9-leg battery at `a3b87e6` | in-tree (path dep) | COMPLETE 9/9, axes EXACT 2/86/0/0 · 76/2150/0/0 · 76/2160/0/0 |
| `cargo publish --dry-run -p beamr-wasm` | **registry 0.18.2** (path stripped at packaging) | **rc=101 — 4 × E0599** |

`beamr-wasm/src/convert.rs` calls `ProcessContext::with_accumulator` at four
sites (:159, :207, :275, :317). That method ships in `1a70068` — *"feat(ar-1):
TermAccumulator — the rooted accumulation window, additive"*, 2026-08-17 —
which is **NOT an ancestor of either 0.18.2 release pin** (`b42156c`,
`4ea651f`; both checked with `git merge-base --is-ancestor`). The compiler
measured it against the registry bytes; git corroborates the provenance. Full
dry-run capture: `publish-dry-run.log` (rc echoed in-log; the background
wrapper reported exit 0 — the wrapper's rc is the launder channel, the log is
the witness).

⚠️ The 4-error count is a LOWER BOUND (errors reveal in layers): resolving the
accumulator sites may expose further post-0.18.2 API use. Nobody should size
the delta off "4".

## Why the battery could not see this

The battery builds the workspace, where `beamr = { version = "0.18.2", path =
"../beamr" }` resolves to the path — a tree carrying ~5 days of unreleased work.
The version field is asserted, never exercised, in every in-tree build: **the
spec `^0.18.2` was a flag not in force.** Only packaging strips the path and
makes the version real. This is the adjacent-instance law applied prospectively,
and the reason the dry-run was pre-registered here as load-bearing rather than
ceremony.

## Consequence for the release commit

`a3b87e6` is **LOCAL-ONLY and must not land as-is**: its manifest and CHANGELOG
both assert "rides beamr 0.18.2", which the dry-run refutes. Held unpushed
pending the word below. The battery evidence at that pin stays valid *as a
battery* (in-tree health at those bytes) and is retained here.

## The two honest paths, neither mine to pick alone

1. **beamr 0.18.3 first** (patch line: TermAccumulator is additive, the
   Unreleased jit-catch fix rides too), then beamr-wasm 0.9.0 declaring
   `^0.18.3`, re-pinned + re-verified. ⚠️ This publishes AR-1-arc code while
   crash-2/phase-4 is still held on Cally's §8.2/§8.4 — a board word, not a
   seat call. The 0.19.0 cutter's list (site-4 raw-trio, #104 scaffold) stays
   with 0.19.0; nothing here forces it early.
2. **Seth's carve-out** at the haematite 0.18-admitting release (wasm rung
   waits), with the retirement trigger — beamr-wasm 0.9.0's publication —
   stated in the clause, per ruling r2's standard.

The finding does not authorize its own follow-up: reported to Waffles
2026-08-18, decision his/board's.
