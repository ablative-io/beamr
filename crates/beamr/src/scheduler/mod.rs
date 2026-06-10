//! Scheduler entry points.
//!
//! Native builds use the OS-threaded scheduler. WASM/no-threads builds expose a
//! cooperative scheduler that the host drives one event-loop turn at a time.

#[cfg(feature = "threads")]
mod native;
#[cfg(feature = "threads")]
pub use native::*;

pub mod wasm;
pub use wasm::WasmScheduler;

#[cfg(not(feature = "threads"))]
pub mod dirty {
    /// Distinguishes BEAM-style dirty scheduler pools.
    ///
    /// The enum remains available for import-resolution metadata on WASM, but
    /// no OS dirty schedulers are created when the `threads` feature is off.
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub enum DirtySchedulerKind {
        /// CPU-bound dirty work.
        Cpu,
        /// IO-bound dirty work.
        Io,
    }
}

#[cfg(not(feature = "threads"))]
pub const DEFAULT_REDUCTION_BUDGET: u32 = crate::process::DEFAULT_REDUCTION_BUDGET;
