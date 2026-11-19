//! Process — the unit of life and isolation.
//!
//! Each process owns its heap, stack, mailbox placeholder, reduction counter,
//! link/monitor sets, and status. Processes share no memory. Spawning costs
//! microseconds. A process that crashes takes only itself down — the rest of the
//! system is unaffected.
pub mod gc;
pub mod heap;
pub mod registry;
mod relations;
pub mod stack;
mod types;

mod code;
mod collection;
mod dictionary;
mod exceptions;
mod jit;
mod lifecycle;
mod logical_clock;
mod memory;
mod natives;
mod registers;
mod scheduling;
mod suspension;
#[cfg(feature = "telemetry")]
mod telemetry;

pub use natives::PendingNativeContinuation;
pub(crate) use relations::SharedRelations;
pub use suspension::{ResumeContinuation, SuspensionKind, SuspensionRecord};
pub use types::*;

// `Box` comes from std's prelude, which is absent under
// `--no-default-features` — and `jit_transfer` boxes its payload, which added
// two errors to the no_std tally (1072 -> 1074). The ratchet's waiver permits
// that debt to STAND, not to GROW, so the sites are made resolvable instead.
// Guarded exactly as `ar1_shape_control.rs` guards its own `alloc` import,
// because lib.rs's `extern crate alloc` is itself conditional — an unguarded
// `use alloc::…` does not resolve in a std build.
#[cfg(not(feature = "std"))]
use alloc::boxed::Box;

use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use crate::mailbox::Mailbox;
use crate::module::Module;
use crate::namespace::NamespaceId;
use crate::native::native_process::NativeBody;
use crate::process::heap::Heap;
use crate::process::stack::Stack;
use crate::term::Term;

/// Default number of reductions assigned to a fresh process time slice.
pub const DEFAULT_REDUCTION_BUDGET: u32 = 4000;

/// Number of X registers in a process register file.
///
/// This is the fixed width of [`Process`]'s `x_regs` array. Opcodes whose
/// bytecode carries no compiler-emitted live count (e.g. `put_list`,
/// `put_tuple2`, `update_record`) must pass this full width to
/// [`crate::gc::ensure_space`] so a defensive in-opcode collection roots every
/// register and never clears a live one.
pub(crate) const X_REG_COUNT: usize = 1024;

/// One isolated BEAM-style process.
///
/// The `Rc` marker intentionally makes `Process` neither [`Send`] nor [`Sync`]:
/// ownership must remain with one scheduler thread at a time.
///
/// ```compile_fail
/// use beamr::process::Process;
///
/// let process = Process::new(0, 233);
/// std::thread::spawn(move || {
///     process.pid()
/// });
/// ```
///
/// ```compile_fail
/// fn assert_sync<T: Sync>() {}
/// assert_sync::<beamr::process::Process>();
/// ```
#[derive(Debug)]
pub struct Process {
    pid: u64,
    capabilities: crate::native::CapabilitySet,
    status: ProcessStatus,
    priority: Priority,
    heap: Heap,
    virtual_binary_heap: usize,
    stack: Stack,
    mailbox: Mailbox,
    handlers: Vec<ExceptionHandler>,
    /// Lowest `handlers` index a raise may consume in the current interpreter
    /// nesting. Zero at the outermost run; a nested run (a compiled body-call
    /// serviced by `jit_call_interpreted`) raises it to the handler count at
    /// entry, so a raise inside that run cannot pop a handler installed by the
    /// code that entered it — that handler's catch label lives on the far side
    /// of native frames the nested run has no way to return through.
    nested_handler_floor: usize,
    current_exception: Option<Exception>,
    dictionary: Vec<(Term, Term)>,
    receive_timeout: Option<ReceiveTimeout>,
    receive_timer_ref: Option<u64>,
    suspension: Option<SuspensionRecord>,
    next_suspension_call_id: u64,
    x_regs: [Term; X_REG_COUNT],
    float_regs: [f64; 16],
    native_continuations: Vec<PendingNativeContinuation>,
    /// Explicit GC roots registered by native code for terms it must keep
    /// alive across allocations. See `ProcessContext::root_term`.
    native_roots: Vec<Term>,
    raw_stacktrace: Vec<RawStackEntry>,
    reduction_counter: u32,
    logical_clock: u64,
    /// Collections ATTEMPTED on this process's heap, ever. Incremented on entry
    /// to a collection, before it can fail.
    gc_attempts: u64,
    /// Collections that COMPLETED successfully. Incremented only on `Ok`.
    ///
    /// ⭐ TWO COUNTERS, DELIBERATELY. `attempts > completions` is the only way
    /// to observe a collection that was attempted and FAILED, and that is the
    /// exact case that made the previous instrument useless: a completion-only
    /// counter reads zero delta on a failed collection and reports "nothing
    /// happened", which is the same blindness as inferring collections from
    /// heap resizes. See `docs/design/beamr/briefs/COLLECTION-OBSERVABLE-NOTE.md`.
    gc_completions: u64,
    namespace_id: NamespaceId,
    code_position: Option<CodePosition>,
    current_module: Option<Arc<Module>>,
    #[cfg(feature = "jit")]
    jit_runtime_context: Option<JitRuntimeContext>,
    jit_status: Option<JitStatus>,
    /// Pending execution error raised by a JIT runtime helper. Rides the
    /// process because compiled code has no `Result` channel: the helper parks
    /// the error here and routes to the deopt return, and `call_native` checks
    /// this field BEFORE treating the status as a restartable deopt — the error
    /// aborts the slice exactly like the interpreter's own `Err(ExecError)`,
    /// never re-executing the function.
    jit_exec_error: Option<crate::error::ExecError>,
    /// Scheduler-level transfer produced by a nested interpreted run inside a
    /// JIT helper. Rides the process for the same reason `jit_exec_error` does:
    /// compiled code can only return a word, and a transfer is not a value.
    ///
    /// The helper that parks one here must leave the resume position the nested
    /// run established IN PLACE, and `call_native` checks this field before
    /// treating the status as a restartable deopt. Boxed because the transfer
    /// is a rare, transient occupant of a struct that is allocated per process.
    jit_transfer: Option<Box<crate::interpreter::ExecutionResult>>,
    #[cfg(feature = "telemetry")]
    receive_wait_started: Option<crate::telemetry::spans::ReceiveWaitStarted>,
    #[cfg(feature = "telemetry")]
    trace_context: Option<crate::telemetry::spans::ProcessTraceContext>,
    links: Arc<relations::LinkList<u64>>,
    remote_links: Arc<relations::LinkList<RemotePid>>,
    monitors: Arc<relations::MonitorTable>,
    trap_exit: bool,
    group_leader: Term,
    /// Optional native handler body. `Some` exactly for native processes
    /// (Shape B). Excluded from structural [`Clone`] — a clone is non-native.
    native: Option<NativeBody>,
    not_send_sync: PhantomData<Rc<()>>,
}

