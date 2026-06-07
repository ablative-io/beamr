//! Minimal process-facing context exposed to native code.
//!
//! Native functions deliberately receive this allocation subset instead of the
//! full process so they cannot inspect scheduler, mailbox, or process internals.

use std::fmt;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::atom::{Atom, AtomTable};
use crate::io::{IoSink, NullSink};
use crate::native::stdlib_stubs::{lists_bifs::ListsMapState, maps_bifs::MapsHofState};
use crate::process::Process;
use crate::term::Term;
use crate::term::binary::packed_word_count;
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
    process: Option<NonNull<Process>>,
    live_x: usize,
}

impl fmt::Debug for ProcessContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessContext")
            .field("pid", &self.pid)
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
            .field("process_heap", &self.process.as_ref().map(|_| ".."))
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
            process: None,
            live_x: 256,
        }
    }

    /// Creates a context with timer services for asynchronous timer BIFs.
    #[must_use]
    pub fn with_timer_services(pid: u64, timers: Arc<Mutex<TimerWheel>>) -> Self {
        Self {
            pid: Some(pid),
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
            process: None,
            live_x: 256,
        }
    }

    /// Creates a context backed by the calling process heap.
    #[must_use]
    pub fn with_process(process: &mut Process, live_x: usize) -> Self {
        let mut context = Self::new();
        context.set_pid(Some(process.pid()));
        context.attach_process_heap(process, live_x);
        context
    }

    /// Attach this context to the calling process heap for native result allocation.
    pub fn attach_process_heap(&mut self, process: &mut Process, live_x: usize) {
        self.process = Some(NonNull::from(process));
        self.live_x = live_x;
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

    /// Reserve a timer reference and schedule a `{timeout, Ref, Message}` tuple.
    pub fn schedule_timeout_timer(
        &mut self,
        delay: Duration,
        target_pid: u64,
        message: Term,
    ) -> Option<Result<TimerRef, Term>> {
        let timers = Arc::clone(self.timers.as_ref()?);
        let reference = timers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .reserve_reference();
        let reference_term = i64::try_from(reference.id())
            .ok()
            .and_then(Term::try_small_int)
            .ok_or_else(|| Term::atom(Atom::BADARG));
        let timeout_message = reference_term.and_then(|reference| {
            self.alloc_tuple(&[Term::atom(Atom::TIMEOUT), reference, message])
        });
        match timeout_message {
            Ok(timeout_message) => {
                timers
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .schedule_reserved(reference, delay, target_pid, timeout_message);
                Some(Ok(reference))
            }
            Err(error) => Some(Err(error)),
        }
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

    /// Ensure that at least `words` words are available in the process heap.
    pub fn ensure_heap_space(&mut self, words: usize) -> Result<(), Term> {
        let live_x = self.live_x;
        let process = self.process_mut()?;
        crate::gc::ensure_space(process, words, live_x).map_err(gc_error_to_term)
    }

    fn process_mut(&mut self) -> Result<&mut Process, Term> {
        let mut process = self.process.ok_or_else(|| Term::atom(Atom::BADARG))?;
        // SAFETY: interpreter-created heap-backed contexts are used only for the
        // duration of one native call while the caller holds exclusive access to
        // the process. Native code cannot copy the private raw handle out of the
        // context, and all process mutations after the call occur after context use.
        Ok(unsafe { process.as_mut() })
    }

    fn alloc_words(&mut self, words: usize) -> Result<&mut [u64], Term> {
        self.ensure_heap_space(words)?;
        let ptr = self
            .process_mut()?
            .heap_mut()
            .alloc(words)
            .map_err(|_| Term::atom(Atom::ERROR))?;
        Ok(heap_slice(ptr, words))
    }

    /// Allocate a tuple on the calling process heap.
    pub fn alloc_tuple(&mut self, elements: &[Term]) -> Result<Term, Term> {
        let words = 1 + elements.len();
        let heap = self.alloc_words(words)?;
        crate::term::boxed::write_tuple(heap, elements).ok_or_else(|| Term::atom(Atom::BADARG))
    }

    /// Allocate a cons cell on the calling process heap.
    pub fn alloc_cons(&mut self, head: Term, tail: Term) -> Result<Term, Term> {
        let heap = self.alloc_words(2)?;
        crate::term::boxed::write_cons(heap, head, tail).ok_or_else(|| Term::atom(Atom::BADARG))
    }

    /// Allocate a float on the calling process heap.
    pub fn alloc_float(&mut self, value: f64) -> Result<Term, Term> {
        let heap = self.alloc_words(2)?;
        crate::term::boxed::write_float(heap, value).ok_or_else(|| Term::atom(Atom::BADARG))
    }

    /// Allocate an inline binary on the calling process heap.
    pub fn alloc_binary(&mut self, bytes: &[u8]) -> Result<Term, Term> {
        let words = 2 + packed_word_count(bytes.len());
        let heap = self.alloc_words(words)?;
        crate::term::binary::write_binary(heap, bytes).ok_or_else(|| Term::atom(Atom::BADARG))
    }

    /// Allocate a big integer on the calling process heap.
    pub fn alloc_bigint(&mut self, negative: bool, limbs: &[u64]) -> Result<Term, Term> {
        let words = 3 + limbs.len();
        let heap = self.alloc_words(words)?;
        crate::term::boxed::write_bigint(heap, negative, limbs)
            .ok_or_else(|| Term::atom(Atom::BADARG))
    }

    /// Allocate a proper list on the calling process heap.
    pub fn alloc_list(&mut self, elements: &[Term]) -> Result<Term, Term> {
        self.alloc_list_with_tail(elements, Term::NIL)
    }

    /// Allocate a list ending in `tail` on the calling process heap.
    pub fn alloc_list_with_tail(&mut self, elements: &[Term], tail: Term) -> Result<Term, Term> {
        if elements.is_empty() {
            return Ok(tail);
        }
        let words = elements
            .len()
            .checked_mul(2)
            .ok_or_else(|| Term::atom(Atom::BADARG))?;
        self.ensure_heap_space(words)?;
        let mut result = tail;
        for element in elements.iter().rev().copied() {
            let ptr = self
                .process_mut()?
                .heap_mut()
                .alloc(2)
                .map_err(|_| Term::atom(Atom::ERROR))?;
            let heap = heap_slice(ptr, 2);
            result = crate::term::boxed::write_cons(heap, element, result)
                .ok_or_else(|| Term::atom(Atom::BADARG))?;
        }
        Ok(result)
    }

    /// Allocate a flatmap on the calling process heap.
    pub fn alloc_map(&mut self, keys: &[Term], values: &[Term]) -> Result<Term, Term> {
        let words = 2 + keys.len() + values.len();
        let heap = self.alloc_words(words)?;
        crate::term::boxed::write_map(heap, keys, values).ok_or_else(|| Term::atom(Atom::BADARG))
    }

    /// Allocate an inline binary and a tuple wrapping it in one no-GC batch.
    pub fn alloc_binary_tuple(&mut self, tag: Atom, bytes: &[u8]) -> Result<Term, Term> {
        let binary_words = 2 + packed_word_count(bytes.len());
        let total_words = binary_words
            .checked_add(3)
            .ok_or_else(|| Term::atom(Atom::BADARG))?;
        self.ensure_heap_space(total_words)?;
        let binary_ptr = self
            .process_mut()?
            .heap_mut()
            .alloc(binary_words)
            .map_err(|_| Term::atom(Atom::ERROR))?;
        let binary_heap = heap_slice(binary_ptr, binary_words);
        let binary = crate::term::binary::write_binary(binary_heap, bytes)
            .ok_or_else(|| Term::atom(Atom::BADARG))?;
        let tuple_ptr = self
            .process_mut()?
            .heap_mut()
            .alloc(3)
            .map_err(|_| Term::atom(Atom::ERROR))?;
        let tuple_heap = heap_slice(tuple_ptr, 3);
        crate::term::boxed::write_tuple(tuple_heap, &[Term::atom(tag), binary])
            .ok_or_else(|| Term::atom(Atom::BADARG))
    }
}

fn gc_error_to_term(error: crate::gc::GcError) -> Term {
    match error {
        crate::gc::GcError::HeapFull(_) => Term::atom(Atom::ERROR),
        crate::gc::GcError::InvalidObjectHeader(_) => Term::atom(Atom::BADARG),
    }
}

fn heap_slice<'a>(ptr: *mut u64, words: usize) -> &'a mut [u64] {
    // SAFETY: `Heap::alloc(words)` returned a non-overlapping allocation with
    // exactly `words` contiguous machine words that remain owned by the process
    // heap. The slice is used immediately to initialise the new object.
    unsafe { std::slice::from_raw_parts_mut(ptr, words) }
}

#[cfg(test)]
mod tests {
    use crate::process::Process;
    use crate::term::{
        Term,
        binary::Binary,
        boxed::{Float, Map, Tuple},
    };

    use super::ProcessContext;

    fn heap_contains(process: &Process, term: Term) -> bool {
        term.heap_ptr()
            .is_some_and(|ptr| process.heap().contains(ptr))
    }

    #[test]
    fn allocation_helpers_write_terms_to_process_heap() {
        let mut process = Process::new(0, 64);
        let (tuple, float, binary, list, map) = {
            let mut context = ProcessContext::with_process(&mut process, 0);
            let tuple = context
                .alloc_tuple(&[Term::small_int(1), Term::atom(crate::atom::Atom::OK)])
                .expect("tuple allocation");
            let float = context.alloc_float(1.5).expect("float allocation");
            let binary = context.alloc_binary(b"abc").expect("binary allocation");
            let list = context
                .alloc_list(&[Term::small_int(1), Term::small_int(2)])
                .expect("list allocation");
            let map = context
                .alloc_map(&[Term::atom(crate::atom::Atom::OK)], &[binary])
                .expect("map allocation");
            (tuple, float, binary, list, map)
        };

        assert!(heap_contains(&process, tuple));
        assert!(heap_contains(&process, float));
        assert!(heap_contains(&process, binary));
        assert!(heap_contains(&process, list));
        assert!(heap_contains(&process, map));
        assert_eq!(Tuple::new(tuple).expect("tuple").arity(), 2);
        assert_eq!(Float::new(float).expect("float").value(), 1.5);
        assert_eq!(Binary::new(binary).expect("binary").as_bytes(), b"abc");
        assert_eq!(Map::new(map).expect("map").len(), 1);
    }

    #[test]
    fn ensure_heap_space_allows_gc_to_reclaim_unreachable_native_results() {
        let mut process = Process::new(0, 8);
        {
            let mut context = ProcessContext::with_process(&mut process, 0);
            for _ in 0..16 {
                let _ = context.alloc_binary(b"abcd").expect("binary allocation");
            }
        }
        assert!(process.heap().young_used() <= process.heap().young_capacity());
    }
}
