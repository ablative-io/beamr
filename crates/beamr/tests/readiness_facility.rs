#![cfg(feature = "readiness")]

//! Public-API proof for the §1.4 in-slice readiness loop.

use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use beamr::atom::Atom;
use beamr::module::ModuleRegistry;
use beamr::native::ReadinessFacility;
use beamr::native::native_process::{NativeContext, NativeHandler, NativeOutcome};
use beamr::process::ExitReason;
use beamr::scheduler::{
    Interest, ReadinessError, ReadinessToken, Scheduler, SchedulerConfig, SchedulerServices,
};
use beamr::term::Term;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
enum HandlerEvent {
    Registered(ReadinessToken),
    Ready(usize),
    FacilityAbsent,
    Failed(String),
}

struct ReadinessLoop {
    reader: UnixStream,
    marker: Atom,
    stop: Atom,
    token: Option<ReadinessToken>,
    ready_count: usize,
    events: mpsc::Sender<HandlerEvent>,
}

impl ReadinessLoop {
    fn fail(&self, message: impl Into<String>) -> NativeOutcome {
        let _sent = self.events.send(HandlerEvent::Failed(message.into()));
        NativeOutcome::Stop(ExitReason::Error)
    }

    fn drain_to_would_block(&mut self) -> io::Result<()> {
        let mut buffer = [0_u8; 64];
        loop {
            match self.reader.read(&mut buffer) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "readiness source closed",
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error),
            }
        }
    }
}

// Naming the trait and all three scheduler types here proves that an external
// consumer can express the complete facility call through public paths only.
fn public_register(
    facility: &dyn ReadinessFacility,
    fd: RawFd,
    pid: u64,
    marker: Atom,
) -> Result<ReadinessToken, ReadinessError> {
    facility.register(fd, Interest::READABLE, pid, marker)
}

impl NativeHandler for ReadinessLoop {
    fn handle(&mut self, context: &mut NativeContext<'_>) -> NativeOutcome {
        if self.token.is_none() {
            let pid = context.self_pid();
            let Some(facility) = context.readiness_facility() else {
                let _sent = self.events.send(HandlerEvent::FacilityAbsent);
                return NativeOutcome::Stop(ExitReason::Error);
            };
            let token = match public_register(facility, self.reader.as_raw_fd(), pid, self.marker) {
                Ok(token) => token,
                Err(error) => return self.fail(format!("register failed: {error}")),
            };
            self.token = Some(token);
            let _sent = self.events.send(HandlerEvent::Registered(token));
            return NativeOutcome::Wait;
        }

        while let Some(message) = context.recv() {
            if message == Term::atom(self.stop) {
                return NativeOutcome::Stop(ExitReason::Normal);
            }
            if message != Term::atom(self.marker) {
                continue;
            }

            if let Err(error) = self.drain_to_would_block() {
                return self.fail(format!("drain failed: {error}"));
            }
            self.ready_count += 1;
            if self.ready_count == 1 {
                let Some(facility) = context.readiness_facility() else {
                    return self.fail("facility disappeared before rearm");
                };
                let Some(token) = self.token.as_ref() else {
                    return self.fail("registration token disappeared before rearm");
                };
                if let Err(error) = facility.rearm(token, Interest::READABLE) {
                    return self.fail(format!("rearm failed: {error}"));
                }
            }
            let _sent = self.events.send(HandlerEvent::Ready(self.ready_count));
        }

        NativeOutcome::Wait
    }
}

struct DisabledProbe {
    result: mpsc::Sender<bool>,
}

impl NativeHandler for DisabledProbe {
    fn handle(&mut self, context: &mut NativeContext<'_>) -> NativeOutcome {
        let _sent = self.result.send(context.readiness_facility().is_none());
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

fn scheduler(services: SchedulerServices) -> Arc<Scheduler> {
    Arc::new(
        Scheduler::with_services(
            SchedulerConfig {
                thread_count: Some(1),
                ..SchedulerConfig::default()
            },
            services,
            Arc::new(ModuleRegistry::new()),
        )
        .unwrap_or_else(|error| panic!("scheduler starts: {error}")),
    )
}

fn recv_event(receiver: &mpsc::Receiver<HandlerEvent>) -> HandlerEvent {
    let event = receiver
        .recv_timeout(TEST_TIMEOUT)
        .unwrap_or_else(|error| panic!("native handler event timed out: {error}"));
    if let HandlerEvent::Failed(message) = &event {
        panic!("native handler failed: {message}");
    }
    event
}

fn run_until_exit_bounded(scheduler: &Arc<Scheduler>, pid: u64) -> ExitReason {
    let (sender, receiver) = mpsc::channel();
    let scheduler_for_wait = Arc::clone(scheduler);
    std::thread::spawn(move || {
        let (reason, _result) = scheduler_for_wait.run_until_exit(pid);
        let _sent = sender.send(reason);
    });
    receiver
        .recv_timeout(TEST_TIMEOUT)
        .unwrap_or_else(|error| panic!("native process {pid} did not exit: {error}"))
}

#[test]
fn native_handler_registers_receives_rearms_and_receives_again() {
    let scheduler = scheduler(SchedulerServices::minimal().owned_readiness());
    let (reader, mut writer) = UnixStream::pair().expect("create readiness source");
    reader
        .set_nonblocking(true)
        .expect("make readiness source nonblocking");
    let (event_sender, event_receiver) = mpsc::channel();

    let pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(ReadinessLoop {
                reader: reader.try_clone().expect("clone readiness source"),
                marker: Atom::OK,
                stop: Atom::ERROR,
                token: None,
                ready_count: 0,
                events: event_sender.clone(),
            })
        }))
        .expect("spawn readiness consumer");

    let token = match recv_event(&event_receiver) {
        HandlerEvent::Registered(token) => token,
        event => panic!("expected registration, got {event:?}"),
    };

    writer.write_all(&[1]).expect("trigger first readiness");
    match recv_event(&event_receiver) {
        HandlerEvent::Ready(1) => {}
        event => panic!("expected first marker, got {event:?}"),
    }

    writer.write_all(&[2]).expect("trigger second readiness");
    match recv_event(&event_receiver) {
        HandlerEvent::Ready(2) => {}
        event => panic!("expected second marker, got {event:?}"),
    }

    // Deregistration remains host-side only; do it while the source and native
    // process are alive, then use an ordinary message to end the test process.
    scheduler.readiness_deregister(token);
    assert!(scheduler.enqueue_atom_message(pid, Atom::ERROR));
    assert_eq!(run_until_exit_bounded(&scheduler, pid), ExitReason::Normal);
    scheduler.shutdown();
}

#[test]
fn disabled_scheduler_exposes_typed_absence_to_native_handler() {
    let scheduler = scheduler(SchedulerServices::minimal());
    let (result_sender, result_receiver) = mpsc::channel();
    let pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(DisabledProbe {
                result: result_sender.clone(),
            })
        }))
        .expect("spawn disabled readiness probe");

    assert!(
        result_receiver
            .recv_timeout(TEST_TIMEOUT)
            .expect("disabled probe reports")
    );
    assert_eq!(run_until_exit_bounded(&scheduler, pid), ExitReason::Normal);
    scheduler.shutdown();
}
