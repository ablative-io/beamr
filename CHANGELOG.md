# Changelog

## Unreleased

## 0.16.2 — 2026-07-23

Memory-safety patch: the two critical findings from the 2026-07-23 external
review (docs/REVIEW-23-07.md), fixed red-first and torn PASS before release.

### Fixed

- **C1 — GC refc-release walk could free foreign memory.** The release walk
  inferred object type from `word[0]`, but a headerless cons whose head is
  atom `false` (encoding 0x19) is indistinguishable from a `BoxedTag::ProcBin`
  header — the walk then executed `Arc::from_raw` on a heap-cons payload:
  arbitrary free + heap corruption at the next minor GC. Fixed structurally:
  every allocation now records an `AllocKind` at allocation time and all three
  release walks (minor young-region, process-death `release_all`, major
  compacted-sources) visit only allocations marked refcounted. `word[0]`
  inference is retired entirely; the fail-safe direction is documented at the
  type (a missed mark can only leak, never free foreign memory). A
  completeness sweep marked five additional refcounted-into-process-heap
  paths the review had not named (mailbox proc-bin delivery, tcp active +
  closed socket messages, udp, io results, jit sub-binary extraction), each
  pinned by a fail-first leak test. The debugger's heap census gets an
  explicitly inspection-only unfiltered walk, documented never-for-release.
- **C2 — ETS stored borrowed caller-heap terms.** `insert` kept `Term`s
  pointing into the inserting process's heap, so a post-insert GC (or process
  death) left tables reading freed/moved memory. ETS now deep-copies terms
  into table-owned storage on insert (`ProcBin`/`SubBinary` flattened to
  table-owned inline bytes — tables hold no Arcs) and copies out to the
  caller's heap on read. Map keys own their own copies (`OwnedEtsKey` /
  `OwnedTermKey` with structural `Borrow`, so probes never copy).

### Caveat

- `EtsTable`'s public `lookup`/`tab2list` signatures changed — required by
  the soundness fix (bare `Term`s in the trait contract WERE the bug).
  Technically breaking inside a patch release; no known external
  implementors or callers (verified across haematite, liminal, frame, aion,
  beamr-wasm — all touch only `OwnedTerm`/`copy_term_to_ets`, unchanged).
  Precedent: Rust's soundness-fix policy (RFC 1122).
- ETS round-trips now flatten large binaries inline: Arc sharing is lost
  through insert/lookup, a memory cost on big-binary tables. Documented in
  code; revisit candidate for the 0.17 window.

## 0.16.1 — 2026-07-23

### Added

- `Scheduler::spawn_native_trap_exit` and `Scheduler::spawn_native_link_trap_exit`:
  native processes spawned with `trap_exit` set BEFORE they are made runnable —
  the native mirror of bytecode `spawn_trap_exit`. Closes the spawn-then-set
  window in which host-side `set_trap_exit` returns `NoCaller` for a freshly
  spawned native that is transiently `Executing` mid-first-slice (`NoCaller`
  conflates that with a truly-dead process, so callers could not retry
  honestly). Existing `spawn_native`/`spawn_native_link` behavior is
  byte-identical (they delegate with the flag off; the `Process` default is
  `false`). Found as a once-per-battery race in liminal's subscriber spawn on
  high-core hosts; consumers using spawn-then-`set_trap_exit` on natives
  should migrate to the new entry points.

## beamr-wasm 0.7.0 — 2026-07-23

- Rides beamr 0.16.0 (dependency spec `0.15.0` → `0.16.0`) so downstream
  wasm consumers resolve ONE beamr per lock. No API changes of its own.

## 0.16.0 — 2026-07-23

The gleam-on-beamr enablement release: a real `gleam_otp` 1.2.0 actor
(genuine gleam-built beams) starts, takes casts, and answers a synchronous
`actor.call` on beamr. Four latent interpreter defects were surfaced by that
spike's refusal to fake past an `undef` and are fixed here; none was reachable
by any consumer's production path on 0.15.x (verified by grep-and-trace plus a
disassembler census over every consumer's loaded bytecode), but all four sit on
the hot path the moment `gleam_erlang`-based code loads.

### Added

- `proc_lib:spawn_link/1` BIF over the existing fenced closure-spawn facility
  (admission honored; a bare `run()` without a scheduler refuses with a typed
  error rather than pretending).
- `receive ... after infinity` — the `infinity` atom now selects an unbounded,
  timer-free wait (previously `badarg`), matching `wait` semantics with no
  polling construct.
- Multi-clause functions admit to the JIT: the admission guard's
  effect-before-deopt analysis is now CFG-sensitive (forward may-reach
  dataflow over the slice block graph, union join, fixpoint) instead of
  linear-slice-order, so mutually exclusive clause exits no longer
  false-positive. A single blocking receive is now admissible under
  path-sensitivity, with a positive differential through the demand path.
  The slicer retains the `func_info` prelude and lowers `FuncInfo` as a
  DEOPT terminal.
- `docs/design/beamr/probes/raiser-scan/` — rerunnable no-match raiser census
  instrument (beam_disasm scanner + probe fixtures) used as this release's
  check over consumer bytecode; positive-control verified.

### Fixed

- `erlang:send/2` silently dropped every cross-process local send (the
  ignored `send_to_attached_self` return). It now routes through the
  `LocalSendFacility` exactly like the `send` opcode — slot-locked delivery,
  sender clock ticked, replay-valid. Anything driving `erlang:send/2` from
  loaded bytecode (gleam `process.send` compiles to it) was affected.
- `func_info` set the current MFA and fell through — a multi-clause no-match
  re-dispatched forever, spinning a scheduler core. It now raises catchable
  `error:function_clause` (bare atom, BEAM semantics), watchdog-proven
  terminating.
- `if_end` raised `error:{if_clause, []}` where BEAM raises the bare atom
  `if_clause` — a loaded `catch error:if_clause` failed to match and the
  process died where BEAM recovers. Bare atom now; the unit test that pinned
  the wrapped shape is re-pinned to the true one.
- `gleam_erlang_ffi:demonitor/1` rejected boxed references (stale
  small-int-only parse — the same class as the 0.15.4 monitor fix). Dual
  parse now: boxed `ReferenceRef` first, legacy small-int fallback.
- JIT: a reachable deopt-after-side-effect divergence through the wired
  demand path (RecvMarkerReserve) is guarded — the whole class is rejected
  by the effect-reachability analysis above, with the replay probe green.

### Removed

- **BREAKING:** the native `gleam_erlang_ffi` selector shadow
  (`register_selector_bifs` and the `selector_ffi` module) is retired. It was
  pinned to a pre-1.3 `gleam_erlang` selector protocol and silently returned
  wrong shapes under 1.3.x; the selector family is now served by the loaded
  `gleam_erlang_ffi.beam` bytecode shipped with the user's `gleam_erlang`.
  With no bytecode loaded the family fails as an honest, catchable
  `error:undef` (pinned by regression test) instead of a silently-wrong
  value. Embedders that registered the shadow should simply drop the call —
  the maps BIFs the bytecode path needs are all present.

