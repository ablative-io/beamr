# WPORT-9 build STOP #1 — the R5 panic leg — finding and ruling

**STOP (Osiris Yogo, 2026-07-24, brief STOP condition 1):** R5's panic leg
is underdetermined at the bytes. The spec's premise — "drive the committed
panic route (the WPORT-7 probe's SYNC-entry shape)" — is false at the
base: NO committed JS-reachable deliberate-panic surface exists in
beamr-wasm. The WPORT-7 sitting drove its panics via the UNCOMMITTED
panic-source diff, which is precisely why that diff was committed as
evidence beside the observations
(`probes/evidence/2026-07-18/wport7-panic-source.diff`, sha-pinned).
Options presented: (a) reclassify — T1 walls + sitting carry the claim,
ledger records why; (b) amend to add a committed panic surface; (c)
induce a panic through an existing edge (rejected by the reporter: any
such edge is a bug, not a mechanism).

**RULING (Artemis Peach, domain owner, 2026-07-24): OPTION (a), RATIFIED**
— with a strengthening that makes it the only honest option: a permanent
T2 panic leg drives the REAL bundle, and a test-cfg'd or debug-only panic
export does not exist in the real bundle — so any permanent leg would
structurally require a SHIPPING deliberate-panic surface, which no
consumer requirement justifies (the D7/WPORT-10 discipline, same as
provenance). Option (b) is not entertained even with the boundary
relaxed; (c) rejected as "a bug with a dependency, not a mechanism."

**The reclassification as ruled:** panic-surfacing = T1 walls
(`panic_hook_installs_exactly_once_across_vm_constructions`,
`panic_reaches_console_and_registered_callback_before_the_trap` —
verified at the ruler's hands, `failure_tests.rs:425`/`:456`) + the
WPORT-7 official sitting with its committed evidence diff as the
real-engine record. The ledger row records WHY no permanent T2 leg
exists. Revival condition: a consumer requirement for a JS-reachable
panic route + the domain owner's word. R5 keeps its output and
process-error legs. The brief text stays as-landed — the ledger and this
record carry the reclassification (amendment-in-ledger, the R8
mechanism; no brief re-land).

**Second finding, accepted IN-SPEC (R2's escape clause):** trapped-exit
and native-completion wake classes are unreachable through the real
bundle from bytecode — the ruler re-verified the latent-gap comment at
`scheduler/wasm.rs:566-575` (bytecode exits perform no link propagation;
unreachable because the cooperative facility refuses link-bearing spawn
variants; the guarding wall belongs to the future bytecode-linking
brief).

## Ledger rows minted by this build (derivation record for R8's close-fold)

- **Panic surfacing — T1 + sitting, no permanent T2 leg.** Walls:
  `panic_hook_installs_exactly_once_across_vm_constructions`,
  `panic_reaches_console_and_registered_callback_before_the_trap`.
  Real-engine record: WPORT-7 official sitting 2026-07-18 (committed
  panic-source diff, sha-pinned). Reason: no committed panic route in the
  real bundle; creating one is shipping surface without a consumer.
  Revival: consumer requirement + domain owner's word.
- **Trapped-exit wake — T1 only from the real bundle.** Wall:
  `trapped_exit_wakes_linked_supervisor_without_external_pump`
  (native-handler trapping). Reason: no link facility is injected for
  bytecode processes; `spawn_link/3`/`process_flag(trap_exit,_)` refuse
  from bytecode; bytecode exit arms perform no link propagation
  (`scheduler/wasm.rs:566-575`, documented latent gap owned by a future
  bytecode-linking brief).
- **Native-completion wake — T1 only from the real bundle.** Walls:
  `native_completion_envelope_wakes_parked_handler_through_the_arbiter`,
  `native_completion_direct_injection_wakes_parked_handler`. Reason:
  native handlers are Rust-side constructs with no JS surface; the real
  bundle cannot mint one.
- **Exited/errored classification (build observation, banked not owned):**
  interpreter raises from bytecode (badmatch witnessed at the bytes)
  classify as EXITED with empty result — the WPORT-7 board finding
  (arc `:146`) biting in practice; undef classifies ERRORED and carries
  the typed `take_exit_error` shape, which is why the R5 process-error
  leg uses undef. Ownership unchanged: the future brief that owns the
  exited/errored vocabulary.
