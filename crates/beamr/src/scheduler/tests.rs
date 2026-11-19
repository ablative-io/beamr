use std::collections::HashMap as StdHashMap;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

#[cfg(feature = "telemetry")]
use opentelemetry::Key;
#[cfg(feature = "telemetry")]
use opentelemetry::trace::{Span, TraceContextExt, Tracer};
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ResourceMetrics};
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};

use dashmap::{DashMap, DashSet};

use super::*;
use crate::atom::{Atom, AtomTable};
use crate::distribution::{DistributionConfig, ResolveError, ResolveFuture};
use crate::ets::{EtsTableMetadata, EtsTableType, Protection};
use crate::hook::{Hook, HookDecision};
use crate::io::NullSink;
use crate::loader::Instruction;
use crate::loader::decode::compact::Operand;
use crate::mailbox::Mailbox;
use crate::module::{Module, ModuleOrigin};
use crate::namespace::NamespaceId;
use crate::native::{
    Capability, CapabilitySet, ProcessInfoItem, ProcessInfoStatus, ProcessInfoValue, SpawnFacility,
    SpawnOptions,
};
use crate::process::heap::{DEFAULT_HEAP_SIZE, Heap};
use crate::process::registry::ProcessTable;
use crate::process::{CodePosition, ExitReason, Priority, ProcessStatus};
use crate::replay::{RecordedSchedule, ReplayEvent, ReplayLog};
use crate::scheduler::execution::{
    SliceOutcome, cleanup_exited_process, cleanup_if_tombstoned_after_store, execute_slice,
    store_runnable_process, take_runnable_process,
};
use crate::supervision::link::LinkSet;
use crate::supervision::monitor::MonitorSet;
use crate::term::{Term, boxed};
use crate::timer::TimerWheel;

mod busy_poll;
mod configuration;
mod disabled_services;
mod exit_reason;
mod group_leader;
mod hooks;
mod jit;
mod park_gaps;
mod process_info;
mod registries;
mod running;
mod shutdown;
mod spawn_admission;
mod spawn_facility;
mod suspension_completion;
mod suspension_gating;
#[cfg(feature = "telemetry")]
mod telemetry_metrics;
#[cfg(feature = "telemetry")]
mod telemetry_spans;
mod timers;
mod wakeups;

pub(super) fn test_module(name: Atom, code: Vec<Instruction>) -> Module {
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            _ => None,
        })
        .collect();
    // Mirror the loader's func_info table: each `FuncInfo` opens a function
    // whose entry ip is the func_info's own position. Without it every
    // scaffold has an EMPTY function table, `Module::mfa_at_ip` returns None
    // for every ip, and MFA-derived telemetry silently reads as `Atom::NIL`.
    let function_table = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::FuncInfo {
                function: Operand::Atom(Some(function)),
                arity: Operand::Unsigned(arity),
                ..
            } => Some((ip, *function, u8::try_from(*arity).unwrap_or(u8::MAX))),
            _ => None,
        })
        .collect();
    Module {
        name,
        generation: 0,
        origin: ModuleOrigin::Preloaded,
        exports: StdHashMap::new(),
        label_index,
        code,
        literals: Vec::new(),
        constant_pool: Default::default(),
        resolved_imports: Vec::new(),
        lambdas: Vec::new(),
        string_table: Vec::new(),
        function_table,
        line_table: Vec::new(),
        line_info: Vec::new(),
    }
}

fn wait_until(deadline_ms: u64, mut predicate: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(deadline_ms);
    while !predicate() {
        assert!(std::time::Instant::now() <= deadline, "condition timed out");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

pub(super) fn receive_then_return_module(registry: &ModuleRegistry, name: Atom) -> Arc<Module> {
    registry.insert(test_module(
        name,
        vec![
            Instruction::Label { label: 1 },
            Instruction::LoopRec {
                fail: Operand::Label(2),
                destination: Operand::X(0),
            },
            Instruction::RemoveMessage,
            Instruction::Return,
            Instruction::Label { label: 2 },
            Instruction::Wait {
                fail: Operand::Label(1),
            },
        ],
    ))
}

/// Pushes an atom message exactly the way a racing sender does: under the
/// slot lock, with `wake_process` left to the caller.
pub(super) fn deliver_atom(shared: &SharedState, pid: u64, atom: Atom) {
    let entry = shared
        .process_bodies
        .get(&pid)
        .unwrap_or_else(|| panic!("process body exists"));
    let mut slot = lock_or_recover(&entry);
    match &mut *slot {
        ProcessSlot::Present(scheduled) => scheduled.0.mailbox_mut().push_owned(Term::atom(atom)),
        ProcessSlot::Executing(metadata) => metadata
            .pending_io_messages
            .push(PendingMailboxMessage::TargetOwned(Term::atom(atom))),
        ProcessSlot::Absent => panic!("process body absent"),
    }
}

/// `receive ... after 1 -> true end` in the erlc instruction shape: the
/// wait_timeout fail label is the receive loop, and timer expiry falls
/// through to `timeout` and the after-body.
pub(super) fn receive_after_module(registry: &ModuleRegistry, name: Atom) -> Arc<Module> {
    registry.insert(test_module(
        name,
        vec![
            Instruction::Label { label: 1 },
            Instruction::LoopRec {
                fail: Operand::Label(2),
                destination: Operand::X(0),
            },
            Instruction::RemoveMessage,
            Instruction::Return,
            Instruction::Label { label: 2 },
            Instruction::WaitTimeout {
                fail: Operand::Label(1),
                timeout: Operand::Unsigned(1),
            },
            Instruction::Timeout,
            Instruction::Move {
                source: Operand::Atom(Some(Atom::TRUE)),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
    ))
}

fn native_call_module(
    registry: &ModuleRegistry,
    name: Atom,
    function: crate::native::NativeFn,
    dirty_kind: Option<crate::scheduler::DirtySchedulerKind>,
) -> Arc<Module> {
    let mut module = test_module(
        name,
        vec![
            Instruction::CallExt {
                arity: Operand::Unsigned(0),
                import: Operand::Unsigned(0),
            },
            Instruction::Return,
        ],
    );
    module.resolved_imports.push(crate::module::ResolvedImport {
        module: name,
        function: name,
        arity: 0,
        target: crate::module::ResolvedImportTarget::Native(crate::native::NativeEntry {
            function,
            dirty_kind,
            capability: Capability::Pure,
        }),
    });
    registry.insert(module)
}

fn single_thread_scheduler(registry: &Arc<ModuleRegistry>) -> Scheduler {
    Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            dirty_cpu_threads: Some(1),
            dirty_io_threads: Some(1),
            dirty_queue_depth: Some(8),
            ..SchedulerConfig::default()
        },
        Arc::clone(registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"))
}

fn exit_value(scheduler: &Scheduler, pid: u64) -> Option<Term> {
    scheduler
        .shared
        .exit_results
        .get(&pid)
        .map(|result| result.root())
}

struct BusyPollHandler {
    received: Arc<Mutex<Vec<Term>>>,
    slices: Arc<AtomicUsize>,
}

impl crate::native::native_process::NativeHandler for BusyPollHandler {
    fn handle(
        &mut self,
        ctx: &mut crate::native::native_process::NativeContext<'_>,
    ) -> crate::native::native_process::NativeOutcome {
        self.slices.fetch_add(1, Ordering::AcqRel);
        while let Some(message) = ctx.recv() {
            lock_or_recover(&self.received).push(message);
        }
        crate::native::native_process::NativeOutcome::Continue
    }
}
