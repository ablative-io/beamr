//! The `NativeServices` bundle built for each time slice, and the small
//! facilities that live only inside it.

use crate::distribution::control::DistributionSendFacility;
use crate::distribution::remote_link::DistributionControlFacility;
use crate::namespace::NamespaceId;
use crate::native::FileIoFacility;
use crate::native::spawn::SpawnFacility;
use crate::scheduler::{SharedState, lock_or_recover};
use std::sync::Arc;

use super::distribution_send::SchedulerDistributionSendFacility;
use super::ets_facility::SchedulerEtsFacility;
use super::group_leader::SchedulerGroupLeaderFacility;
use super::io_facilities::{
    SchedulerFileIoFacility, SchedulerIoMessageFacility, SchedulerTcpIoFacility,
};
use super::links::SchedulerLinkFacility;
use super::local_send::SchedulerLocalSendFacility;
use super::remote_links::SchedulerDistributionControlFacility;
use super::spawn::SchedulerSpawnFacility;
use super::supervision_facility::SchedulerSupervisionFacility;

/// Build the `NativeServices` bundle for a scheduler time slice.
pub(in crate::scheduler) fn build_native_services(
    shared: &Arc<SharedState>,
    namespace_id: NamespaceId,
) -> crate::interpreter::NativeServices {
    let spawn: Arc<dyn SpawnFacility> = Arc::new(SchedulerSpawnFacility {
        shared: Arc::clone(shared),
        namespace_id,
    });
    let link: Arc<dyn crate::native::links::LinkFacility> = Arc::new(SchedulerLinkFacility {
        shared: Arc::clone(shared),
    });
    let group_leader: Arc<dyn crate::native::GroupLeaderFacility> =
        Arc::new(SchedulerGroupLeaderFacility {
            shared: Arc::clone(shared),
        });
    let supervision: Arc<dyn crate::native::supervision::SupervisionFacility> =
        Arc::new(SchedulerSupervisionFacility {
            shared: Arc::clone(shared),
        });
    let process_info: Arc<dyn crate::native::ProcessInfoFacility> =
        Arc::new(SchedulerProcessInfoFacility {
            shared: Arc::clone(shared),
        });
    let code_management: Arc<dyn crate::native::CodeManagementFacility> = Arc::new(
        crate::scheduler::module_management::SchedulerCodeManagementFacility {
            shared: Arc::clone(shared),
        },
    );
    let system_info: Arc<dyn crate::native::SystemInfoFacility> =
        Arc::new(SchedulerSystemInfoFacility {
            shared: Arc::clone(shared),
        });
    let ets_facility: Arc<dyn crate::native::EtsFacility> = Arc::new(SchedulerEtsFacility {
        shared: Arc::clone(shared),
    });
    let pg_facility: Arc<dyn crate::distribution::pg::PgFacility> =
        Arc::clone(&shared.pg_registry) as _;
    // The file facility exists only when the file ring is present (spec §3.3).
    // A Disabled file ring yields no facility, so `submit_file_io` refuses at
    // the BIF surface (badarg) BEFORE any suspension is registered — the
    // refusal-first discipline, held here by simply not offering the facility.
    // §3.3's embedder-typed refusal (Q-B's ServiceUnavailable half) lands with
    // live-Disabled construction in commit 5: the only Disabled file ring
    // today is replay, which never runs file natives live.
    let file_io_facility: Option<Arc<dyn FileIoFacility>> =
        shared.file_io_ring.service().map(|ring| {
            Arc::new(SchedulerFileIoFacility {
                shared: Arc::clone(shared),
                ring: Arc::clone(ring.ring()),
            }) as Arc<dyn FileIoFacility>
        });
    // Distribution-facing facilities are offered ONLY when the bundle is Owned
    // (spec §3.6): with distribution Disabled the net-kernel, remote-send, and
    // control surfaces are absent, so the BIFs take their standalone-node arms
    // (node ⇒ nonode@nohost, nodes ⇒ [], connect/disconnect ⇒ false, remote
    // send ⇒ noconnection) instead of touching an absent runtime.
    let distribution_owned = shared.distribution().is_some();
    let net_kernel = shared
        .distribution()
        .map(|dist| Arc::clone(dist.net_kernel()));
    let distribution_send: Option<Arc<dyn DistributionSendFacility>> =
        distribution_owned.then(|| {
            Arc::new(SchedulerDistributionSendFacility {
                shared: Arc::clone(shared),
            }) as Arc<dyn DistributionSendFacility>
        });
    let distribution_control_facility: Option<Arc<dyn DistributionControlFacility>> =
        distribution_owned.then(|| {
            Arc::new(SchedulerDistributionControlFacility {
                shared: Arc::clone(shared),
            }) as Arc<dyn DistributionControlFacility>
        });
    let local_send: Arc<dyn crate::native::local_send::LocalSendFacility> =
        Arc::new(SchedulerLocalSendFacility {
            shared: Arc::clone(shared),
        });
    crate::interpreter::NativeServices {
        atom_table: Some(Arc::clone(&shared.atom_table)),
        local_node: Some(shared.local_node),
        net_kernel,
        distribution_send,
        local_send: Some(local_send),
        ets_facility: Some(ets_facility),
        pg_facility: Some(pg_facility),
        timers: Some(Arc::clone(&shared.timers)),
        spawn_facility: Some(spawn),
        link_facility: Some(link),
        distribution_control_facility,
        group_leader_facility: Some(group_leader),
        supervision_facility: Some(supervision),
        process_info_facility: Some(process_info),
        io_sink: Some(Arc::clone(&lock_or_recover(&shared.output_sink))),
        code_management_facility: Some(code_management),
        system_info_facility: Some(system_info),
        io_facility: if shared.replay_mode {
            None
        } else {
            shared.io_facility.clone()
        },
        io_message_facility: Some(Arc::new(SchedulerIoMessageFacility {
            shared: Arc::clone(shared),
        })),
        teardown_admission_facility: Some(Arc::new(SchedulerTeardownAdmissionFacility {
            shared: Arc::clone(shared),
        })),
        #[cfg(feature = "readiness")]
        readiness_facility: shared.readiness_consumer.as_ref().map(|_| {
            Arc::new(SchedulerReadinessFacility {
                shared: Arc::clone(shared),
            }) as Arc<dyn crate::native::ReadinessFacility>
        }),
        // Both surfaces ride the file ring, so both are absent when it is
        // Disabled (spec §3.3) — `file_io_facility` is already gated on the
        // ring above; the TCP surface follows the same condition.
        file_io_facility,
        tcp_io_facility: shared.file_io_ring.service().is_some().then(|| {
            Arc::new(SchedulerTcpIoFacility {
                shared: Arc::clone(shared),
            }) as Arc<dyn crate::native::TcpIoFacility>
        }),
        // Withheld when the operator switch is off, which is what stops
        // ALREADY-COMPILED code being entered: the cache itself survives
        // untouched and goes live again if the switch is turned back on.
        jit_cache: shared.jit_enabled().then(|| Arc::clone(&shared.jit_cache)),
        // The profiling handle composes AWAY under replay (reduction counts
        // are validated there and JIT charges differ, commit-5 precedent),
        // wherever the dirty-CPU service is Disabled, and when the operator
        // switch is off: absence at the call edges is the disable mechanism,
        // never a runtime flag check AT the edges. The switch is read once
        // here, per slice, and turns into that same absence — it does not add
        // a flag test to any call edge.
        jit_profiling: (!shared.replay_mode
            && shared.dirty_cpu.service().is_some()
            && shared.jit_enabled())
        .then(|| {
            Arc::new(crate::jit::JitProfilingServices {
                profiler: Arc::clone(&shared.jit_profiler),
                submitter: Arc::new(SchedulerJitSubmissionFacility {
                    shared: Arc::clone(shared),
                }),
            })
        }),
        replay_driver: shared.replay_driver.clone(),
        bif_registry: Some(Arc::clone(&shared.bif_registry)),
        nif_private_data: shared.nif_private_data.clone(),
        suspension_registrar: Some(Arc::new(
            crate::scheduler::suspension::SchedulerSuspensionRegistrar {
                shared: Arc::clone(shared),
            },
        )),
        ..crate::interpreter::NativeServices::default()
    }
}