impl Clone for Process {
    fn clone(&self) -> Self {
        let mut heap = self.heap.clone();
        heap.rebase_snapshot_terms(&self.heap);
        let mut clone = Self {
            pid: self.pid,
            capabilities: self.capabilities.clone(),
            status: self.status,
            priority: self.priority,
            heap,
            virtual_binary_heap: self.virtual_binary_heap,
            stack: self.stack.clone(),
            mailbox: self.mailbox.clone(),
            handlers: self.handlers.clone(),
            nested_handler_floor: self.nested_handler_floor,
            current_exception: self.current_exception,
            dictionary: self.dictionary.clone(),
            receive_timeout: self.receive_timeout,
            receive_timer_ref: self.receive_timer_ref,
            suspension: self.suspension,
            next_suspension_call_id: self.next_suspension_call_id,
            x_regs: self.x_regs,
            float_regs: self.float_regs,
            native_continuations: self.native_continuations.clone(),
            native_roots: self.native_roots.clone(),
            raw_stacktrace: self.raw_stacktrace.clone(),
            reduction_counter: self.reduction_counter,
            logical_clock: self.logical_clock,
            // A clone INHERITS both counts rather than resetting them. The
            // counters exist to answer "did a collection happen across this
            // region?", and a reset would make that delta read zero for a
            // reason that has nothing to do with collection — the same class of
            // spurious zero the resize proxy produced.
            gc_attempts: self.gc_attempts,
            gc_completions: self.gc_completions,
            namespace_id: self.namespace_id,
            code_position: self.code_position,
            current_module: self.current_module.clone(),
            #[cfg(feature = "jit")]
            jit_runtime_context: self.jit_runtime_context,
            jit_status: self.jit_status,
            jit_exec_error: self.jit_exec_error.clone(),
            jit_transfer: self.jit_transfer.clone(),
            #[cfg(feature = "telemetry")]
            receive_wait_started: self.receive_wait_started,
            #[cfg(feature = "telemetry")]
            trace_context: self.trace_context.clone(),
            links: self.links.clone(),
            remote_links: self.remote_links.clone(),
            monitors: self.monitors.clone(),
            trap_exit: self.trap_exit,
            group_leader: self.group_leader,
            // A `Box<dyn NativeHandler>` is not `Clone`, so a structural clone
            // is deliberately non-native. See the `Process::clone` audit note
            // in `is_native` — no live scheduler path clones a native process.
            native: None,
            not_send_sync: PhantomData,
        };
        clone.rebase_roots_from(self);
        clone
    }
}