## 0.15.4 — 2026-07-18

### Added

- Additive registry API: `BifRegistryImpl::replace_existing(module, function,
  arity, native_fn, capability) -> Result<NativeEntry, NativeReplacementError>`
  — atomic replacement of an already-occupied MFA returning the previous
  `NativeEntry` whole (function + capability, directly delegatable). A
  replacement, not an upsert: a vacant MFA returns typed
  `NativeReplacementError::MissingMfa` and leaves the registry observably
  unchanged. The occupied-decision and the swap share one map-entry write
  guard (no lookup-then-replace gap; linearizes at the entry insert), a racing
  lookup observes the whole previous or the whole replacement entry, never
  torn, and no error path can vacate the slot. Intended for registry
  construction (install the complete BIF table, then fence selected MFAs
  before starting scheduler workers) — the driver is aion's spawn-reservation
  fencing of `erlang:spawn/1` / `erlang:spawn_link/1` over the complete Gate-3
  table. Scoped to normal-scheduled BIFs: the replacement entry is written
  with `dirty_kind: None` (the returned previous entry keeps its `dirty_kind`
  intact). Gate tables, scheduler, wasm, and exit surfaces untouched
  (insertion-only change).

## 0.15.3 — 2026-07-18

### Added

- Additive Scheduler exit-observation API: `take_exit_outcome(pid)` — a
  non-blocking, consuming, exactly-once take of a process's `(ExitReason,
  OwnedTerm)` outcome, backed by a durable per-process finalization token that
  survives both legacy tombstone FIFO eviction and the take itself (permanent
  residue pinned at 40 bytes per finalized process, test-asserted) — and a
  single-subscriber bounded exit-event stream (1,024 events) whose `Exited`
  notification is published only after the outcome is takeable, with typed
  `Lagged` overflow and scan-based recovery. Existing exit surfaces
  (`run_until_exit`, `peek_exit_reason`, diagnostic takes, `terminate_process`)
  are semantically unchanged.
- `ExecError::GuardBifUnavailable` with a typed four-arm `GuardBifResolution`,
  exact-string diagnostics through `format_with_atoms`, and an
  `unresolved` import report on `HotLoadResult` (which consequently no longer
  derives `Copy`) — the EMB-001/EMB-002 pair.

### Changed

- The `LitT` chunk emitted by the `.beam` encoder (`encode` feature) changed
  to the tear-ruled **candidate C zero-prefix uncompressed form** (ENC-001): a
  zero u32 size prefix followed by the raw literal-table bytes, replacing the
  zlib-compressed body produced through 0.15.2. The emitted bytes are now a
  pure function of the literal table — no compressor, no ambient
  `Compression::default()`, no dependency-resolved variance — and are
  byte-symmetric with the form the erlc/gleam toolchain emits across the
  committed fixture corpus. Consequences: emitted `.beam` bytes differ from
  0.15.2's output for any module carrying literals, so downstream content
  hashes over emitted bytes (e.g. aion package version identity) shift ONCE on
  the next recompile — expected and correct, never a regression. The decode
  side is unchanged and loads both forms: legacy compressed `LitT` chunks keep
  loading forever.

## 0.15.2 — 2026-07-15

### Fixed

- `make_fun` and `put_map` now route near-full-heap allocations through the
  GC's `ensure_space` safety net (collect, then grow) instead of calling the
  raw heap allocator, which surfaced `heap full: requested N words with M
  available` as a fatal VM execution error. Hit in production by aion's first
  direct-BEAM AWL child workflow: any process whose nursery is near-full at a
  `make_fun` died ~150ms after spawn. Both reservations run before the
  instruction's terms are copied into Rust locals, so a safety-net collection
  cannot leave the closure's free variables or the source map dangling.
- DOWN-message heap reservations under-counted by one word (7 reserved for
  the 8-word local message, 11 for the 12-word remote one). A watcher heap
  with exactly the reserved word count free failed the final tuple allocation
  and the watcher was killed instead of receiving `{'DOWN', Ref, process,
  Pid, Reason}`.

## 0.15.1 — 2026-07-13

### Fixed

- `SupervisionFacility::monitor` against an already-tombstoned target now
  delivers the immediate DOWN through the same dual-slot admission path as
  normal exits: a `Present` watcher is enqueued and woken after the message is
  visible, and an `Executing` watcher receives it via `pending_down_messages`
  merged at store-back. Previously the DOWN was silently dropped for any
  watcher not in the `Present` slot — including every native host observer
  registering while executing — while the result still claimed
  `immediate_down: true`.
- `MonitorResult::immediate_down` is now truthful: it reports whether the DOWN
  was actually admitted, and is `false` when the watcher slot is absent or the
  watcher has already exited.

### Added

- `Scheduler::monitor_with_result(watcher_pid, target_pid)` returns the full
  `MonitorResult` so embedders can observe the immediate-DOWN case. The
  existing `Scheduler::monitor` keeps its signature and delegates to it.

## 0.15.0 — 2026-07-13

### Added

- `Scheduler::send_to_mailbox(pid, OwnedTerm)` is the public threaded-runtime
  host-to-process message primitive. It deep-copies arbitrary owned terms into
  the receiver heap, preserves FIFO with existing atom/timer deliveries, and
  wakes a waiting receiver only after the message is visible. Delivery racing
  an executing slice is merged at store-back and observed by the receiver's next
  receive without a lost-wake window.
- `MailboxSendError` replaces boolean ambiguity for the new API with typed
  `NoSuchProcess`, `ProcessTerminated`, `ProcessSlotUnavailable`,
  `HeapAllocationFailed`, and `InvalidMessage` failures.

## 0.14.0 — 2026-07-12

**The artifact of record for the embedder-composition campaign** (composition
commits 1–6: `docs/EMBEDDER-COMPOSITION-SPEC.md`, `docs/READINESS-CONTRACT-SPEC.md`,
`docs/READINESS-REGISTRATION-API.md`). This version contains exactly the tree
0.13.2 shipped; the minor bump exists because the campaign's public-API breaks
below cannot honestly ride a patch version — which is also why **0.13.1 and
0.13.2 are yanked** (see their entries).

### Breaking

- `Scheduler::dirty_cpu_pool` / `dirty_io_pool` are replaced by
  `try_dirty_cpu_pool` / `try_dirty_io_pool` returning `Option<&DirtyPool>` —
  a composed scheduler can genuinely have no pool, and the old signatures
  could not represent absence without panicking. `#[doc(alias)]`es preserve
  discoverability under the old names.
