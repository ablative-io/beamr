//! Minimal process-facing context exposed to native code.
//!
//! Native functions deliberately receive this allocation subset instead of the
//! full process so they cannot inspect scheduler, mailbox, or process internals.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::atom::AtomTable;
use crate::io::{IoSink, NullSink};
use crate::native::stdlib_stubs::{lists_bifs::ListsMapState, maps_bifs::MapsHofState};
use crate::process::heap::Heap;
use crate::term::Term;
use crate::timer::{TimerRef, TimerWheel};

use super::code_management_bifs::CodeManagementFacility;
use super::links::LinkFacility;
use super::registry::RegistryFacility;
use super::select::SelectFacility;
use super::spawn::SpawnFacility;
use super::supervision::SupervisionFacility;

/// Minimal process-facing context exposed to native code.
///
/// Native functions deliberately receive this allocation subset instead of the
/// full process so they cannot inspect scheduler, mailbox, or process internals.
/// Trampoline request from a BIF that needs interpreter re-entry.
///
/// When a BIF returns normally but needs the interpreter to call a BEAM
/// closure and use the closure's return value as the BIF's result, it stores
/// a `TrampolineRequest` in the process context. The interpreter checks for
/// this after each BIF call.
#[derive(Clone, Debug)]
pub struct TrampolineRequest {
    /// The closure (fun) term to invoke.
    pub fun: Term,
    /// Arguments to pass to the closure.
    pub args: Vec<Term>,
    /// Optional native continuation to resume after the closure returns.
    pub continuation: Option<NativeContinuation>,
}

/// Native continuation state for collection BIFs that call closures repeatedly.
#[derive(Clone, Debug)]
pub enum NativeContinuation {
    /// Continuation for maps higher-order BIFs.
    Maps(MapsHofState),
    /// Continuation for lists:map/2.
    ListsMap(ListsMapState),
    /// Continuation for Gleam result.try/2 compatibility.
    GleamResultTry,
}

/// Suspend request from a BIF that wants the process to wait.
///
/// Used by `select` when no mailbox message matches any handler.
#[derive(Copy, Clone, Debug)]
pub struct SuspendRequest {
    /// Optional timeout in milliseconds. `None` means wait indefinitely.
    pub timeout_ms: Option<u64>,
}

pub struct ProcessContext {
    pid: Option<u64>,
    heap: Option<*mut Heap>,
    owned_allocations: Vec<Box<[u64]>>,
    timers: Option<Arc<Mutex<TimerWheel>>>,
    atom_table: Option<Arc<AtomTable>>,
    spawn_facility: Option<Arc<dyn SpawnFacility>>,
    link_facility: Option<Arc<dyn LinkFacility>>,
    supervision_facility: Option<Arc<dyn SupervisionFacility>>,
    code_management_facility: Option<Arc<dyn CodeManagementFacility>>,
    registry_facility: Option<Arc<dyn RegistryFacility>>,
    select_facility: Option<Arc<dyn SelectFacility>>,
    io_sink: Arc<dyn IoSink>,
    shutdown_requested: bool,
    trampoline: Option<TrampolineRequest>,
    suspend: Option<SuspendRequest>,
}

impl fmt::Debug for ProcessContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessContext")
            .field("pid", &self.pid)
            .field("heap", &self.heap.as_ref().map(|_| ".."))
            .field("timers", &self.timers)
            .field("atom_table", &self.atom_table.as_ref().map(|_| ".."))
            .field(
                "spawn_facility",
                &self.spawn_facility.as_ref().map(|_| ".."),
            )
            .field("link_facility", &self.link_facility.as_ref().map(|_| ".."))
            .field(
                "supervision_facility",
                &self.supervision_facility.as_ref().map(|_| ".."),
            )
            .field(
                "code_management_facility",
                &self.code_management_facility.as_ref().map(|_| ".."),
            )
            .field(
                "registry_facility",
                &self.registry_facility.as_ref().map(|_| ".."),
            )
            .field(
                "select_facility",
                &self.select_facility.as_ref().map(|_| ".."),
            )
            .field("io_sink", &"..")
            .field("shutdown_requested", &self.shutdown_requested)
            .field("trampoline", &self.trampoline)
            .field("suspend", &self.suspend)
            .finish()
    }
}

