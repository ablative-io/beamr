//! Composable ancillary-service ownership primitives (spec §2).
//!
//! Every VM ancillary service (dirty pools, IO rings, distribution runtimes,
//! and the born-composed readiness service) is meant to be held in a
//! [`ServiceMode`]: constructed and joined by one scheduler, injected and
//! shared across several, or refused outright. Commit 1 introduces the
//! primitives and the [inventory](super::inventory) that reports them; the
//! existing eager construction paths are reported as [`ServiceMode::Owned`]
//! and are wired into these wrappers by the later commits (spec §11).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-unique identity of an underlying service instance.
///
/// Minted once at service construction (Owned and Shared alike) and carried by
/// the service, so cloning a [`ServiceMode::Shared`] handle into a second
/// scheduler yields the SAME id there: two inventory entries with equal ids ARE
/// the same instance, which makes the process-wide dedup (spec §5/§9 Q2) a
/// plain group-by rather than a heuristic.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct ServiceInstanceId(u64);

/// Monotonic source of [`ServiceInstanceId`] values. Starts at 1 so 0 is a
/// reserved sentinel for services that were never constructed.
static NEXT_SERVICE_INSTANCE_ID: AtomicU64 = AtomicU64::new(1);

impl ServiceInstanceId {
    /// Identity used for a [`ServiceMode::Disabled`] slot — a service that was
    /// never constructed and so has no instance. Never equal to a minted id.
    pub const DISABLED: Self = Self(0);

    /// Mint a fresh process-unique identity for a newly constructed service.
    #[must_use]
    pub fn mint() -> Self {
        Self(NEXT_SERVICE_INSTANCE_ID.fetch_add(1, Ordering::Relaxed))
    }

    /// Raw token value, exposed for stable ordering and debug output only.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

/// A service that can report its process-wide instance identity.
///
/// Implemented by every service carried in a [`ServiceMode`], so the mode can
/// surface the id regardless of whether the service is Owned or Shared. Sharing
/// an `Arc<T>` shares the one `T`, hence the one id — that is the propagation
/// mechanism the Q2 dedup relies on.
pub trait ServiceIdentity {
    /// The instance identity minted for this service at construction.
    fn instance_id(&self) -> ServiceInstanceId;
}

/// A service whose owner tears it down deterministically (stop + join).
///
/// The single teardown primitive [`ServiceMode::shutdown_if_owned`] drives
/// this; only the owning scheduler ever calls it.
pub trait ShutdownService {
    /// Stop accepting work and join every OS thread the service owns.
    fn shutdown(&self);
}

/// Coarse mode label reported by the inventory, independent of the service type.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum ServiceModeLabel {
    /// Zero threads, zero fds; use is refused explicitly.
    Disabled,
    /// Constructed by, inventoried under, and joined by this scheduler.
    Owned,
    /// Injected; used and inventoried here, never shut down here.
    Shared,
}

/// Ownership wrapper carried by every ancillary VM service (spec §2.1).
pub enum ServiceMode<T> {
    /// Zero threads, zero fds; use is refused explicitly.
    Disabled,
    /// Constructed by, inventoried under, and joined by this scheduler.
    Owned(T),
    /// Injected; used and inventoried as shared, NEVER shut down here.
    Shared(Arc<T>),
}

impl<T> ServiceMode<T> {
    /// The coarse mode label for this slot.
    #[must_use]
    pub fn label(&self) -> ServiceModeLabel {
        match self {
            ServiceMode::Disabled => ServiceModeLabel::Disabled,
            ServiceMode::Owned(_) => ServiceModeLabel::Owned,
            ServiceMode::Shared(_) => ServiceModeLabel::Shared,
        }
    }

    /// Borrow the underlying service, whether Owned or Shared. `None` when
    /// Disabled — the caller must refuse before any queue/suspension side
    /// effect (spec §3.2).
    #[must_use]
    pub fn service(&self) -> Option<&T> {
        match self {
            ServiceMode::Disabled => None,
            ServiceMode::Owned(service) => Some(service),
            ServiceMode::Shared(service) => Some(service),
        }
    }
}

impl<T: ServiceIdentity> ServiceMode<T> {
    /// The instance identity of the underlying service, or
    /// [`ServiceInstanceId::DISABLED`] when this slot holds no service.
    #[must_use]
    pub fn instance_id(&self) -> ServiceInstanceId {
        match self {
            ServiceMode::Disabled => ServiceInstanceId::DISABLED,
            ServiceMode::Owned(service) => service.instance_id(),
            ServiceMode::Shared(service) => service.instance_id(),
        }
    }
}