// ── Facility implementations ────────────────────────────────────────────────

struct SchedulerJitSubmissionFacility {
    shared: Arc<SharedState>,
}

impl crate::jit::JitSubmissionFacility for SchedulerJitSubmissionFacility {
    fn submit(
        &self,
        request: crate::jit::CompilationRequest,
    ) -> Result<(), crate::scheduler::dirty::DirtySubmitError> {
        let job = crate::jit::CompilationJob::new(
            request,
            Arc::clone(&self.shared.jit_compiler),
            Arc::clone(&self.shared.jit_profiler),
            Arc::clone(&self.shared.jit_cache),
        );
        crate::jit::try_submit_jit_compilation(&self.shared.dirty_cpu, job)
    }
}

struct SchedulerTeardownAdmissionFacility {
    shared: Arc<SharedState>,
}

impl crate::native::TeardownAdmissionFacility for SchedulerTeardownAdmissionFacility {
    fn try_reserve(&self) -> Option<Box<dyn Send>> {
        self.shared
            .try_reserve_teardown_admission()
            .map(|admission| Box::new(admission) as Box<dyn Send>)
    }
}

#[cfg(feature = "readiness")]
struct SchedulerReadinessFacility {
    shared: Arc<SharedState>,
}

#[cfg(feature = "readiness")]
impl crate::native::ReadinessFacility for SchedulerReadinessFacility {
    fn register(
        &self,
        fd: std::os::fd::RawFd,
        interest: crate::scheduler::Interest,
        pid: u64,
        marker: crate::atom::Atom,
    ) -> Result<crate::scheduler::ReadinessToken, crate::scheduler::ReadinessError> {
        self.shared.readiness_register(fd, interest, pid, marker)
    }

