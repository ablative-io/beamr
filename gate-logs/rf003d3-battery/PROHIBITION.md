# RF-003-D3 — D-e PROHIBITION GREP over `crates/beamr/src/distribution/`

Runnable standalone; artifacts are the three `prohibition-N-*.log` files and
their own `prohibition-N-*.rc` files. No `-q`, no `2>/dev/null`, no `|| true`.

**A substring count is not an occurrence count — every hit below is WINDOWED**
(production vs `#[cfg(test)]`, and wire-derived vs locally-derived length).
`#[cfg(test)]` boundaries: `sender.rs:662`, `etf.rs:1062`, `connection.rs` tests
follow `:3300`-ish.

## (1) Zeroed-vec allocations — `prohibition-1-veczero.log`, rc 0, 13 hits

| site | window | length source | verdict |
| --- | --- | --- | --- |
| `sender.rs:754,839,947,1046,1257,1815,2037` | test (>662) | the test's OWN written frame | not production |
| `sender.rs:969,1126,1333` | test (>662) | fixed literal `16 * 1024 * 1024` | not wire-derived |
| `sender.rs:1632` | test (>662) | local `body` variable | not wire-derived |
| `handshake.rs:524,579` | **production** | `u16::from_be_bytes(..) as usize` | **out of lane — see finding** |

**Zero production `vec![0_u8; N]` with a wire-derived N remains in the framing
path.** The only two production hits are in `handshake.rs`, which the brief's
own boundary places outside this lane ("SCOPE: the handshake path … is NOT in
this lane. If the re-pin turns up the same unbounded-allocation shape there, it
is a finding for the handoff and a separate brief"). It is reported below, NOT
fixed.

## (2) Wire-length allocation sites — `prohibition-2-wirealloc.log`, rc 0

The two PRODUCTION FRAMING sites, both now cap-first:

| framing site | cap check | allocation | ordering |
| --- | --- | --- | --- |
| `etf.rs` `read_dist_message` | `:225 if length > MAX_DIST_FRAME_BYTES` | `:229 try_reserve_exact` | cap PRECEDES — **this lane** |
| `connection.rs` `frame_buffer_for_header` | `:196 if total_len > MAX_DIST_FRAME_BYTES` | `:206 try_reserve_exact` | cap PRECEDES — task #60, already landed |

Every other hit, windowed:

- `etf.rs:204,335`, `control.rs:268`, `sender.rs:698,1635`,
  `handshake.rs:591,609,616,624` — `Vec::with_capacity` on the ENCODE side,
  sized from data this node already holds. Not a peer-named length.
- `atom_cache.rs:39,40` — `ATOM_CACHE_SIZE`, a compile-time constant.
- `sender.rs:972,1129`, `connection.rs:3398` — inside `#[cfg(test)]`.
- `etf.rs:619,634,661,662,688,741` — the ETF TERM DECODER's fallible reservations
  (`try_reserve_exact` → `DecodeError::SizeLimitExceeded`). A different
  allocation class from framing: these read lengths out of a byte slice that is
  ALREADY RESIDENT (and, downstream of this lane, already bounded by the frame
  cap), they are fallible, and they are guarded by `ensure_decodable_sequence` /
  an immediately following `read_bytes(len)?`. **Stated rather than waved past:**
  the wall's literal wording ("no allocation from a wire length not preceded by
  a `MAX_DIST_FRAME_BYTES` check") would also name these, and they carry no such
  check. They are not in D-a's scope — the brief pins D-a to `read_dist_message`
  — and this file's decoder region is RF-002's. Recorded here so the next reader
  inherits the fact, not a claim of a clean sweep.

## (3) Cap sites — `prohibition-3-capsites.log`, rc 0

ONE constant, no second literal: `64 * 1024 * 1024` appears exactly once in the
distribution tree, at `etf.rs:92`. `connection.rs:23` and `sender.rs:169` (plus
`sender.rs:676` in its tests) IMPORT it; nothing re-mints it.

## FINDING (report, do not fix) — `handshake.rs:524` and `:579`

`read_packet_async` / `read_packet` both do
`let length = u16::from_be_bytes(length_bytes) as usize;` then
`let mut payload = vec![0_u8; length];` — an INFALLIBLE allocation from peer
bytes, with no `MAX_DIST_FRAME_BYTES` check.

**It is NOT the RF-003 shape, and saying otherwise would be a phantom.** The
length prefix is a `u16`, so the allocation is STRUCTURALLY BOUNDED at 65535
bytes by the type. There is no peer-nameable multi-gigabyte commit here; the
residual is that 64 KiB is taken from the infallible allocator, which aborts the
host process only on a box already out of memory. Out of lane by the brief's
boundary; handed off as a datum for whoever writes the handshake-framing brief.