impl<T: ShutdownService> ServiceMode<T> {
    /// The single teardown primitive (spec §2.1). Empties the slot in every
    /// arm — teardown is not observable-later state:
    /// - `Owned`: stop and join the service, exactly once.
    /// - `Shared`: RELEASE the reference (the `Arc` is dropped here, so the
    ///   owner's later teardown is not held open by this scheduler), never
    ///   stop the service.
    /// - `Disabled`: no-op.
    pub fn shutdown_if_owned(&mut self) {
        match std::mem::replace(self, ServiceMode::Disabled) {
            ServiceMode::Owned(service) => service.shutdown(),
            ServiceMode::Shared(reference) => drop(reference),
            ServiceMode::Disabled => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ServiceIdentity, ServiceInstanceId, ServiceMode, ServiceModeLabel, ShutdownService,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestService {
        id: ServiceInstanceId,
        // Held through an external Arc so shutdown is observable after the
        // slot consumes and drops the service.
        shutdowns: Arc<AtomicUsize>,
    }

    impl TestService {
        fn new() -> (Self, Arc<AtomicUsize>) {
            let shutdowns = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    id: ServiceInstanceId::mint(),
                    shutdowns: Arc::clone(&shutdowns),
                },
                shutdowns,
            )
        }
    }

    impl ServiceIdentity for TestService {
        fn instance_id(&self) -> ServiceInstanceId {
            self.id
        }
    }

    impl ShutdownService for TestService {
        fn shutdown(&self) {
            self.shutdowns.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[test]
    fn minted_ids_are_process_unique_and_distinct_from_disabled() {
        let first = ServiceInstanceId::mint();
        let second = ServiceInstanceId::mint();
        assert_ne!(first, second);
        assert_ne!(first, ServiceInstanceId::DISABLED);
        assert_ne!(second, ServiceInstanceId::DISABLED);
    }

    #[test]
    fn shared_handle_clone_propagates_one_id_across_two_schedulers() {
        // One underlying service handed to two schedulers via a cloned Arc:
        // both report the SAME instance id, so the Q2 dedup collapses them.
        let service = Arc::new(TestService::new().0);
        let on_scheduler_a = ServiceMode::Shared(Arc::clone(&service));
        let on_scheduler_b = ServiceMode::Shared(Arc::clone(&service));
        assert_eq!(on_scheduler_a.instance_id(), on_scheduler_b.instance_id());
        assert_eq!(on_scheduler_a.label(), ServiceModeLabel::Shared);

        // Two independently constructed services are distinct instances.
        let owned_a = ServiceMode::Owned(TestService::new().0);
        let owned_b = ServiceMode::Owned(TestService::new().0);
        assert_ne!(owned_a.instance_id(), owned_b.instance_id());
    }

    #[test]
    fn disabled_slot_reports_disabled_identity_and_no_service() {
        let disabled: ServiceMode<TestService> = ServiceMode::Disabled;
        assert_eq!(disabled.instance_id(), ServiceInstanceId::DISABLED);
        assert_eq!(disabled.label(), ServiceModeLabel::Disabled);
        assert!(disabled.service().is_none());
    }

    #[test]
    fn shutdown_if_owned_stops_owned_exactly_once_and_empties_the_slot() {
        let (service, shutdowns) = TestService::new();
        let mut owned = ServiceMode::Owned(service);
        owned.shutdown_if_owned();
        assert_eq!(shutdowns.load(Ordering::Acquire), 1, "stopped once");
        assert_eq!(owned.label(), ServiceModeLabel::Disabled, "slot emptied");
        assert!(owned.service().is_none());

        // A second call is a no-op — the service cannot be stopped twice.
        owned.shutdown_if_owned();
        assert_eq!(shutdowns.load(Ordering::Acquire), 1);
    }

    #[test]
    fn shutdown_if_owned_releases_shared_without_stopping_it() {
        let (service, shutdowns) = TestService::new();
        let backing = Arc::new(service);
        let mut shared = ServiceMode::Shared(Arc::clone(&backing));
        assert_eq!(Arc::strong_count(&backing), 2);

        shared.shutdown_if_owned();
        // The reference is RELEASED — the Arc dropped, the service untouched.
        assert_eq!(Arc::strong_count(&backing), 1, "shared handle released");
        assert_eq!(shutdowns.load(Ordering::Acquire), 0, "never stopped here");
        assert_eq!(shared.label(), ServiceModeLabel::Disabled, "slot emptied");

        // Disabled is a no-op.
        let mut disabled: ServiceMode<TestService> = ServiceMode::Disabled;
        disabled.shutdown_if_owned();
        assert_eq!(disabled.label(), ServiceModeLabel::Disabled);
    }
}
