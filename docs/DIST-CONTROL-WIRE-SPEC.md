# beamr distribution control wire — LINK/UNLINK/EXIT/EXIT2 (work item A-wire)

Cross-node link establishment and EXIT propagation on the real wire, replacing the never-drained
`ControlRouter` buffer. Every symbol below is verified against beamr source at HEAD (0.12.1); read
the cited file:line before editing. Companion to `DISTRIBUTION-FINISH-SPEC.md` (pg/handshake) and
scoped against work item B (multi-subscriber hook), which this spec leaves a named seam for.

The three wire breaks being closed (all confirmed): outbound controls terminate in
`ControlRouter`'s `Arc<Mutex<Vec<_>>>` with zero production drains (`remote_link.rs:54-120`, FUTURE
comment `:99-103`, pushes from `supervision_integration.rs:358-362, :2098-2132`); inbound opcodes
1/3/4/8 fall to `Err(InvalidControl)` (`control.rs:291`) swallowed by `let _ =`
(`supervision_integration.rs:97`); the correct appliers are `#[allow(dead_code)]`
(`process_remote_exit_signal` `supervision_integration.rs:277-324`, `connection_down` `:326-356`).
`bif_exit` badargs on external pids before reaching any facility (`process_bifs/mod.rs:291`,
`as_pid()` returns `None` for boxed pids).

---

## 0. Decision summary

| Q | Ruling |
|---|--------|
| **D1** | (a) Extend the live decode path; new codec bodies live in NEW `distribution/control_link.rs` (control.rs already has 672 code lines — tests start at `control.rs:673` — over the 500 cap). `ControlOp` moves there verbatim from `control_lifecycle.rs:16-68`; orphan modules deleted (§9). |
| **D2** | (a) Direct encode on the calling worker + a NEW bounded, **generation-pinned** must-deliver control lane in `DistSender` (§4). Overflow ⇒ mark the pinned connection down (`ControlOverflow`) ⇒ the noconnection backstop supplies the signal. (c) rejected: `block_on_distribution_send` blocks workers and dials inline (`supervision_integration.rs:694-697`) — forbidden from `cleanup_exited_process`. (b) rejected: an outbox needs retry/GC machinery the down-backstop makes unnecessary. |
| **D3** | (c) then (b): ship LINK/UNLINK/EXIT/EXIT2 now; reserve beamr-private `BEAMR_MONITOR = 102` (const + const-assert only, §3.4). `bif_monitor`/`bif_demonitor` stay local-only badarg (`process_bifs/mod.rs:250, :264`) — loud, correct failure until the monitor stage lands with node-down DOWN purge. |
| **D4** | (a) atoms-only, PLUS one additive variant `ExitReason::NoProc` — required so inbound LINK to a dead pid answers EXIT(noproc) instead of leaving an immortal half-link. `Atom::NOPROC` already exists (`atom/table.rs:32`, interned `:119`); `terminal_reason`'s catch-all (`supervision/link.rs:162`) needs no arm. |
| **D5** | `process::RemotePid` (`types.rs:300-307`) canonical; `control_monitor::RemotePid` + `control_lifecycle::DistributedPid` die with their modules; `supervision/monitor.rs:12` re-pointed. `from` fields for locally-minted pids encode serial 0 (precedent: `control.rs:207-211`). Restart aliasing (no creation on the wire; boxed ExternalPid is 3-component, `write_external_pid` usage `supervision/link.rs:205-211`) accepted and documented at the encoders. |
| **D6/D7** | A ships the ordering rule via a **named composed function** `remote_supervision::on_connection_down` (pg-purge THEN noconnection delivery) called from the single existing registration site (`scheduler/mod.rs:873-880`). No second registrant anywhere, so R2 cannot fire. Work item B splits the two calls into ordered subscribers mechanically. |
| **D8** | Keep the 8-byte `[control_len|payload_len]` frame via existing `encode_frame` (`control.rs:224-239`, visibility → `pub(crate)`); **payload = NIL** (PG_UPDATE precedent `control.rs:221`; a `payload_len=0` variant saves 2 bytes but adds a second framing path — rejected). Dead OTP pass-through framing in `distribution/etf.rs` untouched. Drop `ATOM_CACHE` from `offered()` (`handshake.rs:47`) — the data plane cannot decode cache refs; flag negotiation is an intersection, so removal is cross-version safe. |
| **D9** | Fixed while wiring (§6). Two confirmed defects in `process_remote_exit_signal`'s Executing arm (`supervision_integration.rs:309-321`): (1) `trap_exit` is consulted BEFORE should-die, so an inbound Kill to a trapping Executing target is wrongly trapped (C3 violation); (2) the non-trap arm inserts a bare tombstone, skipping `shared_exit_tombstone`'s ETS-transfer + link_set work (`:2284-2289`). The tombstone-deferral *mechanism* is sound: `cleanup_if_tombstoned_after_store` (`execution/core.rs:78, :87, :168, :450-460`) runs `cleanup_exited_process` → full cascade at store-back. Fix = should-die-first + kill-clears-trap + `shared_exit_tombstone`, byte-matching the in-tree local idiom (`:2255-2260` + `:459-461`). |

**Judge-dispute rulings** (each resolved by source, recorded here):

1. **Exit2 vs links.** `process_remote_exit_signal` today severs the link unconditionally (`:293,
   :310`). The applier gains `kind: RemoteExitKind::{LinkExit, Direct}`: `Direct` (op 8) never
   touches links; `LinkExit` (op 3 + noconnection backstop) removes the link and **no-ops when no
   link existed** — the D7 exactly-once gate, keyed on `Process::remove_remote_link`'s bool
   (`process/mod.rs:1098-1102`) and a new bool return on `ProcessMetadata::remove_remote_link`
   (`process_slot.rs:84-86`).
