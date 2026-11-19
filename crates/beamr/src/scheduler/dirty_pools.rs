//! The dirty CPU and IO pools: building them from configuration, and
//! reaching them.

use super::dirty::DirtyPool;
use super::service::ServiceMode;
use super::{Scheduler, services};
use std::sync::Arc;

impl Scheduler {
    /// The dirty CPU pool, or `None` when this scheduler was composed with the
    /// pool disabled (spec §3.2). Replaces `dirty_cpu_pool()` per the §6
    /// keep-old-working amendment (pair-ruled): the old `&DirtyPool` signature
    /// cannot honestly represent a Disabled pool, so the break is loud and
    /// `try_`-named at the call site.
    #[doc(alias = "dirty_cpu_pool")]
    #[must_use]
    pub fn try_dirty_cpu_pool(&self) -> Option<&DirtyPool> {
        self.shared.dirty_cpu.service()
    }
    /// The dirty IO pool, or `None` when disabled (spec §3.2/§6). Replaces
    /// `dirty_io_pool()` per the same §6 amendment as
    /// [`Self::try_dirty_cpu_pool`].
    #[doc(alias = "dirty_io_pool")]
    #[must_use]
    pub fn try_dirty_io_pool(&self) -> Option<&DirtyPool> {
        self.shared.dirty_io.service()
    }
}

/// Build a dirty pool slot from a config request (spec §3.2/§6):
/// `Some(0)` → `Disabled` (no channel, no workers); `Some(n>0)` → `Owned(n)`;
/// `None` → `Owned(default_threads)`. A zero request never constructs a pool,
/// so a disabled dirty dispatch has nothing to submit into.
/// Resolve a dirty pool slot from its composition choice (spec §2.2/§3.2).
///
/// `FromConfig` defers to the legacy `dirty_*_threads` knob; `Disabled` owns
/// nothing; `Owned(n)` builds `n` workers (`n == 0` disables, matching the
/// `Some(0)` knob semantics); `Shared` injects an embedder-owned pool this
/// scheduler uses but NEVER joins (spec §2.1). The `Shared` arm is safe now
/// because dirty completion routes by the oneshot the submission carries — the
/// pool worker never consults a per-scheduler table — so one pool can back two
/// schedulers without the cross-scheduler routing the rings still lack.
pub(super) fn resolve_dirty_pool(
    name: &str,
    choice: &services::DirtyChoice,
    config_knob: Option<usize>,
    default_threads: usize,
    queue_depth: usize,
) -> ServiceMode<DirtyPool> {
    match choice {
        services::DirtyChoice::FromConfig => {
            build_dirty_pool(name, config_knob, default_threads, queue_depth)
        }
        services::DirtyChoice::Disabled => ServiceMode::Disabled,
        services::DirtyChoice::Owned(workers) => {
            build_dirty_pool(name, Some(*workers), default_threads, queue_depth)
        }
        services::DirtyChoice::Shared(pool) => ServiceMode::Shared(Arc::clone(pool)),
    }
}

fn build_dirty_pool(
    name: &str,
    requested: Option<usize>,
    default_threads: usize,
    queue_depth: usize,
) -> ServiceMode<DirtyPool> {
    match requested {
        Some(0) => ServiceMode::Disabled,
        Some(threads) => {
            ServiceMode::Owned(DirtyPool::with_queue_depth(name, threads, queue_depth))
        }
        None => ServiceMode::Owned(DirtyPool::with_queue_depth(
            name,
            default_threads,
            queue_depth,
        )),
    }
}
