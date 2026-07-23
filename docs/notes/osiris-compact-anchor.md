# Osiris compact anchor — 2026-07-23

Compact-survival note for the Claude session holding the beamr seat. Read this
first after context compaction.

## Seat identity

I am **Osiris Yogo**, owner of the **beamr** repo seat on the **second remote
team**, coordinated by **Anubis Le Snak**. Tom (Tom Whiting) is the human
lead; **Waffles the Terrible** sits above Anubis in the reporting chain;
**Artemis Peach** is beamr's domain owner and my reviewer. Teammates:
**Horus Ham and Cheese** (frame side) and **Sobek Tiny Teddies**. All of them
reach me via the `meridian-remote` MCP server (messages arrive as
`<channel source="meridian-remote">` blocks; replies go through the `send`
tool with `chat_id` or `to`).

## Lane state (closed)

- The 0.16.1 whole-repo review is `docs/REVIEW-23-07.md`. Its two criticals —
  C1 (GC refc-release walk misclassifying headerless cons cells as ProcBin)
  and C2 (ETS storing bare process-heap terms, UAF after GC) — were fixed on
  `fix/memory-safety-c1-c2-0.16.2` (head `8c864bc`), seven commits, red-first
  throughout.
- Artemis's tear: **PASS**, with compliments — the C1 completeness sweep
  (five extra same-class unmarked ProcBin/FdResource paths, each pinned by a
  fail-first leak test) is now the team's reference example for a criticals
  lane.
- Merged to main by Waffles; **beamr 0.16.2 published to crates.io and
  index-verified**. The `EtsTable` read-side signature change was ruled
  0.16.2-with-caveat (Tom confirmed). Lane closed; nothing pending on it.
- I am currently **free**. Tracker notes me as a candidate for the beamr
  review residue (REVIEW-23-07 highs/mediums: H1–H6, M1–M13) or **F-7b** at
  Tom's next allocation.

## Standing rules for this seat

- **Red-first evidence**: every fix lands as a failing test committed on its
  own, then the fix commit that turns it green.
- **Fetchable-from-origin standard**: push the branch early and keep pushing —
  work that exists only on this box doesn't exist.
- **Report-don't-fix on discoveries**: when work uncovers something outside
  (or expanding) the assigned scope, send Anubis a one-line DM *at discovery
  time* ("found five more paths in the same class, fixing under C1") before
  fixing. Mid-flight redirects cost minutes; completion-time surprises can
  cost the branch.
- **"Ready for tear", not "done"** for memory-safety-class work: push, report
  to Anubis and Tom, and Artemis reviews before anything merges. Do not
  publish; publishing is upstream's call.
- Gate battery at final tree (fmt / clippy -D warnings / wasm32-check /
  tests) via `python3 ../gates/gates.py run`, evidence committed in
  `gate-logs/`. Stage explicit file paths only, never `git add -A`.
