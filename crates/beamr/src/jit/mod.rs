//! JIT compilation infrastructure.

pub mod compile_job;
pub mod compiler;
pub mod profiler;
pub mod types;

pub use compile_job::{CompilationJob, submit_jit_compilation};
pub use compiler::{JitCompiler, JitError, JitSettings};
pub use profiler::{JitProfiler, MfaKey, RecordResult};
pub use types::{NativeCode, RootLocation, StackMapEntry};
