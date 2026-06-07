//! Minimal process-facing context exposed to native code.
//!
//! Native functions deliberately receive this allocation subset instead of the
//! full process so they cannot inspect scheduler, mailbox, or process internals.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::atom::{Atom, AtomTable};
use crate::io::{IoSink, NullSink};
use crate::native::stdlib_stubs::{lists_bifs::ListsMapState, maps_bifs::MapsHofState};
use crate::process::Process;
use crate::term::{
    Term,
    binary::{packed_word_count, write_binary},
    boxed::{write_bigint, write_cons, write_float, write_map, write_tuple},
};
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

pub struct ProcessContext<'process> {
    pid: Option<u64>,
    process: Option<&'process mut Process>,
    live_x: usize,
    scratch_allocations: Vec<Box<[u64]>>,
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

impl fmt::Debug for ProcessContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessContext")
            .field("pid", &self.pid)
            .field("process_heap", &self.process.as_ref().map(|_| ".."))
            .field("live_x", &self.live_x)
            .field("scratch_allocations", &self.scratch_allocations.len())
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

impl Default for ProcessContext<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'process> ProcessContext<'process> {
    /// Creates an empty process context.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pid: None,
            process: None,
            live_x: 256,
            scratch_allocations: Vec::new(),
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

    /// Creates a context with timer services for asynchronous timer BIFs.
    #[must_use]
    pub fn with_timer_services(pid: u64, timers: Arc<Mutex<TimerWheel>>) -> Self {
        Self {
            pid: Some(pid),
            process: None,
            live_x: 256,
            scratch_allocations: Vec::new(),
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

    /// Creates a context with access to the calling process heap.
    #[must_use]
    pub fn with_process(process: &'process mut Process, live_x: usize) -> Self {
        let pid = process.pid();
        let mut context = Self::new();
        context.pid = Some(pid);
        context.process = Some(process);
        context.live_x = live_x;
        context
    }

    /// Creates a context with process heap access and timer services.
    #[must_use]
    pub fn with_process_and_timer_services(
        process: &'process mut Process,
        live_x: usize,
        timers: Arc<Mutex<TimerWheel>>,
    ) -> Self {
        let mut context = Self::with_process(process, live_x);
        context.timers = Some(timers);
        context
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

    /// Reserve a timer reference without scheduling it yet.
    pub fn reserve_timer_reference(&mut self) -> Option<TimerRef> {
        let timers = self.timers.as_ref()?;
        Some(
            timers
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .reserve_reference(),
        )
    }

    /// Schedule a message using a previously reserved timer reference.
    pub fn schedule_reserved_timer(
        &mut self,
        reference: TimerRef,
        delay: Duration,
        target_pid: u64,
        message: Term,
    ) -> Option<TimerRef> {
        let timers = self.timers.as_ref()?;
        Some(
            timers
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .schedule_reserved(reference, delay, target_pid, message),
        )
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

    /// Ensure the calling process heap has at least `words` available nursery words.
    pub fn ensure_heap_space(&mut self, words: usize) -> Result<(), Term> {
        let process = self.process.as_deref_mut().ok_or_else(badarg)?;
        crate::gc::ensure_space(process, words, self.live_x).map_err(|_| badarg())
    }

    /// Return the calling process heap, when native code is executing in-process.
    #[must_use]
    pub fn process_heap(&self) -> Option<&crate::process::heap::Heap> {
        self.process.as_deref().map(Process::heap)
    }

    fn alloc_words(&mut self, words: usize) -> Result<&mut [u64], Term> {
        if self.process.is_some() {
            self.ensure_heap_space(words)?;
            let process = self.process.as_deref_mut().ok_or_else(badarg)?;
            let ptr = process.heap_mut().alloc(words).map_err(|_| badarg())?;
            Ok(crate::interpreter::opcodes::core::heap_slice(ptr, words))
        } else {
            self.scratch_allocations
                .push(vec![0_u64; words].into_boxed_slice());
            self.scratch_allocations
                .last_mut()
                .map_or_else(|| Err(badarg()), |allocation| Ok(allocation.as_mut()))
        }
    }

    /// Allocate a tuple on the calling process heap.
    pub fn alloc_tuple(&mut self, elements: &[Term]) -> Result<Term, Term> {
        let words = 1 + elements.len();
        let heap = self.alloc_words(words)?;
        write_tuple(heap, elements).ok_or_else(badarg)
    }

    /// Allocate a cons cell on the calling process heap.
    pub fn alloc_cons(&mut self, head: Term, tail: Term) -> Result<Term, Term> {
        let heap = self.alloc_words(2)?;
        write_cons(heap, head, tail).ok_or_else(badarg)
    }

    /// Allocate a float on the calling process heap.
    pub fn alloc_float(&mut self, value: f64) -> Result<Term, Term> {
        let heap = self.alloc_words(2)?;
        write_float(heap, value).ok_or_else(badarg)
    }

    /// Allocate an inline binary on the calling process heap.
    pub fn alloc_binary(&mut self, bytes: &[u8]) -> Result<Term, Term> {
        let words = 2 + packed_word_count(bytes.len());
        let heap = self.alloc_words(words)?;
        write_binary(heap, bytes).ok_or_else(badarg)
    }

    /// Allocate a big integer on the calling process heap.
    pub fn alloc_bigint(&mut self, negative: bool, limbs: &[u64]) -> Result<Term, Term> {
        let words = 3 + limbs.len();
        let heap = self.alloc_words(words)?;
        write_bigint(heap, negative, limbs).ok_or_else(badarg)
    }

    /// Allocate a proper list on the calling process heap.
    pub fn alloc_list(&mut self, elements: &[Term]) -> Result<Term, Term> {
        if elements.is_empty() {
            return Ok(Term::NIL);
        }

        let words = elements.len() * 2;
        let heap = self.alloc_words(words)?;
        let mut tail = Term::NIL;
        for (cell, element) in heap.chunks_exact_mut(2).rev().zip(elements.iter().rev()) {
            tail = write_cons(cell, *element, tail).ok_or_else(badarg)?;
        }
        Ok(tail)
    }

    /// Allocate a flatmap on the calling process heap.
    pub fn alloc_map(&mut self, keys: &[Term], values: &[Term]) -> Result<Term, Term> {
        if keys.len() != values.len() {
            return Err(badarg());
        }
        let words = 2 + keys.len() + values.len();
        let heap = self.alloc_words(words)?;
        write_map(heap, keys, values).ok_or_else(badarg)
    }
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

#[cfg(test)]
mod allocation_tests {
    use crate::process::{Process, Register};
    use crate::term::{
        Term,
        binary::Binary,
        boxed::{BigInt, Cons, Float, Map, Tuple},
    };

    use super::ProcessContext;

    fn assert_heap_term(process: &Process, term: Term) {
        let pointer = term
            .heap_ptr()
            .expect("boxed/list term should have heap pointer");
        assert!(process.heap().contains(pointer));
    }

    #[test]
    fn allocation_helpers_write_valid_terms_on_process_heap() {
        let mut process = Process::new(7, 32);
        let (float, binary, list, map, bigint) = {
            let mut context = ProcessContext::with_process(&mut process, 0);
            let float = context.alloc_float(1.5).expect("float allocation");
            let binary = context.alloc_binary(b"heap").expect("binary allocation");
            let list = context
                .alloc_list(&[Term::small_int(1), Term::small_int(2)])
                .expect("list allocation");
            let map = context
                .alloc_map(&[Term::atom(crate::atom::Atom::OK)], &[binary])
                .expect("map allocation");
            let bigint = context
                .alloc_bigint(false, &[Term::SMALL_INT_MAX as u64 + 1])
                .expect("bigint allocation");
            (float, binary, list, map, bigint)
        };

        assert_heap_term(&process, float);
        assert_heap_term(&process, binary);
        assert_heap_term(&process, list);
        assert_heap_term(&process, map);
        assert_heap_term(&process, bigint);
        assert_eq!(Float::new(float).map(|value| value.value()), Some(1.5));
        assert_eq!(
            Binary::new(binary).map(|value| value.as_bytes()),
            Some(&b"heap"[..])
        );
        let first = Cons::new(list).expect("list cons");
        assert_eq!(first.head(), Term::small_int(1));
        let map = Map::new(map).expect("map accessor");
        assert_eq!(map.value(0), Some(binary));
        assert!(BigInt::new(bigint).is_some());
    }

    #[test]
    fn rooted_allocation_survives_gc_and_unrooted_is_reclaimed() {
        let mut process = Process::new(7, 16);
        let tuple = {
            let mut context = ProcessContext::with_process(&mut process, 1);
            context
                .alloc_tuple(&[Term::atom(crate::atom::Atom::OK), Term::small_int(1)])
                .expect("tuple allocation")
        };
        process.set_x_reg(0, tuple);
        crate::gc::collect_minor_with_live(&mut process, 1).expect("minor gc");
        let rooted = process.x_reg(0);
        assert!(Tuple::new(rooted).is_some());
        assert_heap_term(&process, rooted);

        process.set_x_reg(0, Term::NIL);
        crate::gc::collect_major(&mut process).expect("major gc");
        assert_eq!(process.heap().young_used(), 0);
    }

    #[test]
    fn ensure_heap_space_collects_for_native_allocations() {
        let mut process = Process::new(7, 4);
        process.set_x_reg(Register::X(0), Term::NIL);
        let binary = {
            let mut context = ProcessContext::with_process(&mut process, 1);
            context
                .alloc_binary(b"this allocation forces heap growth or collection")
                .expect("binary allocation after ensure space")
        };
        assert_heap_term(&process, binary);
    }
}