impl Process {
    /// Create a fresh process with `pid` and a heap capacity of `heap_size`
    /// words.
    #[must_use]
    pub fn new(pid: u64, heap_size: usize) -> Self {
        Self::with_capabilities(pid, heap_size, crate::native::CapabilitySet::all())
    }

    /// Create a fresh process with an explicit capability set.
    #[must_use]
    pub fn with_capabilities(
        pid: u64,
        heap_size: usize,
        capabilities: crate::native::CapabilitySet,
    ) -> Self {
        Self {
            pid,
            capabilities,
            status: ProcessStatus::New,
            priority: Priority::Normal,
            heap: Heap::new(heap_size),
            virtual_binary_heap: 0,
            stack: Stack::new(),
            mailbox: Mailbox::new(),
            handlers: Vec::new(),
            nested_handler_floor: 0,
            current_exception: None,
            dictionary: Vec::new(),
            receive_timeout: None,
            receive_timer_ref: None,
            suspension: None,
            next_suspension_call_id: 0,
            x_regs: [Term::NIL; X_REG_COUNT],
            float_regs: [0.0; 16],
            native_continuations: Vec::new(),
            native_roots: Vec::new(),
            raw_stacktrace: Vec::new(),
            reduction_counter: DEFAULT_REDUCTION_BUDGET,
            logical_clock: 0,
            gc_attempts: 0,
            gc_completions: 0,
            namespace_id: NamespaceId::DEFAULT,
            code_position: None,
            current_module: None,
            #[cfg(feature = "jit")]
            jit_runtime_context: None,
            jit_status: None,
            jit_exec_error: None,
            jit_transfer: None,
            #[cfg(feature = "telemetry")]
            receive_wait_started: None,
            #[cfg(feature = "telemetry")]
            trace_context: None,
            links: Arc::default(),
            remote_links: Arc::default(),
            monitors: Arc::default(),
            trap_exit: false,
            group_leader: Self::initial_group_leader(pid),
            native: None,
            not_send_sync: PhantomData,
        }
    }

    /// Process identifier.
    #[must_use]
    pub const fn pid(&self) -> u64 {
        self.pid
    }

    const fn initial_group_leader(pid: u64) -> Term {
        match Term::try_pid(pid) {
            Some(pid_term) => pid_term,
            None => Term::NIL,
        }
    }

    /// Capabilities granted to this process.
    #[must_use]
    pub const fn capabilities(&self) -> &crate::native::CapabilitySet {
        &self.capabilities
    }

    /// Replace this process's capability set before it is made runnable.
    pub fn set_capabilities(&mut self, capabilities: crate::native::CapabilitySet) {
        self.capabilities = capabilities;
    }

    fn rebase_roots_from(&mut self, original: &Self) {
        for root in &mut self.x_regs {
            *root = self.heap.rebase_term_from(*root, &original.heap);
        }
        for root in self.stack.y_regs_mut() {
            *root = self.heap.rebase_term_from(*root, &original.heap);
        }
        for root in self.mailbox.scan_iter_mut() {
            *root = self.heap.rebase_term_from(*root, &original.heap);
        }
        for entry in &mut self.raw_stacktrace {
            entry.location_info = self
                .heap
                .rebase_term_from(entry.location_info, &original.heap);
        }
        if let Some(exception) = &mut self.current_exception {
            exception.class = self.heap.rebase_term_from(exception.class, &original.heap);
            exception.reason = self.heap.rebase_term_from(exception.reason, &original.heap);
            exception.stacktrace = self
                .heap
                .rebase_term_from(exception.stacktrace, &original.heap);
        }
        for (key, value) in &mut self.dictionary {
            *key = self.heap.rebase_term_from(*key, &original.heap);
            *value = self.heap.rebase_term_from(*value, &original.heap);
        }
        self.group_leader = self
            .heap
            .rebase_term_from(self.group_leader, &original.heap);
    }

    /// Whether this process traps exits.
    #[must_use]
    pub const fn trap_exit(&self) -> bool {
        self.trap_exit
    }

    /// Set whether this process traps exits.
    pub const fn set_trap_exit(&mut self, trap_exit: bool) {
        self.trap_exit = trap_exit;
    }

    /// Group leader PID term.
    #[must_use]
    pub const fn group_leader(&self) -> Term {
        self.group_leader
    }

    /// Set group leader PID term.
    pub const fn set_group_leader(&mut self, group_leader: Term) {
        self.group_leader = group_leader;
    }
}

#[cfg(test)]
mod tests;
