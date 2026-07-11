//! `.beam` container writer — the mirror image of [`super::decode`].
//!
//! Feature-gated behind `encode` (default-off, zero new dependencies). Shares
//! the decoder's `Operand`, `Instruction`, and chunk types, so no format
//! knowledge is duplicated: every encoder is an exact inverse of its decoder,
//! and the round-trip suite (`decode(encode(decode(x))) == decode(x)`) is the
//! ratchet that keeps them aligned.

mod chunks;
mod code;
mod compact;
mod container;
mod literals;
mod opcodes;

pub use container::{EncodeError, encode_module};