- `Scheduler::distribution_connections` / `distribution_config` are replaced
  by `try_distribution_connections` / `try_distribution_config` (same reason:
  `distribution: None` now builds NOTHING — honest absence).
- `ExecError::ServiceUnavailable { service }` — new variant, breaks exhaustive
  matches. Raised when a native dirty call reaches a `Disabled` dirty pool;
  the BEAM-visible exit reason stays the plain `error` it always was.
- `SpawnError::SchedulerTearingDown` — new variant, breaks exhaustive matches.
  Every spawn facility entry point refuses (rather than mutating) once the
  scheduler's teardown drain has closed admission.
- `dirty_cpu_threads: Some(0)` / `dirty_io_threads: Some(0)` now mean
  **Disabled** (zero threads, typed refusal at the pre-suspension gate);
  previously zero rounded up to one thread. `None` keeps the eager legacy
  defaults for one release (see the `Scheduler::new` migration note below).
- Ancillary-service thread names are now service-distinct (each ring names
  its own workers; `DEFAULT_RING_THREAD_PREFIX` is the exported default, and
  `create_ring_with_prefix` / `try_create_ring_with_prefix` exist for
  embedder-named rings). Anything keying on the old shared thread names must
  update.
- New **default feature `readiness`** (requires `threads`; adds the
  already-in-lock `mio` as a direct dependency). Default-features consumers
  compile the readiness registration service; the SERVICE stays composed-off
  unless selected (`FromConfig` ⇒ `Disabled`) — feature-compiled and
  service-enabled are deliberately different defaults (registration-API doc
  §8 OQ-1). `default-features = false` consumers (wasm/cooperative) are
  untouched.

### Added (campaign surface beyond the composition entrypoint)

- Per-service `ServiceMode` model with a stable identity per instance, and
  `Scheduler::service_inventory()` — every ancillary service reports mode,
  configured-vs-actual thread counts, thread names, and fd classes;
  transient-thread classes report as policy lines. The §5 permanent
  assertions (inventory ≡ OS probe, signed 5 ms idle floor with its
  `IDLE_PARK_TIMEOUT` / `IDLE_WAKES_PER_SEC_PER_WORKER` linkage) pin it.
- **Readiness registration service** (composition commit 6, contract §3
  shape (b)): register an fd + durable atom marker for a pid, get woken by
  marker enqueue on readiness — the enif_select-class primitive that lets an
  idle consumer park instead of poll. One poll thread per service instance;
  `Owned` per scheduler or ONE `SharedReadiness` injected across many
  (delivery routes home by registration identity; generation-minted
  `ReadinessToken`s make fd reuse safe). In-slice surface:
  `ProcessContext::readiness_facility()` / `NativeContext::readiness_facility()`
  (register + rearm); host-side acknowledged `Scheduler::readiness_deregister`.
  Poll-thread death degrades to typed `ReadinessError::ServiceFailed`
  refusals, honest `actual: 0` inventory, and bounded deregistration.
- Teardown-admission gating across mutating facilities: dirty submissions,
  every spawn path, message/link delivery, ETS create/delete/transfer,
  group-leader set, supervision exit signals, timer arm/cancel, and readiness
  register/rearm all hold an admission the shutdown drain waits out — a
  mutation cannot land after teardown returns, and post-drain calls refuse
  typed (`ReadinessError::TeardownInProgress` for readiness; existing typed
  surfaces elsewhere).
- `ConnectionManager::disconnect_all()`; per-connection teardown that
  actually closes: an atomically-CLOEXEC teardown dup + `shutdown(2)` makes
  connection closure independent of a wedged writer mutex (budget: one extra
  fd per live connection, ledger-signed).

### Added

- `Scheduler::with_services(config, services, module_registry)` and
  `with_services_and_code_server(..)` — the additive composition entrypoint
  (spec §2.2). `SchedulerServices` describes each ancillary service (dirty CPU/
  IO pools, file/standard/generic IO rings, distribution bundle) with a
  per-service choice; an explicit choice WINS over the matching legacy
  `SchedulerConfig` knob, a `FromConfig` choice defers to it. Non-service knobs
  (`thread_count`, node identity, queue depth, telemetry, private data) always
  apply. `SchedulerConfig`'s existing fields and exhaustive-literal shape are
  unchanged — this is purely additive.
- Named profiles: `SchedulerServices::full_runtime()` (today's full standalone
  VM — every service `FromConfig` plus distribution turned on with a default
  config), `SchedulerServices::minimal()` (every ancillary service `Disabled`:
  no dirty pools, no ring, no process 0, no distribution — only the requested
  normal workers run), and `SchedulerServices::from_config()` (the legacy
  profile that `Scheduler::new` maps to).
- Shared dirty pools: inject an embedder-owned `Arc<DirtyPool>` into several
  schedulers with `SchedulerServices::shared_dirty_cpu` / `shared_dirty_io`.
  The pool is used by each scheduler but joined by NONE of them (the embedder
  owns teardown). Safe now because dirty completion routes by the oneshot the
  submission carries, not by any per-scheduler table.
- `SharedIoRing` + `WithServicesError`: the injectable shared-IO-ring handle and
  the typed refusal `with_services` returns for it. Shared IO rings are refused
  this release — cross-scheduler completion routing lands with the §3.9 routing
  gate in a later commit — so the composition surface is complete and the
  refusal is loud and by name rather than a silent misroute.

### Changed

- The `beamr-cli` runner opts into `SchedulerServices::full_runtime()` instead
  of setting `distribution: Some(default)` on the raw config directly. No
  user-visible behavior change.

### Behavior changes (migration)

- **`Scheduler::new` is the legacy profile for one release.** It preserves
  today's EAGER per-knob defaults (a `num_cpus`-sized dirty CPU pool, a
  10-thread dirty IO pool, a live file-IO ring, a live standard-IO ring with
  process 0) as a migration bridge. Embedders that want a specific service
  footprint should move to `Scheduler::with_services` with `minimal()` /
  `full_runtime()`. Distribution already follows `config.distribution` honestly
  (`None` builds neither runtime, since 0.13.0).
- **Replay disables distribution entirely.** Under replay a
  `distribution: Some(config)` bundle is now `Disabled` — NEITHER the outbound
  sender nor the net-kernel runtime is built (previously the net-kernel runtime
  was still constructed, whose live `connect_node` dial performed real network
  IO behind a disabled facade during replay). No replay path reads live
  distribution state; every distribution BIF already resolves to absence
  (`noconnection` / `false` / `[]`). The one observable flip: `is_alive/0`
  reports `false` under replay (spec-§3.6-consistent for a node with no
  distribution service).
