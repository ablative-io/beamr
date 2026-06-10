//! Beamr — a Rust runtime with the BEAM's execution model.
//!
//! Loads `.beam` bytecode produced by the Gleam toolchain (via `erlc`)
//! and executes it with preemptive scheduling, per-process isolation,
//! supervision primitives, and a native function interface.
#![cfg_attr(all(not(feature = "std"), not(target_arch = "wasm32")), no_std)]

#[cfg(any(not(feature = "std"), target_arch = "wasm32"))]
extern crate alloc;

pub mod atom;
pub mod capability;
pub mod constant_pool;
#[cfg(feature = "net")]
pub mod distribution;
pub mod error;
pub mod etf;
pub mod ets;
pub mod gc;
#[cfg(feature = "threads")]
pub mod hook;
pub mod interpreter;
#[cfg(feature = "threads")]
#[path = "io/mod.rs"]
pub mod io;
#[cfg(feature = "jit")]
pub mod jit;
pub mod loader;
pub mod mailbox;
pub mod module;
pub mod namespace;
pub mod native;
pub mod process;
#[cfg(feature = "threads")]
pub mod replay;
pub mod scheduler;
pub mod supervision;
#[cfg(feature = "telemetry")]
pub mod telemetry;
pub mod term;
#[cfg(feature = "threads")]
pub mod timer;
