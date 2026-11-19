//! Submitting a dirty native call to its dirty scheduler pool.

use crate::atom::Atom;
use crate::native::{ExceptionClass, NativeEntry, ProcessContext};
use crate::process::Process;
use crate::scheduler::dirty::{
    DirtyJob, DirtyResult, DirtySchedulerKind, DirtySubmitError, oneshot,
};
use crate::scheduler::suspension::SuspensionResultPayload;
use crate::scheduler::{SharedState, supervision_integration, timer_integration};
use crate::term::Term;
use std::sync::Arc;

/// One admitted dirty native invocation, bundled for `submit_dirty_call`.
pub(super) struct DirtyInvocation {
    pub(super) call_id: u64,
    pub(super) entry: NativeEntry,
    pub(super) args: Vec<Term>,
    pub(super) mfa: (Atom, Atom, u8),
    pub(super) kind: DirtySchedulerKind,
}

pub(super) fn submit_dirty_call(
    shared: &Arc<SharedState>,
    process: &Process,
    invocation: DirtyInvocation,
    reservation: crate::scheduler::TeardownAdmission,
) -> Result<(), DirtySubmissionError> {
    let DirtyInvocation {
        call_id,
        entry,
        args,
        mfa,
        kind,
    } = invocation;
    if let Some(driver) = &shared.replay_driver {
        let (module, function, arity) = mfa;
        let recorded = match driver.lock() {
            Ok(mut guard) => guard.next_native_call(process.pid(), module, function, arity),
            Err(error) => {
                error
                    .into_inner()
                    .next_native_call(process.pid(), module, function, arity)
            }
        }
        .map_err(DirtySubmissionError::ReplayMismatch)?;
        let _published = shared.publish_suspension_result(
            process.pid(),
            call_id,
            SuspensionResultPayload::Dirty(Box::new(DirtyResult {
                result: recorded.outcome.result,
                exception_class: recorded.outcome.exception_class,
                exception_stacktrace: recorded.outcome.exception_stacktrace,
                owned_result: None,
                suspend: None,
                trampoline: None,
            })),
        );
        let _resumed = timer_integration::resume_suspended(shared, process.pid());
        return Ok(());
    }
    let mut context =
        ProcessContext::with_timer_services(process.pid(), Arc::clone(&shared.timers));
    let services = supervision_integration::build_native_services(shared, process.namespace_id());
    context.set_atom_table(services.atom_table);
    context.set_local_node(services.local_node);
    context.set_net_kernel(services.net_kernel);
    context.set_distribution_send_facility(services.distribution_send);
    context.set_spawn_facility(services.spawn_facility);
    context.set_remote_spawn_facility(services.remote_spawn_facility);
    context.set_link_facility(services.link_facility);
    context.set_local_send_facility(services.local_send.clone());
    context.set_group_leader_facility(services.group_leader_facility);
    context.set_supervision_facility(services.supervision_facility);
    context.set_process_info_facility(services.process_info_facility);
    context.set_code_management_facility(services.code_management_facility);
    context.set_system_info_facility(services.system_info_facility);
    context.set_replay_driver(services.replay_driver);
    context.set_nif_private_data(services.nif_private_data);
    context.set_suspension_registrar(services.suspension_registrar);
    if let Some(sink) = services.io_sink {
        context.set_io_sink(sink);
    }

    let (result_sender, result_receiver) = oneshot::channel();
    let pid = process.pid();
    let job = DirtyJob {
        pid,
        function: entry.function,
        args,
        context,
        result_sender,
    };
    // The caller registered the DirtyCall suspension mirror before this
    // submission: while no completion is published under its call id,
    // `wake_process` leaves the process parked so a mailbox arrival cannot
    // schedule a slice that re-executes the call instruction (and
    // double-submits the dirty call).
    // The DirtyCall arm already refused a Disabled target before registering
    // the suspension; this facade re-checks so the same refusal holds for any
    // future caller. A Disabled result maps to the typed service-unavailable
    // error, never the Badarg that a real submission failure yields.
    if let Err(error) = match kind {
        DirtySchedulerKind::Cpu => shared.dirty_cpu.submit(job),
        DirtySchedulerKind::Io => shared.dirty_io.submit(job),
    } {
        return Err(match error {
            DirtySubmitError::Disabled => DirtySubmissionError::Disabled(kind),
            _ => DirtySubmissionError::PoolUnavailable,
        });
    }

    let shared_for_completion = Arc::clone(shared);
    let shutdown_rx = shared.dirty_completion_shutdown_channel();
    let bridge = std::thread::Builder::new()
        .name(format!("dirty-complete-{pid}"))
        .spawn(move || {
            use crate::scheduler::dirty::oneshot::RecvOutcome;
            match result_receiver.recv_or_shutdown(&shutdown_rx) {
                RecvOutcome::Shutdown => {
                    // Scheduler teardown won the race: exit WITHOUT delivering
                    // (the pool worker's later send fails harmlessly on the
                    // dropped receiver) and release this bridge's SharedState
                    // now, not at job completion — the job may be running on
                    // an embedder-owned shared pool with no completion bound.
                }
                outcome => {
                    let result = match outcome {
                        RecvOutcome::Value(result) => result,
                        _ => DirtyResult {
                            result: Err(Term::atom(Atom::ERROR)),
                            owned_result: None,
                            exception_class: ExceptionClass::Error,
                            exception_stacktrace: Term::NIL,
                            suspend: None,
                            trampoline: None,
                        },
                    };
                    // Published under the submitting call's id: the owning
                    // thread applies it only while this exact suspension is
                    // current, and `publish_suspension_result`'s liveness
                    // double-check removes the entry if the process exited
                    // concurrently.
                    let _published = shared_for_completion.publish_suspension_result(
                        pid,
                        call_id,
                        SuspensionResultPayload::Dirty(Box::new(result)),
                    );
                    let _resumed = timer_integration::resume_suspended(&shared_for_completion, pid);
                }
            }
        });
    // Convert the admission reservation into a retained, OS-joinable handle;
    // a failed spawn drops the reservation instead (releasing the admission).
    match bridge {
        Ok(handle) => reservation.publish(handle),
        Err(_) => return Err(DirtySubmissionError::CompletionBridgeSpawn),
    }
    // Count the transient completion thread for the inventory policy line (§5).
    shared
        .dirty_completion_spawns
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum DirtySubmissionError {
    PoolUnavailable,
    /// The target dirty pool is disabled on this scheduler (spec §3.2). Kept
    /// distinct from `PoolUnavailable` so it surfaces as a typed
    /// service-unavailable error rather than `Badarg`.
    Disabled(DirtySchedulerKind),
    CompletionBridgeSpawn,
    ReplayMismatch(crate::replay::ReplayMismatch),
}

impl From<DirtySubmissionError> for crate::error::ExecError {
    fn from(error: DirtySubmissionError) -> Self {
        match error {
            DirtySubmissionError::PoolUnavailable => Self::Badarg,
            // Distinguishable service-unavailable error at the embedder API
            // (Q-B): the process still terminates with the existing
            // `ExitReason::Error` at the BEAM surface (no new BEAM-visible
            // atom), while `take_exit_error` reports the exact disabled service.
            DirtySubmissionError::Disabled(kind) => Self::ServiceUnavailable {
                service: match kind {
                    DirtySchedulerKind::Cpu => crate::scheduler::inventory::DIRTY_CPU,
                    DirtySchedulerKind::Io => crate::scheduler::inventory::DIRTY_IO,
                },
            },
            DirtySubmissionError::CompletionBridgeSpawn => Self::Badarg,
            DirtySubmissionError::ReplayMismatch(error) => Self::from(error),
        }
    }
}
