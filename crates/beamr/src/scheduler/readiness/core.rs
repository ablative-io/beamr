mod delivery;

use super::types::{
    Generation, Interest, ReadinessBuildError, ReadinessError, ReadinessToken, errno,
};
use crate::atom::Atom;
use crate::scheduler::SharedState;
use mio::unix::SourceFd;
use mio::{Events, Poll, Registry, Waker};
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::JoinHandle;

pub(super) const READINESS_POLL_THREAD_PREFIX: &str = "beamr-readiness";
const WAKER_TOKEN: mio::Token = mio::Token(usize::MAX);

static NEXT_CONSUMER_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(in crate::scheduler) struct ServiceConsumerId(u64);

impl ServiceConsumerId {
    pub(in crate::scheduler) fn mint() -> Self {
        Self(NEXT_CONSUMER_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Identity copied into every registration so a shared poller routes home.
#[derive(Clone)]
pub(in crate::scheduler) struct RouteHome {
    pub(in crate::scheduler) scheduler: Weak<SharedState>,
    pub(in crate::scheduler) consumer: ServiceConsumerId,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum RecordState {
    Live,
    Draining,
}

struct Registration {
    fd: RawFd,
    generation: Generation,
    pid: u64,
    marker: Atom,
    armed: Interest,
    route: RouteHome,
    state: RecordState,
}

#[derive(Default)]
struct Slot {
    generation: u64,
    record: Option<Registration>,
}

#[derive(Default)]
struct RegistrationTable {
    slots: Vec<Slot>,
}

impl RegistrationTable {
    fn vacant_slot(&mut self) -> usize {
        if let Some(index) = self.slots.iter().position(|slot| slot.record.is_none()) {
            index
        } else {
            self.slots.push(Slot::default());
            self.slots.len() - 1
        }
    }
}

/// The one poller, cloned registry, waker, and registration table.
pub(super) struct ReadinessCore {
    poll: Mutex<Poll>,
    registry: Registry,
    waker: Waker,
    table: Mutex<RegistrationTable>,
    poll_epoch: AtomicU64,
    epoch_lock: Mutex<()>,
    epoch_changed: Condvar,
    stopping: AtomicBool,
    failed: AtomicBool,
    thread: Mutex<Option<JoinHandle<()>>>,
    initial_route: Mutex<Option<RouteHome>>,
    #[cfg(test)]
    panic_in_delivery: AtomicBool,
}

impl ReadinessCore {
    pub(super) fn build(
        initial_route: Option<RouteHome>,
    ) -> Result<Arc<Self>, ReadinessBuildError> {
        let poll = Poll::new().map_err(|error| ReadinessBuildError::PollSetUnavailable {
            errno: errno(&error),
        })?;
        let registry = poll.registry().try_clone().map_err(|error| {
            ReadinessBuildError::PollSetUnavailable {
                errno: errno(&error),
            }
        })?;
        let waker = Waker::new(poll.registry(), WAKER_TOKEN).map_err(|error| {
            ReadinessBuildError::PollSetUnavailable {
                errno: errno(&error),
            }
        })?;
        let core = Arc::new(Self {
            poll: Mutex::new(poll),
            registry,
            waker,
            table: Mutex::new(RegistrationTable::default()),
            poll_epoch: AtomicU64::new(0),
            epoch_lock: Mutex::new(()),
            epoch_changed: Condvar::new(),
            stopping: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            thread: Mutex::new(None),
            initial_route: Mutex::new(initial_route),
            #[cfg(test)]
            panic_in_delivery: AtomicBool::new(false),
        });
        let thread_core = Arc::clone(&core);
        let handle = std::thread::Builder::new()
            .name(format!("{READINESS_POLL_THREAD_PREFIX}-poll"))
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    thread_core.poll_loop()
                }));
                if !thread_core.stopping.load(Ordering::Acquire)
                    && (outcome.is_err() || outcome.is_ok_and(|result| result.is_err()))
                {
                    thread_core.failed.store(true, Ordering::Release);
                    thread_core.epoch_changed.notify_all();
                }
            })
            .map_err(|error| ReadinessBuildError::PollSetUnavailable {
                errno: errno(&error),
            })?;
        match core.thread.lock() {
            Ok(mut slot) => *slot = Some(handle),
            Err(poisoned) => {
                core.failed.store(true, Ordering::Release);
                let mut slot = poisoned.into_inner();
                *slot = Some(handle);
            }
        }
        Ok(core)
    }

    pub(super) fn take_initial_route(&self) -> Option<RouteHome> {
        match self.initial_route.lock() {
            Ok(mut route) => route.take(),
            Err(_) => None,
        }
    }

    pub(super) fn register(
        &self,
        route: RouteHome,
        fd: RawFd,
        interest: Interest,
        pid: u64,
        marker: Atom,
    ) -> Result<ReadinessToken, ReadinessError> {
        self.check_available()?;
        let mut table = self.lock_table_for_mutation()?;
        let index = table.vacant_slot();
        let slot = &mut table.slots[index];
        slot.generation = slot.generation.wrapping_add(1).max(1);
        let token = ReadinessToken {
            slot: index as u32,
            generation: Generation(slot.generation),
        };
        let mut source = SourceFd(&fd);
        self.registry
            .register(&mut source, token.mio_token(), interest.as_mio())
            .map_err(|error| ReadinessError::Register {
                errno: errno(&error),
            })?;
        slot.record = Some(Registration {
            fd,
            generation: token.generation,
            pid,
            marker,
            armed: interest,
            route,
            state: RecordState::Live,
        });
        Ok(token)
    }

    pub(super) fn rearm(
        &self,
        token: &ReadinessToken,
        interest: Interest,
    ) -> Result<(), ReadinessError> {
        self.check_available()?;
        let mut table = self.lock_table_for_mutation()?;
        let record = table
            .slots
            .get_mut(token.slot as usize)
            .and_then(|slot| slot.record.as_mut())
            .filter(|record| {
                record.generation == token.generation && record.state == RecordState::Live
            })
            .ok_or(ReadinessError::UnknownToken)?;
        let mut source = SourceFd(&record.fd);
        self.registry
            .reregister(&mut source, token.mio_token(), interest.as_mio())
            .map_err(|error| ReadinessError::Register {
                errno: errno(&error),
            })?;
        record.armed = interest;
        Ok(())
    }

    fn check_available(&self) -> Result<(), ReadinessError> {
        if self.failed.load(Ordering::Acquire) || self.stopping.load(Ordering::Acquire) {
            Err(ReadinessError::ServiceFailed)
        } else {
            Ok(())
        }
    }

    fn lock_table_for_mutation(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, RegistrationTable>, ReadinessError> {
        match self.table.lock() {
            Ok(table) => Ok(table),
            Err(_) => {
                self.failed.store(true, Ordering::Release);
                Err(ReadinessError::ServiceFailed)
            }
        }
    }

    fn lock_table_for_tombstone(&self) -> (std::sync::MutexGuard<'_, RegistrationTable>, bool) {
        let failed = self.failed.load(Ordering::Acquire);
        match self.table.lock() {
            Ok(table) => (table, failed),
            Err(poisoned) => {
                // Publish FAILED before the one sanctioned poisoned-table
                // recovery: every subsequent register/rearm refuses pre-lock.
                self.failed.store(true, Ordering::Release);
                (poisoned.into_inner(), true)
            }
        }
    }

    pub(super) fn deregister(&self, token: ReadinessToken) {
        let epoch = self.poll_epoch.load(Ordering::Acquire);
        let bumped = {
            let (mut table, failed) = self.lock_table_for_tombstone();
            let bumped = self.tombstone(&mut table, token);
            (bumped, failed)
        };
        let (Some(bumped_generation), failed) = bumped else {
            return;
        };
        if !failed {
            self.wake_and_wait(epoch);
        }
        self.free_draining(token.slot as usize, bumped_generation);
    }

    fn tombstone(&self, table: &mut RegistrationTable, token: ReadinessToken) -> Option<u64> {
        let slot = table.slots.get_mut(token.slot as usize)?;
        let record = slot.record.as_mut()?;
        if record.generation != token.generation {
            return None;
        }
        slot.generation = slot.generation.wrapping_add(1).max(1);
        record.generation = Generation(slot.generation);
        record.state = RecordState::Draining;
        record.armed = Interest(0);
        let mut source = SourceFd(&record.fd);
        let _ = self.registry.deregister(&mut source);
        Some(slot.generation)
    }

    fn free_draining(&self, index: usize, generation: u64) {
        let (mut table, _failed) = self.lock_table_for_tombstone();
        if let Some(slot) = table.slots.get_mut(index)
            && slot.generation == generation
            && slot
                .record
                .as_ref()
                .is_some_and(|record| record.state == RecordState::Draining)
        {
            slot.record = None;
        }
    }

    pub(super) fn deregister_all_for(&self, consumer: ServiceConsumerId) {
        self.deregister_matching(|record| record.route.consumer == consumer);
    }

    pub(super) fn deregister_pid(&self, consumer: ServiceConsumerId, pid: u64) {
        self.deregister_matching(|record| record.route.consumer == consumer && record.pid == pid);
    }

    fn deregister_matching(&self, predicate: impl Fn(&Registration) -> bool) {
        let epoch = self.poll_epoch.load(Ordering::Acquire);
        let (mut table, failed) = self.lock_table_for_tombstone();
        let mut draining = Vec::new();
        for (index, slot) in table.slots.iter_mut().enumerate() {
            let Some(record) = slot.record.as_mut() else {
                continue;
            };
            if record.state != RecordState::Live || !predicate(record) {
                continue;
            }
            slot.generation = slot.generation.wrapping_add(1).max(1);
            record.generation = Generation(slot.generation);
            record.state = RecordState::Draining;
            record.armed = Interest(0);
            let mut source = SourceFd(&record.fd);
            let _ = self.registry.deregister(&mut source);
            draining.push((index, slot.generation));
        }
        drop(table);
        if !failed && !draining.is_empty() {
            self.wake_and_wait(epoch);
        }
        for (index, generation) in draining {
            self.free_draining(index, generation);
        }
    }

    fn wake_and_wait(&self, epoch: u64) {
        let _ = self.waker.wake();
        let Ok(mut guard) = self.epoch_lock.lock() else {
            return;
        };
        while self.poll_epoch.load(Ordering::Acquire) <= epoch
            && !self.failed.load(Ordering::Acquire)
            && !self.stopping.load(Ordering::Acquire)
        {
            match self.epoch_changed.wait(guard) {
                Ok(next) => guard = next,
                Err(_) => return,
            }
        }
    }

    fn poll_loop(&self) -> std::io::Result<()> {
        let mut events = Events::with_capacity(256);
        loop {
            self.poll_epoch.fetch_add(1, Ordering::AcqRel);
            self.epoch_changed.notify_all();
            if self.stopping.load(Ordering::Acquire) {
                return Ok(());
            }
            let poll_result = match self.poll.lock() {
                Ok(mut poll) => poll.poll(&mut events, None),
                Err(_) => return Err(std::io::Error::other("readiness poll lock poisoned")),
            };
            if let Err(error) = poll_result {
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            for event in &events {
                if event.token() != WAKER_TOKEN {
                    self.deliver_event(event)?;
                }
            }
        }
    }

    pub(super) fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        let _ = self.waker.wake();
        let handle = match self.thread.lock() {
            Ok(mut thread) => thread.take(),
            Err(mut poisoned) => poisoned.get_mut().take(),
        };
        if let Some(handle) = handle {
            let _ = handle.join();
        }
        self.epoch_changed.notify_all();
    }

    pub(super) fn poll_thread_names(&self) -> Vec<String> {
        if self.failed.load(Ordering::Acquire) || self.stopping.load(Ordering::Acquire) {
            return Vec::new();
        }
        let live = match self.thread.lock() {
            Ok(thread) => thread.as_ref().is_some_and(|handle| !handle.is_finished()),
            Err(_) => false,
        };
        if live {
            vec![format!("{READINESS_POLL_THREAD_PREFIX}-poll")]
        } else {
            Vec::new()
        }
    }

    pub(super) fn live_registration_count(&self) -> usize {
        match self.table.lock() {
            Ok(table) => table
                .slots
                .iter()
                .filter(|slot| {
                    slot.record
                        .as_ref()
                        .is_some_and(|record| record.state == RecordState::Live)
                })
                .count(),
            Err(poisoned) if self.failed.load(Ordering::Acquire) => poisoned
                .into_inner()
                .slots
                .iter()
                .filter(|slot| {
                    slot.record
                        .as_ref()
                        .is_some_and(|record| record.state == RecordState::Live)
                })
                .count(),
            Err(_) => 0,
        }
    }

    #[cfg(test)]
    pub(super) fn poll_iterations(&self) -> u64 {
        self.poll_epoch.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn panic_next_delivery(&self) {
        self.panic_in_delivery.store(true, Ordering::Release);
    }
}

impl Drop for ReadinessCore {
    fn drop(&mut self) {
        self.shutdown();
    }
}