2. **Duplicate LINK ≠ noproc.** `Process::add_remote_link` returns `false` for a duplicate
   (`process/mod.rs:1090-1091`), and `establish_remote_link` forwards that bool (`supervision_integration.rs:253`)
   — so keying a noproc reply (or `link_remote`'s `BadTarget`) on `false` would misfire on
   re-links. Fix: `establish_remote_link`'s Present arm returns `false` only for
   `ProcessStatus::Exited` targets and ignores the duplicate bool (§6.3). Duplicate link = idempotent
   success (OTP parity).
3. **Present-but-Exited LINK window.** The status check happens inside `establish_remote_link`
   under the slot lock — no TOCTOU. Executing-but-tombstoned targets are deliberately linked: at
   store-back, `store_runnable_process` merges `metadata.remote_links` into the process
   (`execution/core.rs:334-336`) and `cleanup_if_tombstoned_after_store` → `propagate_exit` →
   `take_remote_links` sends the linker a wire EXIT with the true terminal reason — self-healing,
   better than a synthetic noproc.
4. **Kill-clears-trap in the local exit/2 Executing arm** (`:2255-2260` does not clear;
   `process_exit_signal` `:459-461` does): ruled **unobservable** — pending metadata messages
   cannot be read mid-slice (the Executing process's mailbox is checked out; deliveries go to
   `metadata.pending_exit_messages`), and the tombstone forces death at store-back before any
   buffered message is receivable. The new remote arm clears it for parity with `:459-461`; the
   local arm is NOT patched (out of scope, no observable divergence — recorded).
5. **Store-back drain** (`execution/core.rs:355-372`) materializes buffered entries unconditionally
   via `enqueue_exit_message_pub`/`enqueue_remote_exit_message_pub` — correct because with the D9
   fix only `!should_die && trap_exit` entries ever enter the buffer; Kill never does. Tuple shape
   `{'EXIT', ExternalPid, Reason}` verified (`supervision/link.rs:187-254`, asserted
   `supervision_tests.rs:913-920`).
6. **Cross-lane ordering** (DC-6, §5): verified `block_on_distribution_send` completes the socket
   write inside the sending BIF (`supervision_integration.rs:682-730` awaits `write_raw`), so a
   dying process's prior sends are fully written before its EXIT(3) is even created. The one honest
   non-guarantee: an EXIT2 (async lane) may be overtaken by a subsequent SEND (sync write) from the
   same caller.
7. **Lost EXIT2 ruling**: op 8 is **best-effort fire-and-forget**. It is delivered iff the pinned
   connection stays up; no connection / overflow / down ⇒ dropped. The noconnection backstop covers
   *linked* processes only and is never claimed for EXIT2. `bif_exit` returns `true` regardless
   (OTP `exit/2` always returns true; OTP's auto-connect divergence documented).
8. **Shared-lane cross-peer blast radius**: accepted for v1. A wedged peer can hold the drain ≤
   `WRITE_TIMEOUT` (5 s, `sender.rs:72`) during which a control to a healthy peer may overflow ⇒
   healthy peer marked down ⇒ noconnection + redial. Correctness preserved (DC-1/DC-3); for frame's
   supervision trees a spurious noconnection is the normal restart path, an availability blip, not
   a lost signal. Escape hatch: per-node sub-channels (`sender.rs:215-216` FUTURE note). Pinned by
   the flood test (§8).
9. **`handle_frame` compat**: signature and behavior kept byte-identical via a wrapper (§3.2 —
   strictest additive posture; its 5 in-file test callers `control.rs:733, :763, :887, :916, :944`
   compile unchanged). Opcodes 1/3/4/8 now decode and return `Ok(false)` through it (no sink)
   instead of `Err(InvalidControl)` — verified no existing test asserts the old error for those
   opcodes. The const-assert test (`control.rs:785-793`) re-points at the moved `ControlOp`.
10. **Reason-atom coercion**: unknown *atom* reasons coerce to `ExitReason::Error` — deliberately
    lethal for non-trapping targets, hostile-input-only (beamr peers emit only the six atoms);
    dropping the frame would lose a death signal. A **non-atom** reason term is shape corruption ⇒
    `Err(InvalidControl)`, frame dropped (scenario 9).
11. **Hook seam + ManualDisconnect**: the composed body is a named function (B's mechanical split
    point). `disconnect_node` runs `mark_down` → hook inline on an arbitrary caller thread
    (`connection.rs:772-778`); a unit test drives `connection_down` delivery from a plain thread
    (§8, addition T-7).

---

## 1. Wire format

Framing unchanged (C6): `[u32 control_len][u32 payload_len][control ETF][payload ETF]`
(`control.rs:224-239`, read loop `connection.rs:1043-1094`). Keepalive stays the all-zero 8-byte
header (`connection.rs:38, :1062-1064` refresh). All four new controls carry **payload = NIL**
(`encode_frame(control, Term::NIL, atom_table)`, 2-byte payload `83 6A`, `tags.rs:13` NIL_EXT=106).

Opcodes are the OTP numbers already tabled in `ControlOp` (`control_lifecycle.rs:16-68`, moving to
`control_link.rs`): inside OTP's 1..=31 with OTP-conformant tuple shapes, which `crate::etf` fully
represents (external pids `etf/encode.rs:25-30` NEW_PID_EXT; atoms). The const-assert regime
(`control.rs:785-793`) continues to constrain *beamr-private* opcodes to >31; OTP numbers for
OTP-shaped messages is the honest use of the range (and the door to stock-OTP interop for this
subset). Known deviation: OTP ≥26 uses UNLINK_ID(35)/UNLINK_ID_ACK(36); beamr peers are
beamr-only and beamr's local unlink has no ack either, so plain UNLINK(4) is internally consistent
— the unlink/exit crossing race is accepted v1 behavior (documented in module docs).

| Op | Control tuple | Emitted by | Reason rule |
|----|--------------|-----------|-------------|
| LINK = 1 | `{1, FromExtPid, ToExtPid}` | `link/1` on external pid (`process_bifs/mod.rs:134-142`), `Scheduler::link_remote` | — |
| EXIT = 3 | `{3, FromExtPid, ToExtPid, ReasonAtom}` | `propagate_exit` remote-link drain (`supervision_integration.rs:58-61`); noproc/tombstone reply to LINK-on-dead (§6.4) | always `terminal_reason` — Kill→Killed pre-applied at `:47` |
| UNLINK = 4 | `{4, FromExtPid, ToExtPid}` | `unlink/1` on external pid | — |
| EXIT2 = 8 | `{8, FromExtPid, ToExtPid, ReasonAtom}` | `exit/2` on external pid (§7), embedder `exit_remote` | raw reason; MAY carry `kill` (untrappable at receiver) |

**Pids** (C6): both fields are NEW_PID_EXT **external** pids. `From` =
`(local_node.name, caller_pid, serial 0, creation 0)` — same technique as `encode_pg_update_frame`
(`control.rs:204-211`). `To` = the target `RemotePid` verbatim `(node, pid_number, serial,
creation 0)`. Node-less pids in either position ⇒ decode `Err(InvalidControl)` (mirrors
`control.rs:309-314`). The serial-0 `from` convention is self-consistent: a LINK's
`from=(A, pid, 0)` is exactly the `RemotePid` the peer stores, and exactly what a later EXIT's
`from` must equal for the `remove_remote_link` equality gate to hit.

**ReasonAtom** ∈ {`normal`, `kill`, `killed`, `error`, `noconnection`, `noproc`}. Decode per
ruling 10.

### 1.1 Exact bytes — LINK `{1, <a@host,42,0>, <b@host,7,3>}`

Tag values from `etf/tags.rs` (VERSION=131 `:3`, SMALL_INTEGER_EXT=97 `:4`,
SMALL_ATOM_UTF8_EXT=119 `:10`, SMALL_TUPLE_EXT=104 `:11`, NEW_PID_EXT=88 `:22`):

```
control (47 bytes):
83                          -- VERSION
68 03                       -- SMALL_TUPLE_EXT, arity 3
61 01                       -- SMALL_INTEGER_EXT 1 (opcode)
58                          -- NEW_PID_EXT           (FromPid)
  77 06 61 40 68 6F 73 74   --   SMALL_ATOM_UTF8_EXT len=6 "a@host"
  00 00 00 2A               --   id = 42
  00 00 00 00               --   serial = 0
  00 00 00 00               --   creation = 0 (not represented in boxed terms; aliasing accepted, D5)
58                          -- NEW_PID_EXT           (ToPid)
  77 06 62 40 68 6F 73 74   --   "b@host"
  00 00 00 07  00 00 00 03  00 00 00 00
payload (2 bytes): 83 6A    -- NIL
frame: [00 00 00 2F][00 00 00 02][control][payload]
```

EXIT/EXIT2 append the reason, e.g. `77 06 6B 69 6C 6C 65 64` (`killed`).

### 1.2 Decode budget (C5)

`decode_control` keeps its 64-word temp `Process::new(0, 64)` (`control.rs:270-273`). Worst new arm
(EXIT/EXIT2): arity-4 tuple = 5 words + 2 boxed ExternalPids at 4 words each
(`EXTERNAL_PID_WORDS = 4`, `supervision/link.rs:199`) = **13 words ≪ 64**; control ETF ≤ ~90 bytes.
No size bump. (The staged monitor DOWN tuple is 6-ary + one big-int ≈ 16 words — still fits;
recorded so 64 is not silently outgrown.)

`DecodeOptions::safe` (`etf/decode.rs:36-37`) stays unset for control decode — **deliberately not
widened** (C5): PG_UPDATE legitimately carries first-contact scope/group atoms; the new arms add no
new unbounded interning class (from-node atoms must equal the authenticated origin, interned at
handshake; reason atoms funnel through the closed six-atom map after decode). Opcode-first peeking
to enable `safe` per-arm is recorded as a follow-up.

### 1.3 Staged monitor direction (design only — NOT built in A)

`pub const BEAMR_MONITOR: i64 = 102;` reserved in `control_link.rs` now, with the const-assert
extended: `const _: () = assert!(PG_UPDATE > SPAWN_REPLY && BEAMR_MONITOR > SPAWN_REPLY);` plus
`ControlOp::from_opcode(BEAMR_MONITOR).is_none()`. Beamr-private because OTP 19/20/21 carry real
Reference terms the term layer cannot represent (no creation; `bif_monitor` returns small-int refs,
`process_bifs/mod.rs:256`). Tuples (integer refs, watcher-node-allocated from the `1 << 56`
partition — value preserved from `control_monitor.rs:17`, kept as a doc note, not dead code):
`{102, 1, WatcherExtPid, TargetExtPid, RefInt}` monitor; `{102, 2, …}` demonitor;
`{102, 3, TargetExtPid, WatcherExtPid, RefInt, ReasonAtom}` down. Lands via one new sink field on
`ControlSinks` (zero signature churn), a `scheduler/remote_monitor.rs` state module, delivery via
the already-correct `enqueue_remote_down_message_pub` (`supervision/monitor.rs:220`), and a
noconnection-DOWN extension of `on_connection_down`. `bif_monitor`'s external-pid badarg lifts only
then.

---

## 2. Module layout

| File | Status | Contents (code lines, excl. `#[cfg(test)]`) |
|---|---|---|
| `distribution/control_link.rs` | **NEW** (~330) | `ControlOp` (moved), opcode consts, `BEAMR_MONITOR`, encoders, `decode_control_addressed`, `ControlSinks` + `dispatch_frame`, `LinkControlDelivery`, `exit_reason_from_wire` |
| `scheduler/dist_control_out.rs` | **NEW** (~140) | outbound send helpers (encode + pinned enqueue) |
| `scheduler/remote_supervision.rs` | **NEW** (~320) | moved+fixed appliers, inbound apply, backstop, `on_connection_down`, `LinkControlDelivery` impl |
| `scheduler/remote_supervision_tests.rs` | **NEW** test | in-crate wire/mailbox tests (`#[cfg(all(test, feature = "net"))]` decl in `scheduler/mod.rs`) |
| `tests/link_distribution_e2e.rs` | **NEW** test | `#![cfg(feature = "net")]`; public-API two-node e2e |
| `control.rs`, `sender.rs`, `connection.rs`, `supervision_integration.rs`, `scheduler/mod.rs`, `process_bifs/mod.rs`, `process_slot.rs`, `process/types.rs`, `execution/core.rs`, `telemetry/lifecycle.rs`, `supervision/monitor.rs`, `handshake.rs`, `remote_link.rs` | edited | thin shims / variants only; `supervision_integration.rs` net-negative (~150 lines move out) |
| `control_lifecycle.rs` + tests, `control_monitor.rs` + tests | **DELETED** (1,459 lines) | §9 |

`net` is a default feature (`Cargo.toml:64`), so both the default gate and `--all-features` run the
new tests (C14).

---

## 3. API spec

### 3.1 NEW `distribution/control_link.rs`

```rust
//! LINK/UNLINK/EXIT/EXIT2 control codec, the distribution opcode table, and the
//! sink-bundle dispatcher. Module docs carry the DC-1..DC-6 delivery contract (§5).

pub enum ControlOp { Link = 1, Send = 2, Exit = 3, Unlink = 4, RegSend = 6, Exit2 = 8,
                     MonitorP = 19, DemonitorP = 20, MonitorPExit = 21,
                     SpawnRequest = 29, SpawnReply = 31 }        // moved VERBATIM from control_lifecycle.rs:16-68
impl ControlOp { pub const fn from_opcode(i64) -> Option<Self>; pub const fn opcode(self) -> i64; }

/// Reserved beamr-private monitor opcode (§1.3). No encoder/decoder ships in A.
pub const BEAMR_MONITOR: i64 = 102;

pub fn encode_link_frame(local_node: Atom, from_pid: u64, to: RemotePid,
                         atom_table: &AtomTable) -> Result<Vec<u8>, EncodeError>;
pub fn encode_unlink_frame(local_node: Atom, from_pid: u64, to: RemotePid,
                           atom_table: &AtomTable) -> Result<Vec<u8>, EncodeError>;
/// `op` ∈ {ControlOp::Exit, ControlOp::Exit2}; any other op is an encode error.
pub fn encode_exit_frame(op: ControlOp, local_node: Atom, from_pid: u64, to: RemotePid,
                         reason: ExitReason, atom_table: &AtomTable) -> Result<Vec<u8>, EncodeError>;

/// Unknown atoms coerce to Error (deliberately lethal, hostile-input-only — ruling 10).
/// Non-atom terms are rejected by the decode arms before reaching this.
pub(crate) fn exit_reason_from_wire(atom: Atom) -> ExitReason;

/// Full addressed decode: term-decode + opcode match for ALL arms.
/// 2/6 inline; 101 delegates to control::decode_pg_update (made pub(crate));
/// 1/3/4/8 are the new arms. When `local_node` is Some:
///   SEND whose to-pid carries node Some(n) != local  -> Err(ControlError::MisAddressed)  (R6 fix)
///   LINK/UNLINK/EXIT/EXIT2 whose `to` node != local  -> Err(ControlError::MisAddressed)
/// Node-less `to` on 1/3/4/8 -> Err(InvalidControl) (strict: only new beamr emits these).
pub fn decode_control_addressed(control_etf: &[u8], atom_table: &AtomTable,
                                local_node: Option<Atom>) -> Result<ControlMessage, ControlError>;

/// Scheduler-side sink for inbound link controls (mirror of PgDelivery, control.rs:125-130).
pub trait LinkControlDelivery: Send + Sync {
    fn apply_link(&self, from: RemotePid, to_pid: u64);
    fn apply_unlink(&self, from: RemotePid, to_pid: u64);
    /// op 3: no-op unless a remote link from `from` exists on the target (DC-4).
    fn apply_link_exit(&self, from: RemotePid, to_pid: u64, reason: ExitReason);
    /// op 8: exit-signal rules regardless of links; never touches link state.
    fn apply_exit2(&self, from: RemotePid, to_pid: u64, reason: ExitReason);
}

/// Sink bundle — absorbs future sink families (monitors) without signature churn.
pub struct ControlSinks<'a> {
    pub delivery:    &'a dyn ControlDelivery,
    pub registry:    Option<&'a dyn ControlRegistry>,
    pub pg:          Option<&'a dyn PgDelivery>,
    pub links:       Option<&'a dyn LinkControlDelivery>,
    /// Authenticated peer the frame arrived from. When Some, a 1/3/4/8 frame whose
    /// `from.node` differs is dropped (Ok(false)) — from-forgery rejection.
    pub origin_node: Option<Atom>,
    /// This node's name; enables MisAddressed validation (R6).
    pub local_node:  Option<Atom>,
}

pub fn dispatch_frame(control_etf: &[u8], payload_etf: &[u8], atom_table: &AtomTable,
                      sinks: &ControlSinks<'_>) -> Result<bool, ControlError>;
// Send/RegSend/Pg* arms behave exactly as control.rs:345-382 today.
// Link/Unlink/Exit/Exit2: origin check first, then route to `links` (None => Ok(false)).
```

Encoders mirror `encode_pg_update_frame`'s body (`control.rs:195-222`): 64-word temp `Process` +
`ProcessContext`, `alloc_external_pid` for **both** pids, `control::encode_frame(control,
Term::NIL, atom_table)`.

### 3.2 CHANGED `distribution/control.rs` (net ≈ +10 lines; enum home only)

```rust
pub enum ControlMessage {
    Send    { to_pid: u64 },                                     // UNCHANGED shape (R6 enforced in decode)
    RegSend { to_name: Atom },
    PgJoin  { .. }, PgLeave { .. },                              // unchanged
    Link    { from: RemotePid, to_pid: u64 },                    // NEW — scalars only (C4)
    Unlink  { from: RemotePid, to_pid: u64 },                    // NEW
    Exit    { from: RemotePid, to_pid: u64, reason: ExitReason },// NEW (op 3, link-exit)
    Exit2   { from: RemotePid, to_pid: u64, reason: ExitReason },// NEW (op 8, exit/2)
}
pub enum ControlError { InvalidFrame, Decode(DecodeError), InvalidControl,
                        /// Frame's target pid names another node (R6).
                        MisAddressed }                           // NEW variant

pub fn decode_control(..) -> ..   // body becomes: control_link::decode_control_addressed(.., None)
pub fn handle_frame(..) -> ..     // signature UNCHANGED (control.rs:337-344); body becomes a
                                  // dispatch_frame call with links/origin/local = None.
                                  // Behavior delta: opcodes 1/3/4/8 now Ok(false), not Err.
pub(crate) fn decode_pg_update(..)   // was private (control.rs:296) — visibility only
pub(crate) fn encode_frame(..)       // was private (control.rs:224) — visibility only
```

The SEND/REG_SEND extraction bodies (`control.rs:276-289`) move into
`control_link::decode_control_addressed`; `decode_control`'s old match is deleted. `ControlMessage`
stays in control.rs (enum home; `Copy` preserved — all new fields are `Copy` scalars).

### 3.3 CHANGED `distribution/connection.rs` (+~25 lines)

```rust
pub enum ConnectionDownReason { …,                               // connection.rs:84-103
    /// The must-deliver control lane overflowed against this connection: the peer
    /// cannot absorb pending LINK/EXIT controls, so it is treated as down and the
    /// noconnection backstop (DC-3) supplies the coarsened signals.
    ControlOverflow }                                            // NEW, additive (HeartbeatTimeout precedent)

impl DistConnection {
    pub(crate) fn mark_down_control_overflow(self: &Arc<Self>);  // NEW, mirrors mark_down_write_timeout (:306-308)
}

// Internal alias (private, connection.rs:115) widens to carry the authenticated origin:
type ControlFrameHandler = dyn Fn(Atom, &[u8], &[u8]) + Send + Sync + 'static;
/// NEW, additive. The read loop (connection.rs:1085-1088) passes `connection.node` —
/// the authenticated handshake name that keys the table (connection.rs:201, :267-271).
pub fn register_control_frame_handler_with_origin<F>(&self, handler: F)
    where F: Fn(Atom, &[u8], &[u8]) + Send + Sync + 'static;
// register_control_frame_handler (connection.rs:689-699) keeps its exact public
// signature; its body wraps: move |_origin, control, payload| handler(control, payload).
```

Doc fix: `connection.rs:98-102` claims heartbeat-down fires "monitor-DOWN machinery" — false today.
Reword to "down-hook: pg purge + noconnection delivery to remote-linked processes
(remote-monitor DOWN purge lands with the monitor stage)".

### 3.4 CHANGED `distribution/sender.rs` (+~70 lines) — the generation-pinned control lane

```rust
/// Bounded depth of the must-deliver control lane. Small on purpose: a peer that
/// cannot absorb this many pending LINK/EXIT controls is effectively down (DC-1).
pub const DIST_CONTROL_QUEUE_CAP: usize = 256;

/// A control frame pinned to the connection GENERATION it was enqueued against (DC-2).
/// The drain writes only to this connection and skips it once down — a control can
/// never leak onto a post-redial socket (the drain's by-node resolve at write time,
/// sender.rs:162, is exactly the hazard this closes).
pub struct ControlOutbound {
    pub connection: Arc<DistConnection>,
    pub frame: Arc<[u8]>,
}

pub enum ControlEnqueueError {
    /// Lane full — enqueue_control has ALREADY marked the pinned connection down
    /// (ControlOverflow) before returning; the caller needs no further action.
    Overflow,
    /// Sender shut down (scheduler teardown); peers converge via EOF.
    Closed,
}

impl DistSender {
    /// NON-BLOCKING must-deliver enqueue (try_send). Full => mark the PINNED
    /// connection down, then return Overflow. Safe from scheduler workers (C7);
    /// the down-hook may run inline on the caller (supported context — the same
    /// as ManualDisconnect, connection.rs:772-778; re-entrancy audit in §10).
    pub fn enqueue_control(&self, item: ControlOutbound) -> Result<(), ControlEnqueueError>;
}
```

Internals: `DistSender` gains `control_tx: mpsc::Sender<ControlOutbound>`; the drain
(`sender.rs:158-180`) becomes one task with `tokio::select! { biased; … }` over
`(control_rx, rx)` — control lane preferred when both are ready (controls are small and
latency-sensitive; preferring them empties the lane fastest). Control write path:
`if item.connection.is_down() { continue }` then
`timeout(WRITE_TIMEOUT, item.connection.write_raw(&item.frame))`, elapse ⇒
`mark_down_write_timeout` — identical failure discipline to the data lane (`sender.rs:172-177`).
Loop exits when both channels are closed. **C9 audit**: `DistSenderInner` gains no new fields;
`ControlOutbound` holds `Arc<DistConnection>` whose `manager` is already `Weak`
(`connection.rs:205`); the bounded lane (256) bounds retained Arcs. **C10**: the overflow
`mark_down` is invoked holding no shard guard (the Arc is owned).

### 3.5 NEW `scheduler/dist_control_out.rs`

```rust
//! Outbound LINK/UNLINK/EXIT/EXIT2: encode on the calling worker, pin the current
//! connection generation, hand off to the control lane. Never blocks, never dials (C7).

pub(super) fn send_link(shared: &SharedState, caller_pid: u64, target: RemotePid)
    -> Result<(), RemoteLinkError>;                              // absent connection => Err(NoConnection)
pub(super) fn send_unlink(shared: &SharedState, caller_pid: u64, target: RemotePid);
/// op-3 link-exit. `from_pid` is the (dead or dying) local endpoint; reason must
/// already be terminal (propagate_exit converts at supervision_integration.rs:47).
pub(super) fn send_exit_linked(shared: &SharedState, from_pid: u64,
                               target: RemotePid, reason: ExitReason);
/// op-8 exit/2. Best-effort (ruling 7); MAY carry Kill.
pub(super) fn send_exit2(shared: &SharedState, caller_pid: u64,
                         target: RemotePid, reason: ExitReason);
```

Common body: `shared.dist_sender` `None` (replay mode, `scheduler/mod.rs:749-753`) ⇒ no-op /
`Err(NoConnection)` for LINK. `shared.distribution_connections.get_connection(target.node)`
(`connection.rs:721-726`) resolved **once, at enqueue** (the DC-2 pin):

- **absent, LINK** ⇒ `Err(RemoteLinkError::NoConnection)` — an unconnected LINK would create an
  immortal local half-link (no connection ⇒ no down event ⇒ no cleanup, ever). No auto-dial (C7;
  explicit `connect_node` is the liminal/frame pattern; OTP auto-connect divergence documented).
- **absent, UNLINK/EXIT/EXIT2** ⇒ drop. Not lossy: for links, the peer's own down-hook already
  delivered/will deliver noconnection for every link to us (DC-3 both-sides); EXIT2 is best-effort
  (ruling 7).
- **present** ⇒ `control_link::encode_*` ⇒
  `enqueue_control(ControlOutbound { connection, frame: Arc::from(frame.into_boxed_slice()) })`.
  `Err(Overflow)` needs no handling (down already marked; the caller's just-touched link is cleaned
  by the inline hook's noconnection). Encode `Err` (unreachable for atom/u64 inputs) ⇒
  `connection.mark_down_control_overflow()` — **DC-1 has no silent arm** (judge-flagged; the
  "nothing safe to send" silent drop is rejected).

### 3.6 NEW `scheduler/remote_supervision.rs` — inbound apply + backstop (fixed appliers)

Everything here moves from `supervision_integration.rs` (over budget, C14);
`supervision_integration.rs` keeps `pub(crate) use remote_supervision::{process_remote_exit_signal,
connection_down};` so `supervision_tests.rs:911/:953` call paths compile (they gain the new `kind`
argument only).

```rust
#[derive(Copy, Clone, Eq, PartialEq)]
pub(crate) enum RemoteExitKind {
    /// EXIT(3) + the noconnection backstop: consumes the link; absent link => no-op (DC-4).
    LinkExit,
    /// EXIT2(8): exit-signal rules; never touches links.
    Direct,
}

/// Moved from supervision_integration.rs:277-324; #[allow(dead_code)] REMOVED; fixed per D9 + ruling 1.
pub(crate) fn process_remote_exit_signal(shared: &SharedState, source_pid: RemotePid,
                                         target_pid: u64, reason: ExitReason, kind: RemoteExitKind);

/// Moved from supervision_integration.rs:326-356; #[allow(dead_code)] REMOVED. Body unchanged
/// (collect-then-apply, no lock across apply) except each application passes LinkExit.
pub(crate) fn connection_down(shared: &SharedState, node: Atom);

/// The composed down-body — work item B's mechanical split point.
/// ORDER CONTRACT (D6, scenario 5): pg purge BEFORE noconnection delivery, so a
/// trap-exit handler reacting to {'EXIT', _, noconnection} never sees the dead
/// node's pg members. Purge is synchronous here, before any wake (C12 preserved).
pub(super) fn on_connection_down(shared: &SharedState, event: ConnectionDownEvent) {
    shared.pg_registry.purge_remote_node(event.node);
    connection_down(shared, event.node);
}

/// Inbound LINK. All decisions under the target's slot lock (ruling 3).
pub(super) fn apply_inbound_link(shared: &SharedState, from: RemotePid, to_pid: u64);
```

**`process_remote_exit_signal` body (normative):**

- **Present** (`supervision_integration.rs:289-308` today): keep the Exited early-return; then
  `if kind == LinkExit && !target.remove_remote_link(source_pid) { return; }` (DC-4 gate; `Direct`
  skips the remove entirely — ruling 1); `should_die` unchanged (C3 formula, `link.rs:170-172`);
  should-die arm unchanged (`terminate` + drop-before-`cleanup_exited_process`, C1); trap arm
  unchanged (`enqueue_remote_exit_message_pub` + drop-before-wake).
- **Executing** (REBUILT — D9): same LinkExit gate on `metadata.remove_remote_link` (now `-> bool`,
  `process_slot.rs:84-86`); **compute `should_die` FIRST** (`reason == Kill || (reason != Normal &&
  !metadata.trap_exit)`); should-die arm: `if reason == Kill { metadata.trap_exit = false; }`
  (parity `:459-461`; local-arm divergence ruled unobservable, ruling 4) then
  `shared_exit_tombstone(shared, target_pid, link::terminal_reason(reason))`
  (`supervision_integration.rs:2284-2289`, visibility → `pub(super)`) — death completes at
  store-back via `cleanup_if_tombstoned_after_store` → `cleanup_exited_process` → `propagate_exit`
  (full cascade/DOWN/resource path, `execution/core.rs:78, :450-460`); trap arm unchanged
  (`pending_exit_messages` push with `PendingExitSource::Remote`, drop-before-wake, C2).
- **Absent**: no-op.

**`apply_inbound_link`:**

```rust
if !establish_remote_link(shared, to_pid, from) {
    // Dead or absent target (never a duplicate — ruling 2): answer EXIT(noproc) or the
    // real terminal reason, so the linker does not hold a dangling half-link forever.
    let reason = shared.exit_tombstones.get(&to_pid)          // exit_tombstones.rs:91
        .map(link::terminal_reason)
        .unwrap_or(ExitReason::NoProc);
    dist_control_out::send_exit_linked(shared, to_pid, from, reason);
}
```

An Executing-but-tombstoned target links successfully and self-heals at store-back (ruling 3).

**`LinkControlDelivery` impl** on `SchedulerDistributionSendFacility` (struct visibility
`supervision_integration.rs:652` → `pub(super)`; impl lives here — same-crate cross-module impl):
`apply_link` → `apply_inbound_link`; `apply_unlink` → `remove_remote_link(shared, to_pid, from)`
(`supervision_integration.rs:262-275`, no reply); `apply_link_exit` →
`process_remote_exit_signal(.., LinkExit)`; `apply_exit2` → `process_remote_exit_signal(.., Direct)`.
All arms are thin shims over the slot-protocol-correct appliers (R4, C1, C2).

### 3.7 CHANGED `scheduler/supervision_integration.rs` (net ≈ −150 lines)

- `establish_remote_link` (`:243-260`) Present arm becomes (ruling 2/3):

  ```rust
  ProcessSlot::Present(ScheduledProcess(process)) => {
      if matches!(process.status(), ProcessStatus::Exited(_)) { return false; }
      let _ = process.add_remote_link(remote);   // duplicate = idempotent success
      true
  }
  ```

  Contract change documented: `false` now means dead/absent only. Callers: `link_remote` (which
  today wrongly returns `BadTarget` → noproc on a duplicate re-link) and `apply_inbound_link`.
- `register_distribution_control_handler` (`:77-106`): registers via
  `register_control_frame_handler_with_origin`; body calls `control_link::dispatch_frame` with
  `ControlSinks { delivery: &facility, registry: Some(&facility), pg: Some(&facility),
  links: Some(&facility), origin_node: Some(origin), local_node: Some(shared.local_node.name) }`.
  The `let _ =` at `:97` becomes `if let Err(_error) = … { #[cfg(feature = "telemetry")]
  crate::telemetry::…dropped-control-frame counter (additive) }` — never panics, never kills the
  read loop (scenario 9). Weak-capture pattern unchanged (`:86`, C9).
- `send_remote_exit` (`:358-362`) body → one line:
  `dist_control_out::send_exit_linked(shared, caller_pid, target, reason)` (reason already terminal
  at the call sites `:47, :60`).
- `SchedulerDistributionControlFacility` (`:2092-2133`): `link_remote` keeps its
  process_table check, adds the connection precondition (maps `send_link`'s `NoConnection`),
  and keeps **establish-then-send order** — LOAD-BEARING: if the send overflows and the inline hook
  fires, `connection_down` must observe the just-established link to convert it to noconnection.
  `unlink_remote` → `remove_remote_link` + `send_unlink`. `exit_remote` → `send_exit2` (this
  facility method IS the EXIT2/`exit/2` path; doc sharpened), always `Ok(())` (ruling 7).
- `process_remote_exit_signal`/`connection_down` move out (§3.6); `#[allow(dead_code)]` at `:277`
  and `:326` deleted with the false "Called by distribution connection layer" comments.

### 3.8 CHANGED `scheduler/mod.rs`

- Delete `use crate::distribution::remote_link::ControlRouter` (`:94`), the
  `control_router` field (`:282`) and init (`:827`); fixtures `supervision_tests.rs:299`,
  `tests.rs:1191, :1550` updated.
- Replace the pg-only hook closure (`:873-880`) with:

  ```rust
  let down_weak = Arc::downgrade(&shared);
  shared.distribution_connections.register_connection_down(move |event| {
      if let Some(shared) = down_weak.upgrade() {
          remote_supervision::on_connection_down(&shared, event);
      }
  });
  ```

- NEW additive embedder API (serves frame — the driving consumer — and makes the e2e drivable
  without BIF plumbing), next to `monitor()`/`exit_signal()` (`:994, :1013`):

  ```rust
  impl Scheduler {
      pub fn link_remote(&self, local_pid: u64, remote: RemotePid) -> Result<(), RemoteLinkError>;
      pub fn unlink_remote(&self, local_pid: u64, remote: RemotePid) -> Result<(), RemoteLinkError>;
  }   // delegate to SchedulerDistributionControlFacility
  ```

### 3.9 CHANGED `native/process_bifs/mod.rs` — `bif_exit` remote arm

Replace the `as_pid()` gate (`:291`) with `PidRef` routing, replicating `bif_link`'s **full**
cfg-arm structure (`:116-148`) verbatim — `distribution_control_facility()` is net-gated and the
missing `#[cfg(not(feature = "net"))]` arm is exactly the class of break commit 31bc4a8 repaired:

```rust
pub fn bif_exit(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [pid_term, reason_term] = args else { return Err(badarg()); };
    let target = PidRef::new(*pid_term).ok_or_else(badarg)?;      // pid_ref.rs:11-23
    let caller_pid = context.pid().ok_or_else(badarg)?;
    let reason = exit_reason_from_term(*reason_term)?;            // gains NoProc arm (:301-311)
    match target {
        PidRef::Local(target_pid) => { /* existing SupervisionFacility path, :294-298 unchanged */ }
        #[cfg(feature = "net")]
        PidRef::Remote(_) => {
            let remote = target.remote_pid().ok_or_else(badarg)?; // pid_ref.rs:54-63
            let facility = context.distribution_control_facility().ok_or_else(badarg)?;
            let _ = facility.exit_remote(caller_pid, remote, reason);
            Ok(Term::atom(Atom::TRUE))    // exit/2 returns true even when undeliverable (ruling 7)
        }
        #[cfg(not(feature = "net"))]
        PidRef::Remote(_) => Ok(Term::atom(Atom::TRUE)),
    }
}
```

`bif_monitor`/`bif_demonitor` (`:242-274`) unchanged; doc-comments gain an explicit
"external pids badarg until the BEAMR_MONITOR stage" note so the gate reads as intentional.

### 3.10 `ExitReason::NoProc` (D4) — exhaustive-match arms

`types.rs:229-240` gains the variant; arms added at: `as_atom` (`types.rs:245-253` →
`Atom::NOPROC`), `exit_reason_from_term` (`process_bifs/mod.rs:301-311`), `exit_reason_label`
(`execution/core.rs:1186-1195` → `"noproc"`), `exit_class` (`telemetry/lifecycle.rs:251-258` →
`"error"` group). `terminal_reason` (`supervision/link.rs:159-164`) needs no arm (catch-all).

---

## 4. Call-graph deltas

```
OUTBOUND (before -> after)
  propagate_exit (supervision_integration.rs:58-61)
    └─ send_remote_exit ─ ControlRouter Vec (never drained) ✗
    └─ send_remote_exit ─ dist_control_out::send_exit_linked ──┐
  bif_link ─ link_remote ─ ControlRouter ✗                      │
           ─ link_remote ─ [conn pin] ─ establish ─ send_link ──┤
  bif_unlink ─ unlink_remote ─ remove ─ send_unlink ────────────┼─ control_link::encode_*
  bif_exit(ext) ─ badarg ✗ ─→ exit_remote ─ send_exit2 ─────────┤
  apply_inbound_link(dead) ─ send_exit_linked(noproc|tombstone)─┘
                                       │
        DistSender::enqueue_control(ControlOutbound{ pinned Arc<DistConnection>, frame })
           │ Full => mark_down(ControlOverflow) on the PINNED generation => hook
           ▼
        drain (biased select, control lane first) ─ is_down? skip ─ write_raw
           │ timeout => mark_down_write_timeout      │ err => mark_down(WriteError)

INBOUND (before -> after)
  read loop (connection.rs:1043-1094) ─ handler(origin, control, payload)
    └─ dispatch_frame(.., ControlSinks{ links: Some, origin_node, local_node })
        ├─ Link   ─ apply_inbound_link ─ establish_remote_link | EXIT(noproc) reply
        ├─ Unlink ─ remove_remote_link
        ├─ Exit   ─ process_remote_exit_signal(.., LinkExit)   [DC-4 gate]
        ├─ Exit2  ─ process_remote_exit_signal(.., Direct)
        └─ Send/RegSend/Pg* ─ unchanged (+ MisAddressed rejection at decode)

NODE LOSS
  mark_down (any reason) ─ hook ─ on_connection_down:
    pg_registry.purge_remote_node  THEN  connection_down ─ per-link
    process_remote_exit_signal(remote, local, NoConnection, LinkExit)
```

---

## 5. Delivery & ordering contract (normative; module docs on `control_link.rs` + `sender.rs`)

- **DC-1 (send-or-down; no silent arm).** For every enqueued control C against pinned connection G,
  exactly one of: (a) C is written to G's socket in per-node FIFO order; (b) G is marked down.
  Loss-path table: lane full ⇒ `mark_down(ControlOverflow)` at enqueue; write error ⇒ `write_raw`
  marks down (`connection.rs:293-295`); write timeout ⇒ `mark_down_write_timeout`
  (`sender.rs:172-177`); encode failure ⇒ `mark_down_control_overflow`. Down ⇒ hook ⇒ DC-3.
  Both-sides convergence: our `mark_down` fires `shutdown.notify_waiters()` and the read loop drops
  its read half closing the socket (`connection.rs:314-316, :1051-1053`), so the peer sees EOF
  (`PeerClosed`, `:1057-1059`), a write error, or its 45 s heartbeat deadline
  (`connection.rs:48, :1122-1124`) — its own hook fires within a bounded window.
- **DC-2 (generation pinning).** A control enqueued against generation G is written to G's socket
  or not at all (`is_down` skip). Controls never leak onto a post-redial connection. Corollary:
  after any down+redial, cross-node link state between the pair starts empty on both sides (all
  links noconnection'd at down) and must be re-established by fresh LINKs.
- **DC-3 (noconnection backstop — scope-honest).** On connection-down, `on_connection_down` runs
  pg-purge then noconnection delivery to **every local process holding a remote link over that
  node** — linked processes ONLY. A lost link-EXIT is therefore never a lost signal (coarsened to
  `noconnection`). EXIT2 to an unlinked target has NO backstop and is best-effort (ruling 7).
- **DC-4 (exactly-once exit per link).** For each remote link, exactly one of {wire EXIT(3),
  noconnection} is applied: both paths consume the link entry under the target's slot lock;
  `LinkExit` with no entry is a no-op. Within one socket the race cannot occur (frames precede EOF
  serially; the hook fires after the read loop exits); across generations the gate closes it.
- **DC-5 (per-node control FIFO, C11).** All link controls for a node traverse one bounded channel,
  one drain task, one per-connection writer mutex (`connection.rs:203, :288-292`) — a LINK enqueued
  before an EXIT is written before it. (Supersedes the retired `ControlRouter` order tests,
  `remote_link.rs:204-230`.)
- **DC-6 (cross-lane ordering, honest statement).** A wire EXIT(3) produced by a process's death
  never overtakes messages that process sent before dying: SENDs complete their socket write inside
  the sending BIF (`block_on_distribution_send` awaits `write_raw`,
  `supervision_integration.rs:682-730`), the EXIT is enqueued only after death, and the writer
  mutex serializes bytes. NOT guaranteed: an EXIT2 (async lane) may be overtaken by a subsequent
  SEND from the same caller; no ordering exists between link controls and pg frames.

Replay mode: `dist_sender` is `None` (`scheduler/mod.rs:749-753`) ⇒ LINK fails `NoConnection`,
UNLINK/EXIT/EXIT2 no-op — distribution is globally off in replay.

---

## 6. (folded into §3.6–3.7 — applier semantics are normative there)

## 7. (folded into §3.9 — bif_exit)

## 8. Test plan

Harnesses: `tests/link_distribution_e2e.rs` (public API only: two Schedulers per the
`pg_distribution_e2e.rs:9-123` shape — `DynamicResolver` `:25-50`, ctor `:52-75`, `Idle` actor
`:86-110`, `eventually` `:113-119`; aliveness via `Scheduler::process_namespace`), and in-crate
`scheduler/remote_supervision_tests.rs` (same two-real-Scheduler harness but with `shared`-internal
access for mailbox/trap assertions — the `supervision_tests.rs` fixture + `read_mailbox_tuple`
pattern `:913-920`). Every multi-node test wraps in the HS-5 60 s watchdog
(`distribution_mesh_handshake.rs:93-108`) — **scenario 10** is a harness property.

| # | Package scenario | Coverage |
|---|---|---|
| 1 | Cross-node link, clean exit, both directions | e2e: `Scheduler::link_remote(pid_a, RemotePid{b,pid_b,0})`, `exit_signal(pid_b, Error)` on B ⇒ non-trapping A dies (`process_namespace(pid_a)` → None), mirrored. In-crate: trapping A's mailbox gets exactly `{'EXIT', <ext-pid>, error}` 3-tuple over a real loopback frame. Also asserts DC-6: N messages sent by B pre-death all precede the EXIT. |
| 2 | Normal exit does not kill | B exits Normal ⇒ A survives (positive aliveness recheck); in-crate trapping A gets `{'EXIT', _, normal}` (C3 includes Normal for trappers, `:302-307` arm). |
| 3 | Kill→Killed across the wire | `exit_signal(pid_b, Kill)` ⇒ `propagate_exit` pre-terminalizes (`:47`) ⇒ wire carries `killed` ⇒ trapping A survives with the tuple (parity `killed_signal_is_trappable_by_linked_process`). Variant: `exit_remote(.., Kill)` ⇒ EXIT2 carries raw `kill` ⇒ B target dies untrappably even when trapping AND its link to a third process survives if kind=Direct left links alone. |
| 4 | Node death → noconnection | e2e: drop B's `AcceptHandle` + `Scheduler` ⇒ read-loop EOF ⇒ `mark_down(PeerClosed)` (`connection.rs:1057-1059`) ⇒ real hook ⇒ every remote-linked A-proc dies/traps `noconnection`. The direct-call unit (`supervision_tests.rs:924-964`) is kept. |
| 5 | pg purged before noconnection observed | e2e: trapping+linked A-proc with B pg members; after A's noconnection EXIT is observable, `remote_members` is empty — structural (purge precedes delivery sequentially in `on_connection_down`). |
| 6 | No double-fire | e2e: graceful wire EXIT then node drop ⇒ exactly one `{'EXIT',…}` tuple. In-crate reverse order: `connection_down` first, then a hand-encoded EXIT(3) through `dispatch_frame` ⇒ LinkExit gate no-ops. Both race orders pinned. |
| 7 | Hook multi-subscriber | Deferred to work item B (hook API untouched). A's obligation: one test asserts BOTH composed effects fire in order on one event, and `pg_join_visible_on_peer_and_purged_on_node_down` + `reconnection_…without_stale_resurrection` (`pg_distribution_e2e.rs:128…`) pass **unmodified** (C12). |
| 8 | Inbound EXIT to Executing target | In-crate: make-Executing fixture, EXIT(3) frame through `dispatch_frame`; trapping ⇒ `pending_exit_messages` `(Remote, reason)` materialized at `store_runnable_process` (`execution/core.rs:355-372`); non-trapping+error ⇒ `shared_exit_tombstone` ⇒ dead at store-back with ETS heired + local links cascaded (pins the D9 alignment). **D9 regression:** Executing+trapping+Kill (EXIT2) dies `killed`, untrapped. |
| 9 | Hostile/unknown frames | Unit: opcode 77, wrong arities, node-less pids on 1/3/4/8, tuple reason, `to` naming a third node (MisAddressed), `from.node != origin` (forgery) ⇒ `Err`/`Ok(false)`, no panic. e2e: garbage frame via `write_raw` from a raw peer ⇒ read loop alive (subsequent SEND delivers). |
| 10 | Watchdog | 60 s watchdog on every multi-node test (harness property, above). |

**Judge-mandated additions:**

- **T-1 wedged-peer overflow flood (unit, sender.rs):** never-reading peer fixture
  (`sender.rs:459-574` pattern); `enqueue_control` 4× `DIST_CONTROL_QUEUE_CAP`; assert every call
  non-blocking, down-hook fires exactly once (`ControlOverflow` or `WriteTimeout` — either is
  DC-1(b)), connection leaves the table.
- **T-2 exit-storm exactly-once convergence (in-crate, the R1 kill-shot):** N=2000 A-procs each
  remote-linked to a distinct trapping B-proc; kill all in one burst (exceeds both queue caps by
  construction). Assert total `{'EXIT',_,R}` across B == 2000 exactly (DC-4), each
  R ∈ {`error`, `noconnection`}; connection survived ⇒ all `error`; overflowed ⇒ remainder
  `noconnection` and the pair is down on both sides.
- **T-3 generation pinning:** enqueue a control against connection G; mark G down; redial (HS-4
  path, `connection.rs:755-769`); assert the frame never reaches the new socket.
- **T-4 duplicate-LINK idempotence:** inbound LINK twice ⇒ one link entry, NO noproc reply
  (ruling 2); `link_remote` twice ⇒ `Ok(())` both times.
- **T-5 LINK-to-dead reply:** inbound LINK to a dead pid ⇒ peer's linker receives
  `{'EXIT', _, noproc}` (or tombstone terminal reason); non-trapping linker dies.
- **T-6 misaddressed SEND (R6 fix):** SEND frame whose to-pid is external with a third node's name
  ⇒ dropped, local pid N untouched; node-less to-pid still delivers (legacy tolerance).
- **T-7 ManualDisconnect inline hook:** `disconnect_node` from a plain `std::thread` ⇒
  noconnection delivered to linked procs, no deadlock (ruling 11).
- **T-8 cross-peer lane contamination (documents accepted risk):** wedged peer X + burst; if a
  control to healthy peer Y overflows, Y's down converges to noconnection + Y redials — no lost
  signals, blip only.
- **Test conversions:** `remote_link_exit_sends_exit_control` (`supervision_tests.rs:876-897`,
  asserts `control_router.messages()`) rewritten as a wire assertion — real `DistSender` +
  `register_test_connection` (`connection.rs:1012-1024`) to a loopback reader,
  `cleanup_exited_process` on a remote-linked pid, reader `split_frame` + `decode_control` ⇒
  `ControlMessage::Exit { from, to_pid, reason: Error }`. `:911/:953` gain the `kind` arg.
  `control_lifecycle_tests.rs`/`control_monitor_tests.rs` deleted; `ControlOp` round-trip tests
  move to `control_link.rs`.

---

## 9. Retirements (R5, D5)

Deleted: `control_lifecycle.rs` (563), `control_lifecycle_tests.rs` (324), `control_monitor.rs`
(403), `control_monitor_tests.rs` (169); decls `distribution/mod.rs:6-7`. Salvaged: `ControlOp`
(moved, §3.1), the `1 << 56` ref-partition value (doc note, §1.3). Gone: `dispatch_control_message`,
`ControlLifecycleState` (stores `Term::NIL` for remote pids — the latent bug), `ControlPlane`,
`RecordingMonitorSender`, `DistributedPid`, `control_monitor::RemotePid`.
`supervision/monitor.rs:12` → `use crate::process::RemotePid` (identical shape;
`enqueue_remote_down_message_pub` `:220` compiles unchanged — the monitor stage's landing pad).
`remote_link.rs` shrinks to `DistributionControlFacility` + `RemoteLinkError` (~45 lines; module
path kept — `native` imports unchanged): `ControlRouter`, its `ControlMessage`, `local_remote_pid`,
the FUTURE comment `:99-103`, and tests `:130-250` deleted. `SharedState.control_router` removed
(`scheduler/mod.rs:94, :282, :827` + 3 fixture sites). Public-module removals are sanctioned
0.x-minor breaks at **0.13.0**; the C13 liminal-0.11.0 surface is untouched (§11).

---

## 10. Hazards

1. **Stale frame mid-drain race:** `is_down` passes, the peer applies a frame, then its own down
   fires — converges: the peer's purge/backstop runs after its read loop delivered the frame,
   removing whatever the frame established.
2. **Inline-hook re-entrancy (R3):** the hook runs inline on the dist runtime, a scheduler worker
   (overflow path), or an arbitrary thread (ManualDisconnect). `on_connection_down` takes only
   short-hold slot/registry locks and does no I/O; workers do NOT hold slot locks while executing
   BIFs (the slot is `Executing`, lock released), so no inversion with
   `block_on_distribution_send`. Its cascade may call `send_exit_linked` → `enqueue_control`
   (non-blocking) → at worst `mark_down` of a DIFFERENT node → recursive hook — depth bounded by
   node count, each node exactly once (`down.swap` AcqRel `connection.rs:311` + ptr-eq `remove_if`
   `:502-505`); same-node recursion impossible. No locks held at `invoke` (`:160-169, :495-510`).
   Documented on `on_connection_down`.
3. **Cross-peer lane contamination:** accepted v1 (ruling 8), pinned by T-8; escape hatch =
   per-node sub-channels.
4. **`take_remote_links_from` consume-then-fail (`supervision_integration.rs:227-241`):** every
   post-take failure converges — encode failure ⇒ pinned connection marked down; no connection ⇒
   peer's own hook covers its side; overflow ⇒ our down + backstop. No retry, no unbounded Vec.
5. **Restart aliasing (D5):** no creation on the wire; a peer restart during a partition can alias
   pre-restart pids until the reconnect handshake replaces the connection. Recorded at encoders.
6. **UNLINK/EXIT crossing (no UNLINK_ID):** an unlink racing an inbound EXIT can still deliver the
   EXIT (link present on the receiver at apply time). OTP-26 ack protocol deliberately not
   implemented (beamr-only peers, local semantics have no ack). Accepted v1, documented.
7. **Pre-existing (out of scope, flagged for a separate ruling):** the LOCAL
   `process_exit_signal` Executing should-die arm cascades inline AND leaves `metadata.links`
   intact (`supervision_integration.rs:452-481` + store-back `execution/core.rs:78`), risking
   re-signaling trapping linked processes. The new remote arm deliberately avoids the pattern
   (tombstone-only); no wire code path is entangled with it.
8. **`handle_frame` behavior delta:** opcodes 1/3/4/8 return `Ok(false)` instead of
   `Err(InvalidControl)` through the compat wrapper — verified no test depends on the old error.

---

## 11. Compatibility (C13) & versioning

Byte-stable: `ConnectionManager::{connected_nodes, get_connection, set_runtime_handle,
register_connection_down(single closure), register_control_frame_handler}`,
`DistConnection::write_raw`, `PgRegistry` methods, `control::{encode_pg_update_frame,
encode_send_frame}`, `handle_frame`, `Scheduler::{distribution_connections, atom_table,
pg_registry, start_distribution_listener}`, `RemoteMember`, `ConnectionDownEvent{node, reason}`.
Additive: `ConnectionDownReason::ControlOverflow`, `ExitReason::NoProc`, `ControlMessage::{Link,
Unlink, Exit, Exit2}`, `ControlError::MisAddressed` (all non-exhaustive-free enums liminal does not
match on — C13 list), `register_control_frame_handler_with_origin`, `DistSender::enqueue_control`,
`Scheduler::{link_remote, unlink_remote}`, `control_link` module. Removals (`ControlRouter`, orphan
modules) are off the C13 surface. Version: **0.12.1 → 0.13.0** (`Cargo.toml:3`).

---

## 12. Migration — commit sequence

Every commit passes: `cargo fmt --check`, `cargo check`, `cargo test -p beamr` (lib + integration)
default features AND `--all-features`, `cargo clippy --all-targets -D warnings`, `--no-default-features`
check for the bif cfg arms. Zero new `#[allow]`; three existing `#[allow(dead_code)]` removed
(appliers + `PendingExitSource`, `process_slot.rs:15`).

1. **`feat(distribution): control_link codec + addressed decode (wire-inert)`** —
   `control_link.rs` (ControlOp moved; `control_lifecycle.rs` gains a `pub use` shim until commit 5;
   `remote_link.rs:8` + const-assert test + `supervision_tests.rs:892` re-pointed); encoders;
   `decode_control_addressed`/`ControlSinks`/`dispatch_frame`/`LinkControlDelivery`;
   control.rs variant additions + wrapper bodies + `pub(crate)` visibilities; `ExitReason::NoProc`
   + 4 match arms. Round-trip/hostile/misaddressed unit tests. Inert: nothing encodes on any prod
   path; inbound 1/3/4/8 decode but hit `links: None` ⇒ dropped, same net behavior as today.
2. **`feat(distribution): generation-pinned must-deliver control lane`** — `ControlOutbound`,
   `enqueue_control`, `DIST_CONTROL_QUEUE_CAP`, biased two-lane drain, `ControlOverflow` +
   `mark_down_control_overflow`; sender module docs gain the DC-1/DC-2 sections. Tests: control
   FIFO (mirror `per_node_fifo_ordering` `sender.rs:289-348`), T-1 flood, T-3 pinning.
3. **`feat(scheduler): inbound link-control apply + noconnection backstop (D9, DC-4, R4, R6)`** —
   `remote_supervision.rs` (moved+fixed appliers, `RemoteExitKind`, `apply_inbound_link`,
   `on_connection_down`, `connection_down`); `dist_control_out.rs` (needed by the noproc reply);
   `establish_remote_link` Exited fix; `process_slot` bool + allow removal;
   `register_control_frame_handler_with_origin` + read-loop origin + handler switch to
   `dispatch_frame` with all sinks; composed hook closure replaces `scheduler/mod.rs:873-880`;
   `shared_exit_tombstone` → `pub(super)`; connection.rs `:98-102` doc fix; dead_code removals.
   Tests: scenarios 6-reverse/8/9, D9 regression, T-4, T-5, T-6, T-7. **Backstop is live before any
   EXIT rides the wire** (judge-mandated order); inbound apply is exercised by in-crate frames only
   — no peer emits yet.
4. **`feat(scheduler): outbound controls on the wire; retire ControlRouter`** — `send_remote_exit`
   + facility rewired onto `dist_control_out`; `link_remote` connection precondition; `bif_exit`
   PidRef + full cfg arms; `Scheduler::{link_remote, unlink_remote}`; `control_router` field +
   `ControlRouter` + its tests deleted; fixtures updated; `supervision_tests.rs:876-897` → wire
   assertion. Tests: in-crate scenarios 1/2/3/6-forward, T-2 storm, T-8. Cross-version note: a
   commit-3 peer decodes-and-applies these frames; a pre-commit-1 peer drops them (`Err` swallowed
   at its `:97`) and converges via node-down — the pg rollout parity.
5. **`refactor(distribution): delete orphaned control planes; canonical RemotePid`** — delete 4
   files + `distribution/mod.rs:6-7` decls + the ControlOp `pub use` shim;
   `supervision/monitor.rs:12` import swap.
6. **`test(distribution): cross-node supervision e2e; drop ATOM_CACHE (D8)`** —
   `tests/link_distribution_e2e.rs` (scenarios 1-6, 9, 10 at the public-API level); delete
   `| Self::ATOM_CACHE.0` from `offered()` (`handshake.rs:47`; const at `:27` kept); update
   handshake tests asserting offered bits (`:874, :878, :918, :922`); verify pg e2e suite passes
   unmodified (C12/C13 regression gate).
7. **`release: beamr 0.13.0`** — version bump + changelog per house release process.

---

## 13. Constraints: discharged vs deferred

**Discharged:** C1 (all inbound arms are shims over the slot-protocol appliers; drop-before-wake
preserved verbatim), C2 (Executing routes through `pending_exit_messages`/`PendingExitSource::Remote`),
C3 (should-die formula untouched; D9 fixes the one violation; EXIT pre-terminalized, EXIT2 carries
Kill), C4 (scalar-only `ControlMessage` variants; mailbox tuples via
`enqueue_remote_exit_message_pub`), C5 (13 ≪ 64 words; safe-atom posture unchanged, not widened),
C6 (external pids both fields; keepalive untouched; private opcodes >31 with extended
const-assert), C7 (encode on caller; `try_send`; drain never dials; no auto-connect), C8/R1
(must-deliver lane + DC-1 + T-1/T-2), C9 (pinned Arcs bounded; `manager` already Weak; hook closure
Weak), C10 (all new down paths funnel through `mark_down`; overflow holds no guards), C11 (DC-5),
C12 (purge body untouched, synchronous, explicitly ordered first), C13 (§11), C14 (new modules;
over-budget files net-negative or thin-shim-only; control.rs net ≈ +10 flagged as pre-existing
debt; no unwrap/expect/panic outside tests — poisoned locks via `unwrap_or_else(|e| e.into_inner())`;
both feature gates every commit). R2 (no second registrant; named composed seam), R3 (hazard 2),
R4 (thin shims), R5 (§9), R6 (MisAddressed for new ops AND the SEND fix, plus origin-forgery
rejection).

## 14. Landing-order reconciliation with work item B (normative; supersedes conflicting lines above)

`CONN-EVENTS-HOOK-SPEC.md` lands FIRST (its §9; R2 mitigation), so by the time this spec's commits
run, the single-slot registration at `scheduler/mod.rs:873-880` no longer exists — it has become the
hub + `scheduler/connection_lifecycle.rs::handle_connection_event` (pg-purge then
`supervision_integration::connection_down`), and the noconnection backstop is ALREADY LIVE (B
commit 3), which discharges this spec's judge-mandated "backstop before outbound EXITs" ordering by
construction. Adaptations to this spec under B-first:

- §0 D6/D7 and §3.6's `on_connection_down` are NOT built as a new registration: the composed body
  already exists as `handle_connection_event`. This spec's commit 3 instead (a) moves the appliers
  into `remote_supervision.rs` as planned, keeping the
  `pub(crate) use remote_supervision::{process_remote_exit_signal, connection_down};` shim in
  `supervision_integration.rs` so B's `handle_connection_event` call sites compile unchanged, and
  (b) extends nothing in the hook path — scenario-5 ordering lives in B's composed subscriber.
- §3.8's "replace the pg-only hook closure" step is void (B already did it). The `ControlRouter`
  deletions stand.
- B's `handle_connection_event` Up arm seam ("reserved for work item A control-lane init") stays
  empty in this spec too: the generation-pinned lane needs no per-Up initialization (`ControlOutbound`
  pins `Arc<DistConnection>` directly; DC-2 needs no per-session state).
- Versioning: B's commit 5 release step and this spec's commit 7 merge into ONE `release: beamr
  0.13.0` commit after both items land, with a combined changelog section and ADR-012.

**Deferred (recorded):** remote monitors (§1.3; `bif_monitor` badarg until then); Reference-term /
creation plumbing; arbitrary exit-reason terms (D4); OTP auto-connect on link/exit; per-node
control sub-channels (ruling 8 escape hatch); C5 opcode-first `safe` decode; `global::remove_node`
hook migration (`global.rs`, currently never called); multi-subscriber hook + connection-UP events
(work item B — the `on_connection_down` seam is its split point); UNLINK_ID ack protocol
(hazard 6); local `process_exit_signal` Executing double-cascade hazard (hazard 7 — separate
ruling); local exit/2 kill-clears-trap cosmetic divergence (ruling 4 — no observable effect);
telemetry counter naming for dropped control frames (additive, non-blocking).

## 15. As-built addendum (2026-07-07, release 0.13.0 — records where the landed code deviates; the deviations are normative)

The wire series landed as commits d528abd (A0 hub follow-ups), 92db922 (§12.1
codec), 2143c01 (§12.2 control lane), cc15d72 (§12.3 inbound apply), 6bd97c8
(§12.4 outbound wire), 7604683 (§12.5 orphan deletion), 02500c1 (§12.6 e2e +
ATOM_CACHE), and f6421fc (adversarial-review fixes). Deviations and rulings:

- **Node-less-pid semantics (amends §3.1):** the as-built ETF codec has no
  node-less pid representation — local pids encode as NEW_PID_EXT carrying
  `nonode@nohost`. That name is the node-less marker: rejected as
  `InvalidControl` in either field of ops 1/3/4/8, exempted from
  `MisAddressed` in the SEND arm (legacy tolerance, T-6). The literal
  PidRef-node-None checks remain as wire-unreachable defense-in-depth.
  `NONODE_NOHOST` became `pub(crate)`.
- **Serial normalization (amends §1 "To = RemotePid verbatim"):**
  `link_remote`/`unlink_remote` normalize `RemotePid.serial` to 0 at the
  facility boundary; a nonzero embedder serial otherwise breaks the DC-4
  remove-gate equality when the peer's EXIT mints serial 0 (review finding).
  Encoders unchanged; the wire carries the serial-0 identity.
- **NoConnection unwind (amends §3.7):** `link_remote`'s NoConnection arm
  unwinds the just-established local half-link before returning — a literal
  reading left the immortal half-link §3.5 names as the hazard.
- **Inbound post-establish recheck (extends §3.8):** `apply_inbound_link`
  rechecks the origin connection after establishing; if absent/down it
  delivers the missed `noconnection` (the DC-4 gate keeps both race orders
  exactly-once). Closes the inbound mirror of the outbound unwind against
  write-side downs (WriteTimeout/ControlOverflow/disconnect_node), whose
  backstop scan is NOT ordered after the read loop.
- **Store-back reconciliation (new, critical review finding):**
  `store_runnable_process`'s remote_links merge was add-only — removals
  recorded against an Executing process's metadata were resurrected,
  double-firing DC-4 and making `unlink/1` on a remote pid a no-op. The merge
  now reconciles removals with metadata authoritative, mirroring the monitors
  merge. The local `links` add-only merge is pre-existing (hazard 7, separate
  ruling).
- **GC rooting (new, review finding):** remote EXIT delivery to a trapping
  target allocates the external source pid and exit tuple as ONE contiguous
  8-word allocation behind one `ensure_space` — the two-step form left the
  pid unrooted across a GC-capable reservation.
- **u32 pid ceiling (amends §3.5 "unreachable" claim):** `wire_encodable`
  guards the senders; `send_link` maps oversized pids to
  `RemoteLinkError::BadTarget`, `send_unlink`/`send_exit` drop (no such link
  can exist / ruling-7 best-effort). The encode-failure connection-down arm
  remains as backstop and its rustdoc is now accurate.
- **Keepalive exclusion:** the scheduler control handler early-returns on
  empty control+payload so heartbeats do not increment
  `beamr.distribution.control_frames_dropped` (handler-side deliberately —
  the read loop's zero-length forwarding is hub-series-tested behavior).
- **Real sizes (supersedes §2's table for changelog/citation purposes; code
  lines excl. tests, at 02500c1):** control_link.rs 326; dist_control_out.rs
  101; remote_supervision.rs 145 (establish/remove stayed in
  supervision_integration.rs behind the re-export shim); sender.rs 77→128;
  connection.rs 831→850; control.rs 507→480; supervision_integration.rs
  1921→1852. control_lifecycle.rs was 513 lines at deletion, not 563.
- **Open (confirmed by review, deferred for design rulings):**
  (W2) generation-unpinned noconnection backstop — a link established
  against live g+1 during the Down(g) dispatch window is spuriously severed;
  the fix needs per-link session pinning through spec'd public API shapes
  (§3.3's handler signature, Process remote_links storage). (W3) the
  Executing should-die arm's ETS-heir transfer locks a second slot under the
  target's slot lock — pre-existing ABBA class (D9 byte-matches the local
  idiom); fix belongs inside shared_exit_tombstone/transfer.
  Also: `--no-default-features` check is red with 1020 pre-existing no-std
  errors throughout the series (count-identical at every baseline) — §12's
  gate leg is waived, not green; restoring no-std is a separate work item.