- **Distribution is one bundle behind one manager.** The outbound sender and
  the net-kernel now share a single heartbeat-enabled `ConnectionManager`
  (previously two disjoint connection tables). Direct remote sends are
  bounded: a wedged peer writer yields `NoConnection` + connection retirement
  at the drain's 5 s write timeout instead of hanging; `connect_node` carries
  a 15 s whole-attempt deadline; `is_alive/0` is `true` only with a live
  distribution service AND a non-default node name. Teardown joins the
  runtime workers to completion (no leaked runtime threads), safe from any
  calling context.
- Process 0 (group-leader IO server) is registered exactly when the
  standard-IO ring is `Owned`; under `minimal()` there is no process 0, and
  top-level group leaders seed from a dead-leader sentinel rather than
  self-queueing IO forever.

### Fixed

- A kill landing in the store→register park gap left a dead pid's wait-set
  entry behind forever, and a link-cascade kill of a stored process stranded
  its body, owned fds, pg memberships, and metric state while silently
  dropping its remote-link EXITs. Process finalization is now exactly-once
  behind two ownership tokens (table token for pid-keyed work, body token for
  resource release), and the cascade path finalizes like a direct kill.
- The readiness deregistration epoch handshake had a lost-wakeup window (a
  notify could land between a waiter's predicate check and its wait, with no
  second chance on a tickless poller); every predicate-state writer now
  passes through the epoch lock before notifying.
- The readiness in-slice surface was reachable from BIF-path
  `ProcessContext` but not from native-handler `NativeContext` — caught by
  the first external consumer, fixed the same night, and the
  first-external-consumer gate (an integration test consuming public paths
  only) is now standing verification doctrine.

## 0.13.2 — 2026-07-12 [YANKED]

0.14.0's tree, released under a patch version: 0.13.1 plus the readiness
§1.4 conformance fix (`NativeContext::readiness_facility`). **Yanked
2026-07-12** because the composition campaign's public-API breaks (see
0.14.0 § Breaking) cannot ride a patch bump. Use 0.14.0 — it is the same
tree with an honest version.

## 0.13.1 — 2026-07-12 [YANKED]

First release of the embedder-composition campaign (composition commits 1–6,
including the readiness registration service). **Yanked 2026-07-12**, same
reason as 0.13.2: breaking API changes under a patch version. Use 0.14.0.

## 0.13.0

Distribution grows real cross-node supervision: LINK/UNLINK/EXIT/EXIT2 now
travel the wire (previously only SEND/REG_SEND/PG_UPDATE did — cross-node
links compiled but never delivered a death signal), backed by a
multi-subscriber connection-event hub and a generation-pinned must-deliver
control lane. Specs: `docs/CONN-EVENTS-HOOK-SPEC.md` and
`docs/DIST-CONTROL-WIRE-SPEC.md` (each carries an as-built addendum recording
where the landed code deviates); decision record: ADR-012.

### Added

- Connection-event hub (`docs/CONN-EVENTS-HOOK-SPEC.md`):
  `ConnectionManager::subscribe_connection_events` /
  `subscribe_connection_events_with_snapshot` / `unsubscribe_connection_events`
  deliver generation-tagged `NodeUp`/`NodeDown` events to any number of
  subscribers with per-node alternation and exactly-once-per-session
  guarantees. `NodeUp` carries `peer_creation` so subscribers can distinguish a
  peer VM restart (all remote pids dead) from a connection blip (pids
  survive). The snapshot variant synthesizes catch-up `NodeUp`s for
  already-live sessions under a stitch-race-free gate — subscribing late
  misses nothing and double-sees nothing. Dispatch is synchronous on the
  transition thread (events are facts by the time `register_connection` /
  `connection_down` returns) with owner-thread reentrancy; the pre-existing
  single replace-on-register hook slot is now a compatibility facade over the
  hub (registered last, byte-stable semantics for 0.11-era embedders).
- A peer VM restart that re-dials before the old socket dies (live
  displacement or canonical-arm bounce with a changed `creation`) now closes
  the old session properly: `NodeDown(old)` + `NodeUp(new)` both fire, pg
  groups are purged, and `noconnection` reaches linked processes — previously
  the redial coalesced silently into the stale session.
- Cross-node link supervision on the wire (`docs/DIST-CONTROL-WIRE-SPEC.md`):
  OTP control opcodes LINK=1, EXIT=3, UNLINK=4, EXIT2=8 encode/decode and
  deliver. `Scheduler::link_remote` / `unlink_remote` establish and sever
  links whose EXIT signals actually cross the wire; exit reasons map per OTP
  semantics (`kill` crosses as `killed` on link-EXIT, raw on EXIT2), trapping
  targets receive `{'EXIT', From, Reason}` with a correctly-built external-pid
  source, and delivery contracts DC-1..DC-6 pin exactly-once semantics: for
  every established link, a dying peer process yields exactly one of {wire
  EXIT, `noconnection` backstop} — never zero, never two.
- Must-deliver control lane: link controls ride a dedicated 256-slot
  generation-pinned queue with a biased drain (controls before data). The lane
  cannot silently drop: overflow marks the pinned connection down
  (`ConnectionDownReason::ControlOverflow`, new variant) so the `noconnection`
  backstop delivers what the wire could not. This replaces the data path's
  silent-drop-at-1024 behavior for supervision traffic.
- `ExitReason::NoProc`; `RemotePid` link endpoints normalize `serial` to 0 at
  the facility boundary (documented on `link_remote`) so an
  embedder-constructed nonzero serial cannot dodge the EXIT-delivery equality
  gate.
- Telemetry counter `beamr.distribution.control_frames_dropped` (reason
  attribute) for malformed/misaddressed inbound control frames; heartbeat
  keepalives are excluded.

### Fixed

- Remote-link removals recorded while the target process was mid-slice
  (Executing) were silently resurrected at store-back — the checkout merge was
  add-only. Consequences before the fix: `unlink/1` on a remote pid was a
  deterministic local no-op, and the exactly-once EXIT gate could double-fire
  (a second spurious `noconnection` after a real wire EXIT, killing a
  non-trapping process that had survived a `normal` exit). Store-back now
  reconciles removals with metadata authoritative, mirroring the monitors
  merge.
- Remote EXIT delivery to a trapping process built the external source pid,
  then crossed a GC-capable allocation without rooting it — under nursery
  pressure the delivered `{'EXIT', From, Reason}` tuple held a dangling `From`.
  The pid and tuple are now one contiguous allocation behind one reservation.
- An inbound LINK racing a write-side connection-down (write timeout, control
  overflow, `disconnect_node`) could establish a link the backstop scan had
  already passed — the death signal was lost forever. The apply now rechecks
  the origin connection post-establish and delivers the missed `noconnection`
  (the exactly-once gate keeps both race orders single-delivery).
- Local pids beyond the wire's u32 range are refused at `link_remote`
  (`RemoteLinkError::BadTarget`) instead of tearing the whole connection down
  on every outbound control after the pid counter passes 2^32.

