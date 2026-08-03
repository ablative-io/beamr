# BRIEF #64 — RED-FIRST artifact, captured at clean base

Captured BEFORE any conversion. Base at capture:
`92ca6c49724a68c47cca47141bd81978271b3178` plus the baseline-registration
commit (evidence only — no tracked source changed by it).

- Start (UTC): `2026-08-03T00:31:02Z`
- End (UTC): see `red-end.utc`

All three tests live in `crates/beamr/src/distribution/sender.rs`, in the
existing `#[cfg(test)] mod tests`. Return codes read directly with `echo $?` on
its own line; each command redirected to its log (no pipeline, no silence
redirection).

## 1. D2 evidence — control-frame encoded sizes, measured at the bytes

Test: `control_frame_encoded_sizes_measured_at_the_bytes`
Command: `cargo test -p beamr --lib distribution::sender::tests::control_frame_encoded_sizes_measured_at_the_bytes -- --nocapture --exact`
Return code: **0** (this one is a measurement, not a red assertion)
Log: `d2-measurement.log`

| Case | LINK | UNLINK | EXIT | EXIT2 |
| --- | --- | --- | --- | --- |
| typical node names (`local@127.0.0.1` / `peer@127.0.0.1`) | 74 B | 74 B | 88 B | 88 B |
| node names at the ETF atom ceiling (65535 B each) | 131117 B | 131117 B | **131131 B** | **131131 B** |

Worst-case control frame: **131131 bytes** (~128 KiB).
Control lane at full 256-slot occupancy: **33569536 bytes** (~32.0 MiB).

Why 131131 is a genuine CEILING and not just the largest case tried: a control
frame is `{Op, FromExtPid, ToExtPid[, ReasonAtom]}` over an **always-NIL
payload**, so it carries no user term whatsoever. Its only variable-length
components are

- the two node-name atoms, each length-ceilinged at 65535 bytes by
  `encode_atom_name` (`distribution/etf.rs:290` — `ATOM_UTF8_EXT` carries a
  `u16` length) and independently refused above `u16::MAX` by the handshake
  (`distribution/handshake.rs:787`); and
- the reason atom, drawn from `ExitReason`'s CLOSED six-atom set
  (`process/types.rs:245-257`), of which `noconnection` is the longest.

The pid components are fixed-width `u32`s. The 65535-byte node name is not a
hypothetical: it is exactly what a hostile peer may advertise and have
accepted through the handshake, so this row is the adversarial worst case.

**33569536 < 67108864**, i.e. the ENTIRE control lane at full occupancy retains
less than ONE maximum-size inbound data frame (`MAX_DIST_FRAME_BYTES`).

## 2. RED — data lane is byte-blind

Test: `data_lane_bounds_retained_bytes_not_just_slot_count`
Command: `cargo test -p beamr --lib distribution::sender::tests::data_lane_bounds_retained_bytes_not_just_slot_count -- --nocapture --exact`
Return code: **101** (FAILED — this is the red)
Log: `red-data-lane.log`

```
#64 RED (data lane): offered 64 frames / 268435968 bytes behind a parked drain; delivered 64 frames / 268435968 bytes

panicked at crates/beamr/src/distribution/sender.rs:1632:9:
data lane must bound RETAINED BYTES, not just slots: 64 frames (64 delivered)
carried 268435968 bytes through a lane whose byte budget is 134217728, with the
slot count at 64 of 1024
```

Shape of the demonstration: the drain is parked on a wedged peer (accepted but
never read, so `write_all` blocks until `WRITE_TIMEOUT`), and 64 frames of
4 MiB each are offered behind it. Each frame is far below
`MAX_DIST_FRAME_BYTES`, so no per-frame cap is in play; the COUNT is 64 of
1024 slots (6% full); the BYTES are 256 MiB against a 128 MiB
(`2 * MAX_DIST_FRAME_BYTES`) budget. Every frame is admitted and later
delivered, because nothing on this lane reads `frame.len()`.

## 3. RED — control lane is byte-blind

Test: `control_lane_bounds_retained_bytes_not_just_slot_count`
Command: `cargo test -p beamr --lib distribution::sender::tests::control_lane_bounds_retained_bytes_not_just_slot_count -- --nocapture --exact`
Return code: **101** (FAILED — this is the red)
Log: `red-control-lane.log`

```
#64 RED (control lane): offered 64 frames / 268435968 bytes into 256 slots behind a parked drain; accepted 64, refused 0

panicked at crates/beamr/src/distribution/sender.rs:1704:9:
control lane must bound RETAINED BYTES, not just slots: all 64 frames
(268435968 bytes) were admitted into 64 of 256 slots against a byte budget of
134217728
```

## What the two reds jointly establish, stated precisely

Both lanes are byte-blind **as API surfaces**: `DistOutbound::ToNode.frame` and
`ControlOutbound.frame` are both `pub` `Arc<[u8]>` fields on `pub` types with
`pub` enqueue methods, and neither enqueue path inspects `frame.len()`. The
control-lane red reaches that blindness through a synthetic caller.

The two lanes differ in whether that blindness is REACHABLE IN PRODUCTION, and
section 1 is the measurement that settles it:

- **Data lane** — the frames are minted by `scheduler::pg_propagation`
  (`pg_propagation.rs:52-72`) today, but the in-tree caveat at
  `connection.rs:81-86` records that nothing at the type level prevents a
  `DistOutbound::ToNode` from carrying a user payload, and `control::encode_frame`
  (`distribution/control.rs:259-274`) admits `u32` control and payload lengths.
  The lane's frames have NO structural ceiling. The byte bound is load-bearing.
- **Control lane** — its producers are enumerated and closed: every frame is
  minted by `scheduler::dist_control_out` through the `distribution::control_link`
  encoders, and section 1 measures their structural ceiling at 131131 bytes.
  256 slots x that ceiling is 32 MiB — a hard, computable residency bound.

The D2 disposition rests on that contrast, not on a preference.
