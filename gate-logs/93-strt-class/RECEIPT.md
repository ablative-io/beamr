# beamr #93 STRT-CLASS — battery receipt

Base `cdaec06` (= `origin/main`). Runner: `runner.sh` beside this file. Six
canon legs read from the committed `gates.json` **at run time**, never
transcribed.

## Verdict: 6/6 rc 0. COMPLETE marker DERIVED (legs_declared=6, legs_scored=6).

fmt 0 · clippy `-D warnings` 0 · wasm32-check 0 · wasm-tests 0 · tests 0 ·
blocking-call-in-native-bif 0.

Tree check (three-artifact form, counted with `wc`, never `grep -c`):
**4 pre, 4 post** — the four files this commit touches, unchanged across the
run. The filter declares two exclusions by name: the known untracked
`.claude/skills/` and this battery's own `gate-logs/93-strt-class/`.

Interpreter provenance, logged every run (Cally's rider):
`/usr/bin/python3` → resolved `/usr/bin/python3`, **Python 3.9.6**. This is a
macOS Command-Line-Tools shim: present here, hollow on a box without CLT. Bare
`python3` on this machine is broken outright and once let a battery score six
legs green while executing nothing.

## The defect

`encode_module` emitted `ImpT`, `ExpT` and `StrT` **only when non-empty**. Our
loader reads an absent optional chunk as empty, so such a module round-tripped
through beamr perfectly and every in-tree test passed. OTP's `beam_lib`
hard-requires all three and refuses the file with `{missing_chunk, _, "StrT"}`
**before disassembly begins**. A module beamr wrote could not be opened by
`beam_disasm`, `beam_lib`, or anything built on them.

⭐ **The cost was already paid once:** a 7,828-module ecosystem sweep silently
skipped our module, and a skip is indistinguishable from a clean result.

## The required set was MEASURED, not assumed

Each chunk stripped in turn from a working module, remainder fed to
`beam_disasm` under **OTP 29 / erts 17.0.3**. The unstripped module was run as a
**positive control** to prove the rebuild itself faithful.

* **Required** (refusal when absent): `AtU8`, `Code`, `ImpT`, `ExpT`, `StrT`.
* **Optional** (no refusal): `Attr`, `CInf`, `Dbgi`, `Docs`, `FunT`, `Line`,
  `LocT`, `Meta`, `Type`.
* **`LitT` — INCONCLUSIVE, and claimed as such.** Stripping it from a module
  that has literals leaves dangling references, so that arm is a **confound**,
  not evidence. It stays conditional.

Corroboration from the other side: OTP's own `erlc` output carries `StrT` with
length **0** — an empty chunk it emits anyway. The required-five decision
matches what the reference compiler already does.

⚠️ **`ExpT` and `ImpT` were found by generalising, not by observation.** Only
`StrT` was seen failing in the wild. Stripping proved the other two behave
identically, which is what turned a one-row fix into the class fix.

## The falsifier — the gate is shown to SEE the defect

⛔ **A green gate beside a fix proves nothing.** Two arms at this tree:

| arm | `container.rs` | encode_round_trip result |
| --- | --- | --- |
| **subject** | fix present | **6/6 pass** |
| **falsifier** | reverted to `cdaec06`, gate retained | **3 of 6 FAIL** |

The falsifier arm names 20 real fixtures, e.g.
`gleam@list.beam: missing required chunk(s) StrT; emitted AtU8 Code ImpT ExpT FunT LitT Line`.

The in-suite gate is a **presence assertion on the container**, and says so —
it is not a claim that OTP accepts the module. The end-to-end proof is below.

## End-to-end proof against OTP itself

Population: **all 24** `.beam` files in `test-workflows/sample`. Nothing
sampled. Each decoded by beamr's loader, re-encoded, and run through OTP 29
`beam_disasm:file/1`.

| arm | `beam_disasm` OK | failed |
| --- | --- | --- |
| original `erlc` bytes (**positive control**) | **24 / 24** | 0 |
| beamr re-encode, fix ABSENT | **0 / 24** | 24 (21 `missing_chunk "StrT"`, 3 other) |
| beamr re-encode, fix PRESENT | **12 / 24** | 12 (all `cannot_disasm_instr`) |

The control matters: the originals disassemble perfectly, so the failures are
beamr's re-encode and not an OTP limitation on Gleam-compiled code.

## ⚠️ What this does NOT claim — the remaining 12 are a DIFFERENT defect

**This fix does not make every beamr-emitted module disassemblable.** The
`missing_chunk` class goes 21 → 0; a second class, previously **masked** by it,
is now visible: beamr emits `Operand::TypedRegister` into `Code` while never
emitting the `Type` chunk those indices point into, so the reference dangles and
`beam_disasm` raises `cannot_disasm_instr`.

Correlation, measured through beamr's own decoder (parsed, not grepped):
fixtures with ≥1 typed register = **12**; with 0 = **12**. The 12 with ≥1 are
**exactly** the 12 that still fail; the 12 with 0 are **exactly** the 12 that now
pass. **24/24 agreement, both directions** — a mechanism, not a hopeful reading.

It cannot be fixed by emitting a chunk today: `ParsedModule` has no type table
(only `type_index` survives decode). Tracked separately as **#95**, with three
costed options and no remedy chosen here.

Also recorded so it is not rediscovered as new: every re-encoded module draws
`warning: code segment did not end with int_code_end`. A warning, not a refusal;
no module failed because of it.

## The canon `tests` leg CHANGED — declare this loudly

    - cargo test --workspace
    + cargo test --workspace --features beamr/encode

`encode` is not a default feature, and **no `gates.json` leg and no CI workflow
enabled it**. `crates/beamr/tests/encode_round_trip.rs` is
`#![cfg(feature = "encode")]`, so the entire round-trip ratchet — shipped by
ENC-001 — had **never run in the gate**. Shipping a regression test into a
binary nothing executes would be a fake remedy.

Bound on the class, measured: of the **8** test binaries whose whole file is
feature-gated, **7** are gated on DEFAULT features (`threads`, `readiness`) and
**exactly 1** — this one — was on a non-default feature. The whole-*file* class
is closed. Inline `#[cfg(feature = …)]` test modules behind non-default features
(`telemetry` alone has 156 cfg references) are **NOT** measured here and are
**#96**. That inference is exactly what #96 exists to prevent.

## Axes — pre-registered, and one prediction was WRONG

Pre-registration written **before** the run (scratchpad
`AXES-PREDICTION-93.txt`). Both commands were then run on the **same tree**:

| | old leg | new leg |
| --- | --- | --- |
| result-lines | 73 | **73** |
| passed | 2094 | **2107** |
| failed / ignored | 0 / 0 | **0 / 0** |

`passed` **2107 — predicted exactly**, +13, every one named: 7 unit tests under
`src/loader/encode/`, and 6 in `tests/encode_round_trip.rs`
(`every_fixture_round_trips_through_encode`, `corpus_round_trips_when_env_set`,
`hand_built_edge_cases_round_trip`, `empty_optional_chunks_round_trip`,
`required_chunks_are_emitted_even_when_empty` **(new)**,
`the_required_chunk_check_detects_an_absent_chunk` **(new)**). Only **2 of the
13 are newly written**; the other 11 are pre-existing tests running for the
first time.

`result-lines` **73 — I predicted 74. My prediction was wrong**, and the reason
is the finding. Binary sets from both runs were diffed: 72 vs 72, `comm` empty
on both sides.

⭐ **`encode_round_trip` was ALWAYS built and ALWAYS run.**
`#![cfg(feature = "encode")]` empties a file's *contents*, not its existence as
a target. With the feature off it printed `running 0 tests` /
`test result: ok. 0 passed` — and that counted as a result-line every single
run. **The result-line axis cannot distinguish "ran" from "ran nothing", and
`ok. 0 passed` is the same green as any other.** This is the six-legs-green-
having-executed-nothing shape one level down, at the test BINARY rather than the
LEG; the DERIVED COMPLETE marker guards the leg version, nothing guarded this
one. Zero-passed result lines went 5 → 4; the remaining 4 were checked and are
structurally empty (`src/lib.rs`, `src/main.rs`, `beamr_wasm`, `gleam_types`
doc-tests), not feature-emptied.

**Forward rule for priors:** when a leg change enables a feature, predict the
**passed** axis. Result-lines move only when a test *target* is added or
removed — never when one is emptied or refilled.

## Provenance of the base prior

The old-leg baseline `73 / 2094` was **inherited** from
`gate-logs/0182-release-rerun/RECEIPT.md` — and then **re-measured at this tree**
rather than trusted, giving exactly `73 / 2094`. The inherited prior was
correct, and it was checked rather than assumed.