impl Default for ProcessContext {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessContext {
    /// Creates an empty process context.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pid: None,
            heap: None,
            owned_allocations: Vec::new(),
            timers: None,
            atom_table: None,
            spawn_facility: None,
            link_facility: None,
            supervision_facility: None,
            code_management_facility: None,
            registry_facility: None,
            select_facility: None,
            io_sink: Arc::new(NullSink),
            trampoline: None,
            suspend: None,
            shutdown_requested: false,
        }
    }

    /// Creates a context connected to the calling process heap.
    #[must_use]
    pub fn with_process_heap(pid: u64, heap: &mut Heap) -> Self {
        let mut context = Self::new();
        context.pid = Some(pid);
        context.heap = Some(heap as *mut Heap);
        context
    }

    /// Creates a context connected to timer services and the calling process heap.
    #[must_use]
    pub fn with_timer_services_and_heap(
        pid: u64,
        timers: Arc<Mutex<TimerWheel>>,
        heap: &mut Heap,
    ) -> Self {
        let mut context = Self::with_timer_services(pid, timers);
        context.heap = Some(heap as *mut Heap);
        context
    }

    /// Creates a context with timer services for asynchronous timer BIFs.
    #[must_use]
    pub fn with_timer_services(pid: u64, timers: Arc<Mutex<TimerWheel>>) -> Self {
        Self {
            pid: Some(pid),
            heap: None,
            owned_allocations: Vec::new(),
            timers: Some(timers),
            atom_table: None,
            spawn_facility: None,
            link_facility: None,
            supervision_facility: None,
            code_management_facility: None,
            registry_facility: None,
            select_facility: None,
            io_sink: Arc::new(NullSink),
            trampoline: None,
            suspend: None,
            shutdown_requested: false,
        }
    }

    /// Return the calling process id when provided by the runtime.
    #[must_use]
    pub fn pid(&self) -> Option<u64> {
        self.pid
    }

    /// Set the calling process id.
    pub fn set_pid(&mut self, pid: Option<u64>) {
        self.pid = pid;
    }

    /// Return the spawn facility, if one has been configured.
    #[must_use]
    pub fn spawn_facility(&self) -> Option<&dyn SpawnFacility> {
        self.spawn_facility.as_deref()
    }

    /// Set the spawn facility for process creation BIFs.
    pub fn set_spawn_facility(&mut self, facility: Option<Arc<dyn SpawnFacility>>) {
        self.spawn_facility = facility;
    }

    /// Return the link facility, if one has been configured.
    #[must_use]
    pub fn link_facility(&self) -> Option<&dyn LinkFacility> {
        self.link_facility.as_deref()
    }

    /// Set the link facility for link management BIFs.
    pub fn set_link_facility(&mut self, facility: Option<Arc<dyn LinkFacility>>) {
        self.link_facility = facility;
    }

    /// Return the supervision facility, if one has been configured.
    #[must_use]
    pub fn supervision_facility(&self) -> Option<&dyn SupervisionFacility> {
        self.supervision_facility.as_deref()
    }

    /// Set the supervision facility for monitor/demonitor/exit BIFs.
    pub fn set_supervision_facility(&mut self, facility: Option<Arc<dyn SupervisionFacility>>) {
        self.supervision_facility = facility;
    }

    /// Return the code-management facility, if one has been configured.
    #[must_use]
    pub fn code_management_facility(&self) -> Option<&dyn CodeManagementFacility> {
        self.code_management_facility.as_deref()
    }

    /// Set the code-management facility for hot-code BIFs.
    pub fn set_code_management_facility(
        &mut self,
        facility: Option<Arc<dyn CodeManagementFacility>>,
    ) {
        self.code_management_facility = facility;
    }

    /// Return the atom table, if one has been configured.
    #[must_use]
    pub fn atom_table(&self) -> Option<&AtomTable> {
        self.atom_table.as_deref()
    }

    /// Set the atom table for type conversion BIFs.
    pub fn set_atom_table(&mut self, table: Option<Arc<AtomTable>>) {
        self.atom_table = table;
    }

    /// Return the registry facility, if one has been configured.
    #[must_use]
    pub fn registry_facility(&self) -> Option<&dyn RegistryFacility> {
        self.registry_facility.as_deref()
    }

    /// Set the registry facility for process name registry BIFs.
    pub fn set_registry_facility(&mut self, facility: Option<Arc<dyn RegistryFacility>>) {
        self.registry_facility = facility;
    }

    /// Schedule a timer via the runtime timer wheel.
    pub fn schedule_timer(
        &mut self,
        delay: Duration,
        target_pid: u64,
        message: Term,
    ) -> Option<TimerRef> {
        let timers = self.timers.as_ref()?;
        Some(
            timers
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .schedule(delay, target_pid, message),
        )
    }

    /// Reserve a timer reference and schedule with a message derived from it.
    pub fn schedule_timer_with_reference<F>(
        &mut self,
        delay: Duration,
        target_pid: u64,
        message: F,
    ) -> Option<TimerRef>
    where
        F: FnOnce(TimerRef) -> Term,
    {
        let timers = self.timers.as_ref()?;
        let mut timers = timers.lock().unwrap_or_else(|error| error.into_inner());
        let reference = timers.reserve_reference();
        timers.schedule_reserved(reference, delay, target_pid, message(reference))
    }

    /// Cancel a timer via the runtime timer wheel.
    pub fn cancel_timer(&mut self, reference: TimerRef) -> Option<Duration> {
        let timers = self.timers.as_ref()?;
        timers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .cancel(reference)
    }

    /// Allocates a term on the calling process heap.
    ///
    /// Gate 1 only has immediate terms, so this currently returns the term
    /// unchanged. Boxed values can later route through the process heap without
    /// changing the native calling convention.
    pub const fn allocate_term(&mut self, term: Term) -> Term {
        term
    }

    // --- Select facility ---

    /// Return the select facility, if one has been configured.
    #[must_use]
    pub fn select_facility(&self) -> Option<&dyn SelectFacility> {
        self.select_facility.as_deref()
    }

    /// Set the select facility for mailbox scanning BIFs.
    pub fn set_select_facility(&mut self, facility: Option<Arc<dyn SelectFacility>>) {
        self.select_facility = facility;
    }

    /// Return the configured output sink for `io` module BIFs.
    #[must_use]
    pub fn io_sink(&self) -> &dyn IoSink {
        self.io_sink.as_ref()
    }

    /// Set the output sink for `io` module BIFs.
    pub fn set_io_sink(&mut self, sink: Arc<dyn IoSink>) {
        self.io_sink = sink;
    }

    /// Request runtime shutdown after the current BIF returns.
    pub fn request_shutdown(&mut self) {
        self.shutdown_requested = true;
    }

    /// Take and clear the shutdown request flag.
    pub fn take_shutdown_request(&mut self) -> bool {
        let requested = self.shutdown_requested;
        self.shutdown_requested = false;
        requested
    }

    // --- Trampoline ---

    /// Store a trampoline request for the interpreter to execute.
    ///
    /// The interpreter checks for a trampoline after each BIF call. When
    /// present, it sets up the closure call and uses the closure's return
    /// value as the BIF's return value.
    pub fn set_trampoline(&mut self, fun: Term, args: Vec<Term>) {
        self.trampoline = Some(TrampolineRequest {
            fun,
            args,
            continuation: None,
        });
    }

    /// Store a trampoline request with native continuation state.
    pub fn set_continuation_trampoline(
        &mut self,
        fun: Term,
        args: Vec<Term>,
        continuation: NativeContinuation,
    ) {
        self.trampoline = Some(TrampolineRequest {
            fun,
            args,
            continuation: Some(continuation),
        });
    }

    /// Take the trampoline request, clearing it from the context.
    ///
    /// Returns `None` if no trampoline was requested.
    pub fn take_trampoline(&mut self) -> Option<TrampolineRequest> {
        self.trampoline.take()
    }

    /// Check whether a trampoline request is pending.
    #[must_use]
    pub fn has_trampoline(&self) -> bool {
        self.trampoline.is_some()
    }

    // --- Suspend ---

    /// Request that the process be suspended (waiting for messages).
    ///
    /// Called by `select` when no mailbox message matches any handler.
    pub fn request_suspend(&mut self, timeout_ms: Option<u64>) {
        self.suspend = Some(SuspendRequest { timeout_ms });
    }

    /// Take the suspend request, clearing it from the context.
    pub fn take_suspend(&mut self) -> Option<SuspendRequest> {
        self.suspend.take()
    }

    // --- Heap allocation helpers ---

    fn alloc_words(&mut self, words: usize) -> Result<&mut [u64], Term> {
        if let Some(heap) = self.heap {
            // SAFETY: the interpreter creates this context from the currently
            // running process and invokes native code synchronously. The raw
            // handle is not shared outside the context and is used only to
            // perform immediate process-heap allocations.
            let ptr = unsafe { &mut *heap }
                .alloc(words)
                .map_err(|_| Term::atom(crate::atom::Atom::BADARG))?;
            return Ok(crate::interpreter::opcodes::core::heap_slice(ptr, words));
        }

        self.owned_allocations
            .push(vec![0u64; words].into_boxed_slice());
        self.owned_allocations
            .last_mut()
            .map(|allocation| allocation.as_mut())
            .ok_or_else(|| Term::atom(crate::atom::Atom::BADARG))
    }

    /// Allocate a tuple on the calling process heap.
    pub fn alloc_tuple(&mut self, elements: &[Term]) -> Result<Term, Term> {
        let words = 1 + elements.len();
        let heap = self.alloc_words(words)?;
        crate::term::boxed::write_tuple(heap, elements)
            .ok_or_else(|| Term::atom(crate::atom::Atom::BADARG))
    }

    /// Allocate a cons cell on the calling process heap.
    pub fn alloc_cons(&mut self, head: Term, tail: Term) -> Result<Term, Term> {
        let heap = self.alloc_words(2)?;
        crate::term::boxed::write_cons(heap, head, tail)
            .ok_or_else(|| Term::atom(crate::atom::Atom::BADARG))
    }

    /// Allocate a float on the calling process heap.
    pub fn alloc_float(&mut self, value: f64) -> Result<Term, Term> {
        let heap = self.alloc_words(2)?;
        crate::term::boxed::write_float(heap, value)
            .ok_or_else(|| Term::atom(crate::atom::Atom::BADARG))
    }

    /// Allocate a binary on the calling process heap.
    pub fn alloc_binary(&mut self, bytes: &[u8]) -> Result<Term, Term> {
        let words = 2 + crate::term::binary::packed_word_count(bytes.len());
        let heap = self.alloc_words(words)?;
        crate::term::binary::write_binary(heap, bytes)
            .ok_or_else(|| Term::atom(crate::atom::Atom::BADARG))
    }

    /// Allocate a bigint on the calling process heap.
    pub fn alloc_bigint(&mut self, negative: bool, limbs: &[u64]) -> Result<Term, Term> {
        let heap = self.alloc_words(3 + limbs.len())?;
        crate::term::boxed::write_bigint(heap, negative, limbs)
            .ok_or_else(|| Term::atom(crate::atom::Atom::BADARG))
    }

    /// Allocate a proper list on the calling process heap.
    pub fn alloc_list(&mut self, elements: &[Term]) -> Result<Term, Term> {
        let mut tail = Term::NIL;
        for element in elements.iter().rev() {
            tail = self.alloc_cons(*element, tail)?;
        }
        Ok(tail)
    }

    /// Allocate a map on the calling process heap.
    pub fn alloc_map(&mut self, keys: &[Term], values: &[Term]) -> Result<Term, Term> {
        let heap = self.alloc_words(2 + keys.len() + values.len())?;
        crate::term::boxed::write_map(heap, keys, values)
            .ok_or_else(|| Term::atom(crate::atom::Atom::BADARG))
    }
}