### Removed

- The orphaned test-only control planes `distribution/control_lifecycle.rs`
  and `distribution/control_monitor.rs` (never wired to the wire; their
  numeric opcode-table test moved to `control_link.rs`) and the scheduler's
  `ControlRouter` (accumulated EXITs into a never-drained queue). The landed
  wire path replaces all three.
- `DistributionFlags::offered()` no longer advertises `ATOM_CACHE` — the codec
  never implemented cache references, so offering it invited undecodable
  frames from spec-conforming peers. Accepting it from peers is unchanged.

### Known limitations (deliberate, recorded)

- Remote monitors stay at local-only semantics this release: `BEAMR_MONITOR`
  opcode 102 is reserved in the codec and rejected on the wire; the monitor
  stage needs external-pid plumbing at the BIF layer first (spec §1.3).
- Links are node-keyed, not generation-keyed: a link established in the
  narrow window while a `NodeDown(g)` is dispatching after a redial installed
  session g+1 can be spuriously severed. The fix needs per-link session
  pinning through public API shapes — deferred for a design ruling rather
  than rushed (DIST-CONTROL-WIRE-SPEC as-built addendum, finding W2).
- The kill-9 verification harness (true SIGKILL of a subprocess peer, as
  opposed to in-process socket-drop e2e — which this release does test) is
  deferred to a follow-up work item.
- `cargo check -p beamr --no-default-features` is red with 1020 pre-existing
  no-std errors, byte-count-identical from baseline ec5d7f8 through this
  release — the series introduced no new breakage, but that gate leg is
  waived, not green. Restoring no-std is a separate work item.

## 0.12.1

### Fixed

