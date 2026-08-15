# GROUND — #193 beamr JIT crash road (measured 2026-08-15, my hands)

Dispatch: Waffles DM d0303f0b 2026-08-15 ~09:03Z, on Tom's "get more
moving" word. Tom's ruling carried: "fix THE path, no interpreter dial"
(= the jit_threshold stopgap stays withdrawn; the remedy is the fixed
JIT reaching consumers, not disabling compilation). New stake: Tom is
designing scheduled estate functions as SLEEPING BEAMR PROCESSES
(sleep-for-free + supervision trees); Waffles named this road the single
genuine blocker.

CHARTER PINNED (Waffles dcc7b6b5, 2026-08-15 09:06Z): NO new crash
specimen exists — his board line predates my #88-era close and my
measured state supersedes it. #193 = the REMAINING ROAD: consumer
convergence onto the fixed set under the PIN-2 census predicate + the
soak gate + the 0.18 migration lane + my open residues (#91, AR-1,
#101). Acceptance frame (his words): "consumers converged + soak
evidence says a host can sleep for weeks without the JIT eating it."
Tom's design doc (ablative/docs/design/
supervision-and-execution-substrate-20260815.md §Dependencies): "#193
gates Class A, full stop — a scheduler host that can crash mid-sleep is
not a scheduler. Until it lands, Class A is design-only." Anything new
my re-measure turns up: mine to charter, loudly on the beamr lane.

PREDICATE RECONCILIATION TO DISCLOSE: PIN-2's ruled fixed set was
{0.16.4, 0.17.1, 0.18.2}; the advisory later ruled 0.16.4 OUT OF
EXISTENCE (0.16.x backport = redesign, refused). Effective fixed set =
{0.17.1, 0.18.2}. Subset ⇒ no weakening, but the ruled text names a
version that will never exist — record it, don't silently rewrite it.

## Measured state (all at my hands, this box, 2026-08-15)

Tags: v0.17.0 v0.17.1 v0.18.0 v0.18.1 v0.18.2 — NO v0.16.4.
CHANGELOG advisory (in-tree, ruled): 0.16.x line UNPATCHED BY DESIGN —
"backporting this fix to it was considered and ruled against" (runtime
seam absent at that base; fix = redesign there). "0.18.2 reaches no
existing consumer on its own" — every 0.x minor is a semver major;
migration = manifest edit, not lock refresh.

Consumer locks (repo checkouts on this box — deployed binaries may
differ; locks may lag origin):
- aion:      beamr 0.16.3 + beamr 0.18.2  (engine line MOVED to fixed;
             0.16.3 residual served transitively via haematite 0.7.x/
             0.8.x + liminal-rs/-server per the #88-era measured split)
- liminal:   beamr 0.16.3   — affected, jit default-on, no fix on line
- haematite: beamr 0.16.3 (+ beamr-wasm 0.7.0) — same; native rung was
             graded mitigated-unexposed (zero registered modules,
             gate-logs/beamr-jit-reachability-20260812.md in haematite)
             with MY confirmation duty noted there: "the beamr seat must
             confirm the JIT cannot fire without a registered module"
- frame:     beamr 0.16.3   — same class

Implication chain (the 0.18 migration lane's shape): liminal +
haematite + frame need manifest migrations to 0.18.2 (0.17.1 exists but
is a half-step); aion's residual 0.16.3 entry clears only when
haematite + liminal move (transitive). PIN-2 census predicate (ruled):
every beamr entry in a consumer's tracked lock ∈ {0.16.4, 0.17.1,
0.18.2} with per-entry declaration — note 0.16.4 does NOT exist, so the
fixed set is effectively {0.17.1, 0.18.2}; predicate currently FAILS at
all four consumers (aion partially).

Owners: liminal = Hermes Crumpet; haematite = Apollo Biscuit; frame =
Athena Zooper Dooper; aion = Vesper Lynd. Owners-land-own-repos — my
seat drives beamr-side, coordinates, verifies census; their seats land
their manifests. Waffles' production gate (ruled, #88 era): aion on a
fixed line, COMPILED past 3h24m with tier-up events. Soak status
unknown at my seat — ask.

## Open beamr-side residues on the crash road (my board)

- #91 F3 reachability (prologue Line instruction) — successor to #89,
  pending.
- AR-1: 17 native as_bytes sites OPEN+SIZED under AR-1-LANDING-GATE.md
  (Cally's pre-registered gate; per-site commits; 2 sites not
  absorbable). Native-path class, NOT JIT-gated.
- #101 tier-up signature — awaiting Vesper's population enumeration.
- #80 producer-set discriminator re-verdict (AR-1-adjacent).

## Hazards / rails for this lane

- Deployed-binary vs repo-lock distinction (the #88 lesson: lock cannot
  attribute; the artifact's embedded version is the datum).
- Census gates STATE THEIR SCOPE in output ("beamr entries only").
- "cargo update -p beamr says Adding not Updating" — partial bump is
  SILENT; per-entry declaration, count derived, never bare integer.
- haematite lock tracking status: re-verify before claiming delivery
  (was untracked/gitignored in the #88 era; my read above came from a
  file on disk — check `git ls-files Cargo.lock` at haematite).
- No interpreter dial: jit_threshold stopgap ruled out by Tom. Any
  proposal that reduces to "don't compile" is out of charter.

## FINDING 1 (2026-08-15, my hands at beamr main 1f5bee0) — JIT cannot
## fire without a registered module: CONFIRMED, both rungs

Native rung (haematite's grading condition — DISCHARGED):
- The hotness profiler's SOLE production feeder is
  record_jit_call_miss(profiling, module: &Module, ...) at
  interpreter/opcodes/core.rs:800-838 — inside interpreter opcode
  dispatch, requiring a live &Module whose bytecode is being executed.
  A compile request is built from module.function_instructions(entry_ip)
  — the module's OWN slice.
- Compile jobs are born ONLY from that feeder (in-tree comment at
  jit/aot.rs:270: "record_call at a live edge is the only birth path");
  job completion is the cache's production insert (compile_job.rs:86).
- Compiled-code ENTRY (invoke_jit, core.rs:782) requires a cache HIT.
  The only other insert path — AOT companion load
  (load_companion_into_cache, aot.rs:248-277) — ALSO requires &Module,
  has no production wire-up (#13 A1 pending), and its generation-0 keys
  can never match a live lookup: registry generations start at 1
  (module.rs:422 fresh=1, :405-418 monotonic ≥1, VERIFIED at bytes).
- Native processes (spawn_native) execute native slices with ZERO jit
  references (scheduler/execution/native_slice.rs — grep 0 hits).
⇒ Empty ModuleRegistry ⇒ no bytecode interpretation ⇒ no heat, no
compile, no compiled entry. Haematite native rung mitigated-unexposed
grading HOLDS. (Property of today's call sites — evaporates the day a
module is registered; the upgrade recommendation stands regardless.)

Wasm rung (Seth's stated not-covered limit — now COVERED):
- At tag v0.16.3 (the exact era haematite's beamr-wasm 0.7.0 consumes):
  beamr-wasm depends on beamr default-features=false,
  features=["cooperative","json"]; jit = [..., "threads", ...] ⇒ the
  JIT module is structurally excluded from the wasm build (this is the
  #78 amendment's mechanism, verified at the era's own manifest bytes).
  The jit_send defect CANNOT exist in the wasm rung. (The 0.16.3
  memory-safety advisory class is a separate, unrelated exposure.)

Lane post: entry 0871d5af (opener). Charter: Waffles dcc7b6b5.
