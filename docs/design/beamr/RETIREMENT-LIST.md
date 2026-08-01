# Retirement list — conventions inherited from the library world that ownership lets us retire

**Authority**: Waffles the Terrible, ruling 4 part two (record `30f0d71c`,
2026-08-01), on the stack-ownership audit's findings 14/15/16, with the
coordinator's carve-out folded: *keep the cheap door (OTP opcode numerals),
drop the expensive aspiration (real-OTP wire compatibility as a blocking
constraint)*. Companion amendments landed the same day in
`briefs/RF-008.json` and `briefs/RF-003.json` (superseded text quoted in
place) and in `README.md` (the "Distributed Erlang" line no longer implies
OTP-node interop).

**THE LIST AUTHORIZES NOTHING.** Each item lands — or is explicitly kept —
through its own lane with its own brief, its own controlled zero
**re-measured at that lane's build base**, and its own gate battery. A
finding does not authorize its own follow-up; membership on this list is a
routing fact, not a license. The measurements below are pinned at
`17a10de` and go stale the moment the tree moves.

## Explicitly NOT on this list

- **OTP opcode numerals** (`ControlOp`, opcodes ≤ 31 in OTP shapes;
  `docs/DIST-CONTROL-WIRE-SPEC.md` calls this "the door to stock-OTP interop
  for this subset"). The door is cheap and preserves optionality. Kept by
  ruling.
- **The encoder-byte change-control** in RF-008/RF-003. It stands on
  persistence and mixed-version grounds (replay files on disk; rolling
  upgrades between beamr versions) — real, testable, and unaffected by
  dropping the OTP ground.

## Item 1 — `crates/beamr/src/distribution/atom_cache.rs` (554 lines, zero live consumers)

Measured at `17a10de`: the only references outside the file itself are the
`pub mod atom_cache;` declaration (`distribution/mod.rs:3`) and a handshake
test **asserting the offered distribution flags EXCLUDE the atom cache**
(`distribution/handshake.rs:856`, `offered_flags_exclude_atom_cache`) — the
runtime does not merely neglect this module, it deliberately opts out of the
capability on the wire. Positive control: the same search shape over the
sibling module name returns live call sites, so the zero is a measured zero,
not a dead instrument.

Retirement shape when its lane opens: deletion brief, re-measure the
consumer zero at that lane's base, keep the handshake exclusion test (it
pins wire behavior that outlives the module), and record the semver datum
(the module is `pub` via `pub mod`) in the release-boundary ledger before
landing.

## Item 2 — `negotiated_flags`: a DFLAG negotiation whose result is never read

Measured at `17a10de`: zero reads of `negotiated_flags` outside
`distribution/handshake.rs`. Positive control on the same struct at the
same call sites: the sibling accessor `remote_creation` IS read at
`distribution/connection.rs:1290` and `:1728`. The handshake negotiates a
flag set, stores it, and nothing downstream ever consults it.

Retirement shape when its lane opens: either stop carrying the result
(narrow the handshake output to what is read) or write down the reason it
must stay (e.g. a named future consumer with an owner and a trigger) — a
stored value nothing reads is a workaround-shaped feature nobody decided to
build. Same re-measure-at-base rule as Item 1.

## Adding to this list

An item needs: the inherited convention named, the audit-grade measurement
(zero + positive control, with the head it was taken at), the ownership
argument for why beamr does not need it, and what the ruled carve-out
keeps. Items leave this list only by a landed lane (retired or
explicitly kept-with-reason), never by edit.