- Run-queue priority lanes pop FIFO from the owner side instead of LIFO. A permanently-runnable native process (one that returns `NativeOutcome::Continue` every slice — a busy-poll connection loop, for example) was re-popped immediately after its own requeue, forever: every other pid on that scheduler thread starved indefinitely, work stealing could not rescue them (the owner's queue never exposed more than one item outside a nanoseconds-wide window), and messages delivered to a starved pid sat in its mailbox unobserved while `wake_process` correctly no-opped (the pid was runnable the whole time, not waiting). With N scheduler threads and more than N spinning natives, exactly N processes made progress. Both published crates.io releases 0.11.0 and 0.12.0 ship the LIFO lanes — consumers running busy-poll natives on a shared scheduler should upgrade. Regression-pinned under the real supervised spawn path with spawn/exit churn.

## 0.12.0

### Added

- `Scheduler::spawn_link_closure(parent_pid, closure_term)`: spawn a linked child process that runs a zero-arity closure (thunk). Unlike the `args: Vec<Term>` spawn entrypoints — whose argument terms are NOT heap-copied and require the caller to keep any backing heap alive — the closure's environment (free variables) is deep-copied into the child's own heap via the mailbox copy machinery before the child becomes runnable, so the caller's heap may be collected, mutated, or freed the moment the call returns. The child heap doubles on `HeapFull` up to a 2^26-word cap. Target resolution matches `call_fun` (generation match with unique-id validation, unique-id fallback across generations, old-generation fallback); export funs (`fun m:f/0`) resolve through the export table; native-entry funs are not spawnable. The link is established atomically at spawn (no unlinked window) and the child does not trap exits. Built for Aion's in-VM activity tier (linked activity child processes running SDK-supplied thunks).

## 0.11.0 — 2026-06-28

The cooperative wasm runtime release (WR-0..WR-10, this range landing
WR-2..WR-10): beamr's native-process model runs on the single-threaded
cooperative `WasmScheduler` — no tokio, no crossbeam channels, no OS threads
in the execution path. `beamr-wasm` 0.5.0 rides along. The public threaded
API is unchanged (additive + cfg-widening only).

### Added

- Native processes dispatch through the unified cooperative `run_until_idle`:
  native and bytecode processes share a single host pump, with native slice
  outcomes folded into the same `WasmRunSummary` and yielded-requeue buffer
  the bytecode arm uses (WR-3).
- Cooperative native timers: `WasmScheduler` carries a shared `TimerWheel`,
  so `NativeContext::send_after`/`schedule` build real `Deliver` timers
  instead of hitting an inert `None` wheel; expirations drain once per turn
  via `tick_native_timers`/`tick_native_timers_at` (WR-4).
- Cooperative supervision and restart: `spawn_native`'s `link_to` establishes
  the bidirectional link, exit propagation delivers `{'EXIT', From, Reason}`
  to trapping links and applies `should_die_from_signal` semantics to
  non-trapping ones (the predicate is now shared with the threaded path by
  construction), and restart is the trapping supervisor re-invoking the
  retained factory (WR-5). A review pass rewrote the link cascade as a
  transitive worklist mirroring the threaded path — the initial in-place kill
  let grandchildren survive and left dead processes re-enterable as zombies.
- `spawn_actor_cooperative` + `CoopActorRef`/`CoopSenderHandle`:
  fire-and-forget `cast` and non-blocking `call_async`/`call_async_timeout`
  returning a host-pumpable `CallFuture<Reply>`; ref correlation reuses the
  threaded envelope machinery, so concurrent calls never cross replies (WR-6).
- Native handlers reach the wasm async-NIF seam: `NativeContext::start_async`
  parks the handler without blocking the event loop, and `complete_async`
  delivers the completion as an `{ok, Value}`/`{error, Reason}` mailbox
  message on a later turn (WR-7).
- `DynActor`/`ReplyFn`/`WireTerm` — a term-carrying actor an untyped host
  drives over `call_async` with no new wire code — plus
  `NativeContext::alloc_owned_term`; on the beamr-wasm JS seam,
  `WasmVm::spawn_actor(handler)`, `call(pid, request)` returning a real
  Promise, and `cast` (WR-8).
- Wasm time base: the native timer wheel, the cooperative timer seam, and
  the in-memory replay driver read `web_time::Instant` (performance.now() on
  wasm; identical to `std::time::Instant` on native). beamr-wasm gains the
  requestAnimationFrame host pump: `WasmVm::pump_once`, `start_pump()`
  returning a `PumpHandle`, and idempotent `PumpHandle::stop()` (WR-10).
- Distribution reconnection hardening (HS-4/HS-5): `connect_node` treats a
  down-but-not-yet-reaped connection entry as not-connected, so re-dial after
  a dropped link is deterministic instead of being told the peer is up; plus
  a 3-node full-mesh handshake-convergence integration test (six
  simultaneous dials, per-node runtimes, hard watchdog) pinning the 0.10.0
  deadlock fix in CI.

### Fixed

- Safety-net GC collections inside `put_list`/`put_tuple2`/`update_record`
  conservatively root the full X register file. The hardcoded `live_x` of
  256 both under-rooted and NIL-cleared any live term in a register at index
  ≥ 256 — silent corruption. These opcodes carry no Live operand in this
  VM's bytecode, so conservative full-width rooting is the only sound choice
  (#106).

## 0.10.0 — 2026-06-27

### Fixed

- Distribution handshake deadlock (HS-0..HS-3,
  `docs/DISTRIBUTION-HANDSHAKE-DESIGN.md`): simultaneous cross-dials could
  hang `connect` forever and prevent a ≥3-node mesh from forming. Three
  coordinated fixes: whole-handshake deadlines (default 5 s,
  `with_handshake_timeout`, new `HandshakeError::Timeout`) so the outbound
  connect always returns and no accept-side responder parks forever (HS-1);
  race-safe connection install — `register_connection` dedups against an
  existing live link per peer name, dropping the newcomer's stream and
  replacing stale down entries, so two simultaneous handshakes cannot leave
  a clobbered, orphaned reader (HS-2); and the OTP simultaneous-connect
  tie-break (`ok`/`ok_simultaneous`/`nok` status bytes decided by node-name
  comparison) so exactly one symmetric link survives per pair — the losing
  initiator's `nok` folds into a benign non-retrying success (HS-3). The
  pre-fix silent-peer hang and simultaneous-dial mesh scenarios are pinned
  as regression oracles (HS-0).

### Added

- Wasm-runtime port groundwork (WR-0/WR-1,
  `docs/WASM-RUNTIME-PORT-DESIGN.md`): a new `cooperative` Cargo feature
  (std + crossbeam-queue only) with cooperative spawn/local-send facilities
  and a native-aware turn on `WasmScheduler` proving a native Actor runs
  cooperatively; host-only modules (io/jit/timer/replay/distribution/hook)
  are feature-gated so beamr compiles toward `wasm32-unknown-unknown` with
  no default features. The native default build is unchanged and the
  cooperative build is warning-free.

## 0.9.0 — 2026-06-24

The distribution layer's minor-release marker: promotes the cross-node work
landed in 0.8.3 (OTP handshake, async sender, cross-node pg) to a minor
version.

- Added `Scheduler::atom_table()` so distribution-facing embedders intern
  names into the SAME atom table the scheduler uses internally — pg
  group/scope atoms and the node atoms from
  `ConnectionManager::connected_nodes()` are indices into it, so a
  separately-constructed table would not match. Mirrors the accessor
  `WasmScheduler::atom_table()` already exposes.

## 0.8.3 — 2026-06-24

The distribution layer lands: cross-node process groups over authenticated
connections with non-blocking propagation.

### Added

- OTP handshake wired into `ConnectionManager` connect/accept:
  cookie/challenge/MD5-digest auth with constant-time compare and
  cryptographically random challenges; connection identity comes from the
  authenticated `HandshakeResult::remote_name` (the address→atom identity
  seam is deleted); cookie configured via `DistributionConfig`; public
  `Scheduler::start_distribution_listener`. The handshake completes before
  the data-frame read loop starts.
- Distributed process groups: local pg join/leave propagate to every
  connected node via a `PG_UPDATE` control frame (op 101, member as an
  external pid carrying the local node name); inbound frames apply on the
  peer's `PgRegistry`; a connection-down hook purges the lost node's
  members wholesale.
- Async distribution sender (`DistSender`): all outbound distribution I/O
  moves to a single owned 1-worker runtime with a bounded queue. pg
  broadcast enqueues instead of `block_on` on a scheduler worker thread
  (killing the latency cliff), process exit purges pg membership locally and
  propagates the leave async (never blocking the death path), and writes
  carry a 5 s timeout so a wedged-but-connected peer is marked down instead
  of stalling propagation cluster-wide.

### Fixed

- The distribution control-frame handler captured a strong `SharedState`
  reference, so schedulers with distribution enabled never dropped; it now
  upgrades a `Weak` per frame (regression-pinned: `strong_count == 0` on
  drop).

## 0.8.2 — 2026-06-24

- Timer messages are actually delivered: the timer wheel was
  receive-timeout-only, so `send_after`/`start_timer` scheduled messages
  that never reached any mailbox. Timers now carry a `TimerKind`
  (`ReceiveTimeout` keeps the mark-and-wake code-jump path; `Deliver` pushes
  the message into the target mailbox with Executing-slot-safe semantics and
  wakes the process).
- Native processes gain timer access: `NativeContext` carries an optional
  shared timer wheel with `schedule`/`send_after`/`cancel_timer`.
- Replay log `FORMAT_VERSION` 1 → 2: the timer-kind byte round-trips, and an
  unknown byte is `InvalidFormat` rather than a silent default.

## 0.8.1 — 2026-06-23

- Corrected the `recv_marker` opcode family to OTP numbering
  (173=bind/2, 174=clear/1, 175=reserve/1 — beamr had the three rotated with
  mismatched arities), which desynced decoding through the receive prologue
  and made the loader reject valid modules with "export label N does not
  exist"; `recv_marker_bind`'s second operand is modelled as a register
  (Ref), not a label.
- Added `Scheduler::peek_exit_reason` — a non-blocking, non-consuming read
  of a dead process's exit reason, for supervisors that must observe an
  external kill without parking on `run_until_exit`.
- Exit tombstones are bounded: the unbounded pid→ExitReason map is now an
  insertion-ordered store with FIFO eviction above 65,536 live entries,
  evicting a pid's paired exit-result satellites together with its
  tombstone — closing a slow per-connection/per-request leak with read
  semantics unchanged (eviction can never strand a blocked
  `run_until_exit`: the awaited tombstone is always the newest entry).

## 0.8.0 — 2026-06-23

The native-process release: Rust code participates in the process model as
real processes.

### Added

- Native-process core (NATIVE-001): a native process IS a `Process` carrying
  a Rust `NativeHandler` — factory-based `spawn_native`, `run_native_slice`,
  and `NativeContext::send` through the real `LocalSendFacility`. Reuses the
  park-gap protocol, exit tombstones, and pending-message merge verbatim; no
  new process-slot variants or sync primitives.
- Native-process supervision (NATIVE-002): links, monitors, exit signals,
  trap_exit, and factory-based restart reuse the pid-keyed exit-propagation
  machinery unchanged; adds `NativeContext::set_trap_exit` and generic
  `Scheduler::is_native`/`monitor`/`exit_signal`.
- Ergonomic actor API (NATIVE-003): gen_server-style `Actor` trait
  (`handle_call`/`handle_cast`), ref-correlated `call` and fire-and-forget
  `cast`, and public `spawn_actor` returning a Clone-able `SenderHandle`.
  Blocking `call` lives only on the external `SenderHandle`; handlers get a
  cast-only `ActorContext`, so the call-deadlock is unreachable by
  construction. Feature-gated; the bytecode path is untouched.

### Fixed

- A trapping process that was mid-slice (Executing) when a linked process
  exited normally never received `{'EXIT', Pid, normal}` — both the
  `process_exit_signal` and `exit_signal` (erlang:exit/2) Executing arms
  gated delivery on a non-normal reason. Both now gate on trap_exit alone,
  matching the Present arm, the remote sibling, and OTP semantics.

## 0.7.0 — 2026-06-22

### Fixed

- Cross-process local send actually delivers: `B ! Msg` between two
  processes driven through the real Send opcode silently dropped
  (`messaging::send` only delivered to an in-hand receiver, and the
  scheduler always passed none). A new `LocalSendFacility` delivers via the
  I/O-delivery template — a Present receiver gets a deep copy onto its heap
  with push-before-wake, an Executing receiver (mid-slice on another thread)
  gets the message ETF-encoded and decoded onto its heap at store-back, and
  self-sends deliver to the in-hand process. Replay clock observation
  happens under the slot lock.
- ETF decode gained reference arms (`NEWER_REFERENCE_EXT` 90,
  `REFERENCE_EXT` 114): the encoder emitted `NEWER_REFERENCE_EXT` but the
  decoder had no arm, so ref-bearing messages (gen_server call tags, monitor
  DOWNs) were silently dropped on the Executing path.

### Added

- Encode/copy failures on the send path surface via a `messages_dropped`
  telemetry counter instead of vanishing.

### Compatibility

- `NativeServices` gains a `local_send` field and is now
  `#[non_exhaustive]`; embedders constructing it as a struct literal must
  update.

## 0.6.4 — 2026-06-16

- Added `erlang:integer_to_list/2` (radix 2–36, OTP semantics).
  gleam_json's error-path hex formatter calls `integer_to_list(I, 16)`,
  which was undefined — crashing workflows that hit JSON parse errors during
  diagnostics rendering.

## 0.6.3 — 2026-06-15

- io_uring backend: added the missing `SendMsg`/`RecvMsg` match arms (the
  new `IoOp` variants made the Linux build fail on a non-exhaustive match);
  implements async sendmsg/recvmsg via io_uring opcodes with heap-stable
  storage for msghdr, iovec, and address buffers.

## 0.6.2 — 2026-06-15

- Linux build fix for io-uring 0.7.12: `Statx::new` went from 5 args to 3 —
  flags and mask are builder methods and the statxbuf pointer is an opaque
  type.

## 0.6.1 — 2026-06-13

- `put_list` and `put_tuple2` self-ensure heap space before allocating: when
  data-dependent decoding builds more cells than the preceding `test_heap`
  reservation covers, the raw bump allocator returned a fatal `HeapFull`,
  bypassing the GC-and-grow path. Both opcodes now call `ensure_space()`
  before reading operands, matching `update_record`.

## 0.6.0

### Correctness

- Off-heap (ProcBin) and sub-binary terms survive the whole BIF surface: `byte_size`/`bit_size`/`binary_part`/`is_bitstring`/`iolist_size`, `binary_to_term`, `code:load_binary` bytes, file/TCP/UDP byte and filename extraction, and the JSON bridge previously accepted only inline heap binaries (≤ 64 bytes) and raised `badarg` on anything larger — the cause of "binaries over 64 bytes kill a resumed workflow with bad argument". All now go through the representation-agnostic `BinaryRef` accessor. `byte_size`/`bit_size` additionally accept bs match contexts: OTP 26+ compilers emit the gc_bif on the reused match-context register for match tails (`<<_, Rest/binary>> = B, byte_size(Rest)`) instead of materializing the tail sub-binary.
- Message sends copy ProcBin terms by sharing their refcounted off-heap bytes and copy sub-binaries' visible ranges threshold-aware; both previously failed delivery with `InvalidBoxedTerm`.
- Published host suspension results (`Scheduler::wake_with_result`/`wake_with_result_for` and the IO-bridge completion seam) are deep-copied into owned storage at publish time and materialized on the owning process heap at slice-start apply — a boxed result term no longer points into publisher storage of foreign lifetime across the publish-to-apply window. Heap space is collected/grown before the apply copy on both the host and dirty completion paths, so arbitrarily large results cannot die on `HeapFull`.
- `call_ext_last` native tail calls are suspension-safe: the y-frame pop is deferred until a clean (non-dirty) native call completes, so a suspending native's wake re-execution no longer double-pops the stack — previously the eventual return landed at the caller's own call site with the result in x0, crashing with `bad function term {ok, ...}` whenever the suspending call's argument expression contained a cross-module call (`fn() { ffi.sleep(duration.to_milliseconds(d)) }`). Code targets and dirty natives keep the eager pop.
- Host results applied at tail-call parks (`call_ext_only`/`call_ext_last`) return to the caller — popping the deferred frame first — instead of advancing past the function's last instruction; the suspension record carries the park's resume continuation, chosen at suspend time. Scope: threaded scheduler — the WASM scheduler's completion apply still advances blindly (known follow-up, consistent with its pid-keyed completion map).

### Compatibility

- `SuspensionRecord` gained a `continuation` field and `interpreter::opcodes::trampoline::handle_suspend` takes the parked call's completion shape; embedders constructing these VM-internal types directly must update. The embedder-facing `Scheduler`/`ProcessContext` APIs are unchanged.

## 0.5.0

### Correctness

- Suspension protocol redesign (call-identity gating): every result-gated suspension — host await, dirty native call, hook suspend — now carries a per-process monotonically increasing call id recorded at suspend time. Completions are published keyed by `(pid, call id)` and applied at slice start only when the id matches the process's current suspension at its recorded park position; stale completions are dropped instead of being applied blind (the pid-keyed, position-blind application could advance the instruction pointer at the wrong park position — or twice — desyncing execution into "invalid operand for instruction pointer"). Gated host awaits (`ProcessContext::request_await_suspend`, file/UDP/TCP/inet ring operations, `submit_io_and_suspend`) have a wake guard: plain message arrivals can no longer re-execute the await native and double-submit its host work. `request_suspend` keeps its message-wakeable re-execution semantics for re-entrant natives (select, marker awaits) and now returns the suspension call id; `Scheduler::wake_with_result_for(pid, call_id, term)` is the exact completion API and `wake_with_result`/`wake_with_dirty_result` resolve the id at publish time (and return `bool`). `Scheduler::resume_process` is identity-gated (it can no longer resume an in-flight dirty call) and sticky (a resume racing the hook suspension's park gap is recorded and consumed, never lost). Completion application owns the timed-await lifecycle, so a completion-vs-timeout race can neither re-run the native nor leave stale timeout metadata that a later wait would re-arm. Process exit purges all per-pid suspension state. Resuming native continuations may legally re-suspend or trampoline (previously their requests were silently dropped), dirty natives may re-suspend as host awaits or trampoline closures (requests travel through `DirtyResult`), and pending continuations are position-gated so a re-entered await at equal stack depth cannot re-fire a continuation with garbage x0. Scope: threaded scheduler — the WASM scheduler keeps its single-threaded pid-keyed completion map (known follow-up).
- Wave 1 scheduler/VM fixes: opcode 115 (`is_function2`) decodes with its arity operand instead of crashing every literal-arity `is_function/2` guard; `try_case` consumes the current exception so a caught-and-handled exception no longer surfaces as an exit exception; the Wait arm registers in the wait set before its final mailbox recheck (lost-wakeup race against concurrent delivery); a dirty suspension whose resume raced the park is unparked by a fallback recheck.
- Registered `erlang:is_function/1` and `is_function/2` as callable BIFs — body-position calls and variable-arity guards (which compile to the guard-BIF instruction) previously crashed at call time on the unresolved erlang import.
- `receive ... after` timeouts are delivered per BEAM semantics: timer expiry falls through to the `timeout` instruction (the after-body) instead of re-scanning the receive loop and re-arming forever, and the receive timer stays armed across non-matching message wakeups instead of being cancelled with a stale ref that blocked re-arming. Timer expiry is now mark-and-wake: the owning scheduler thread applies the timeout jump at slice start, closing the expiry-vs-park race (the wait-arm recheck also notices a timer that fired inside the park gap). Scope: threaded scheduler only — the WASM scheduler (cancel-on-enqueue) and the JIT wait path (clear-ref on re-execution) still re-arm the full timeout after a non-matching wake; both are known follow-ups.

### Output

- Lists of printable latin1 character codes format as double-quoted strings (`[104,105]` prints as `"hi"`), matching `io_lib:printable_list/1` semantics and the Erlang shell.

## 0.4.9

- `bs_match` `'=:='` chunks compare as integer values, fixing literal-pattern matches against binary segments.

## 0.4.8

- Dirty-parked processes stay parked across mailbox wakes: a message arriving while a dirty native call is in flight no longer schedules a slice that re-executes the call instruction.

## 0.4.7

- Only dirty results resume dirty-call suspensions; mailbox deliveries can no longer resume a process suspended on an in-flight dirty native call.

## 0.4.6

- NIF private data — the `enif_priv_data` equivalent, carried into continuation resume contexts.
- Closed a lost-wakeup race between host delivery and NIF suspend.

## 0.4.5

- Allocation-list fun entries reserve the full closure base, fixing heap reservation for funs allocated through allocation lists.

## 0.4.4

- Release of the 0.4.3 series (no code changes beyond the version bump).

## 0.4.3

- Removed all remaining `gleam_stdlib`/`gleam@` native stub shadows; OTP-level natives made contract-exact. Fixed seven VM bugs found by extended gate stdlib coverage, plus binary-match opcodes and `string:trim` semantics.
- Deterministic replay: causal message ordering, persisted replay logs, a record/replay CLI, and hardened log validation.
- WASM scheduler: receive timers and async NIF promises bridged, direct JS term conversion, JS message send and callbacks, bundle builder with an edge-worker example.
- Workflow telemetry bridged into process tracing; Aion `with_timeout` trampoline continuation variant.

## 0.4.2

- Release bump for the correctness work documented under 0.4.1 below (core correctness, structural GC rooting, fresh Gleam gate).

## 0.4.1

### Correctness

- Fixed `STRING_EXT` literal materialisation: ETF tag 107 is a compact list of byte-sized integers and now becomes cons cells instead of a binary (root cause of `lists:reverse/1` badarg on list literals).
- Exit results and exceptions are captured as owning deep copies before process heap teardown, fixing use-after-free formatting of CLI results and error reasons.
- Native BIF allocation sequences are now structurally GC-safe: self-rooting allocators, `with_rooted`/`rooted_push` scopes, and native continuation state traced as process roots (previously x-registers above the BIF arity were not roots).
- `bs_create_bin` handles real compiler-emitted segment forms; big-integer literals load through the constant pool; unary minus/`abs` and integer-to-string conversions cover bignums.
- Capability-denied imports bind an explicit `ResolvedImportTarget::Denied` variant instead of comparing function pointers, which broke under release codegen.

### Features

- Export funs (`fun M:F/A`): EXPORT_EXT literals materialise as callable values dispatched by MFA through `call_fun`/`call_fun2` and native trampolines — passing `int.to_string` to `list.map` works.
- Native OTP 27 `json` module (`decode/1`, `encode/1`, `encode_integer/1`, `encode_float/1`, `encode_binary/1`), dependency-free and always on, with the OTP error contract `gleam_json` matches on.
- `beamr imports` also lists deferred module dependencies, so empty output now genuinely means the module runs standalone.

### Fixes

- Removed native stubs that shadowed real Gleam stdlib bytecode with wrong semantics (`gleam@list:map` argument order, `gleam@string_tree:split` returning nil).
- The CLI shares its atom table and BIF registry with the scheduler; spawn failures report resolved MFA names instead of `#<unknown atom>`.
- `io_lib_format:fwrite_g/1` keeps a decimal point in whole floats (`1.0`, not `1`).
- Fixed a whole-suite DashMap self-deadlock and a TCP fd-reuse test flake; the test suite (1,500+ tests) and strict clippy (`-D warnings`) gate the workspace.

## 0.4.0

### Headline features

- Added always-on JIT compilation via Cranelift, including runtime profiling, native-code cache support, and adaptive threshold tuning through scheduler configuration.
- Added AOT/native bundle support for exported module functions with Gleam type sidecars. AOT bundles persist a host-target-validated cache envelope and recorded function metadata; native Cranelift function pointers remain process-local and are recompiled on load.
- Added single-binary packaging support with embedded `.beam` archives and runtime loading APIs for packaged modules.
- Added a differential testing framework for comparing beamr behavior with BEAM/Gleam expectations, including JIT-threshold-forced differential runs.
- Added Criterion benchmark targets for JIT comparison and extended JIT comparison workloads.
- Added the new `gleam-types` crate for extracting, serializing, and loading Gleam type sidecars consumed by beamr's typed JIT/AOT paths.

### Breaking changes

- Runtime/API surface now carries JIT state: `SchedulerConfig` includes `jit_threshold`, `SharedState` owns JIT profiler/cache fields, and `Process` tracks JIT runtime/status fields.
- Process/runtime internals gained additional fields for Phase 4 execution state; code constructing these structures directly must use the updated constructors or provide the new fields.

### Release notes

- Publish order is `gleam-types` first, then `beamr` after the `gleam-types = 0.4.0` dependency is available.
- Actual crates.io publishing and pushing `v0.4.0` require explicit project-lead approval.