    fn rearm(
        &self,
        token: &crate::scheduler::ReadinessToken,
        interest: crate::scheduler::Interest,
    ) -> Result<(), crate::scheduler::ReadinessError> {
        self.shared.readiness_rearm(token, interest)
    }
}

/// Real `ProcessInfoFacility` backed by the scheduler's shared state.
pub(in crate::scheduler) struct SchedulerProcessInfoFacility {
    pub(in crate::scheduler) shared: Arc<SharedState>,
}

impl crate::native::ProcessInfoFacility for SchedulerProcessInfoFacility {
    fn process_info(
        &self,
        pid: u64,
        item: crate::native::ProcessInfoItem,
    ) -> Option<crate::native::ProcessInfoValue> {
        self.shared.process_info(pid, item)
    }
}

pub(in crate::scheduler) struct SchedulerSystemInfoFacility {
    pub(in crate::scheduler) shared: Arc<SharedState>,
}

impl crate::native::SystemInfoFacility for SchedulerSystemInfoFacility {
    fn scheduler_count(&self) -> usize {
        self.shared.scheduler_count()
    }

    fn process_count(&self) -> usize {
        self.shared.process_count()
    }

    fn atom_count(&self) -> usize {
        self.shared.atom_count()
    }

    fn atom_limit(&self) -> usize {
        self.shared.atom_table.limit()
    }

    /// Stated explicitly rather than inherited: `SharedState` carries no
    /// cumulative reduction counter and no VM start-time accumulator. The
    /// per-slice reduction budget is consumed and reset, never accumulated,
    /// and `record_process_slice_metrics` (`scheduler/mod.rs`) is
    /// feature-gated, sampled per-pid and write-only into the telemetry sink,
    /// so it cannot back a totals BIF. When totals tracking lands, this is the
    /// site that builds `Some`.
    fn statistics_summary(&self) -> Option<crate::native::system_info_bifs::StatisticsSummary> {
        None
    }

    fn memory_summary(&self) -> crate::native::system_info_bifs::MemorySummary {
        self.shared.memory_summary()
    }
}
