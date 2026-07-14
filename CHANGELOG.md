# Changelog

## Unreleased

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
