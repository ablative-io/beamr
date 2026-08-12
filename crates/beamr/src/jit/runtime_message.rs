//! Message-passing runtime helpers callable from JIT-generated code.

use crate::error::ExecError;
use crate::interpreter::NativeServices;
use crate::interpreter::opcodes::messaging::{send_local_via_facility, send_to_self};
use crate::process::{JitStatus, Process, ProcessStatus, ReceiveTimeout};
use crate::term::Term;
use crate::term::pid_ref::PidRef;

use super::ir_exceptions::JitReturn;
use super::runtime::process_from_abi;

const RECEIVE_STATUS_MESSAGE: u8 = 0;
const RECEIVE_STATUS_EMPTY: u8 = 1;
const WAIT_STATUS_NEW_MESSAGE: u8 = 0;
const WAIT_STATUS_TIMEOUT: u8 = 1;
const WAIT_STATUS_WAITING: u8 = 2;

pub(crate) const SEND_STATUS_SENT: u8 = 0;
pub(crate) const SEND_STATUS_ABORT: u8 = 1;

/// The `Send` opcode from compiled code, arm-for-arm equal to the
/// interpreter's `opcodes::messaging::send`.
///
/// Every arm either produces the interpreter's exact observable outcome or
/// parks the interpreter's exact `ExecError` via
/// [`Process::set_jit_exec_error`] and returns [`SEND_STATUS_ABORT`] — there
/// is no silently-unimplemented destination. The abort arms mutate nothing
/// (or restore it, as the shared facility path does for the sender clock), so
/// the seam's abort is equivalent to the interpreter erroring on the same
/// instruction. `Send` stays out of `is_runtime_deopt_capable`: the abort
/// route ends the slice at `call_native`, it never restarts the function.
///
/// Facilities are reached through the [`JitRuntimeContext`] services pointer
/// the interpreter installs around every native call; a missing context or
/// facility mirrors the interpreter's own documented facility-absent arm
/// (silent fall-through with the send's success value), not a new behavior.
///
/// [`JitRuntimeContext`]: crate::process::JitRuntimeContext
pub(crate) extern "C" fn jit_send_message(
    process: *mut Process,
    dest_pid: u64,
    message: u64,
) -> JitReturn {
    let Some(process) = process_from_abi(process) else {
        // ABI null guard, unreachable from generated code (the process pointer
        // is the compiled function's own argument).
        return send_return(SEND_STATUS_SENT, message);
    };
    let message_term = Term::from_raw(message);
    let Some(target) = PidRef::new(Term::from_raw(dest_pid)) else {
        process.set_jit_exec_error(ExecError::Badarg);
        return send_return(SEND_STATUS_ABORT, 0);
    };
    // SAFETY: The interpreter installs the services pointer in
    // `JitRuntimeContext` for exactly the synchronous duration of the native
    // JIT call; helpers run before that context is cleared. `as_ref` rejects
    // the null placeholder some installers pass.
    let services: Option<&NativeServices> = process
        .jit_runtime_context()
        .and_then(|context| unsafe { context.services.as_ref() });
    if !target.is_local() {
        #[cfg(feature = "net")]
        {
            let Some(facility) = services.and_then(|services| services.distribution_send.clone())
            else {
                process.set_jit_exec_error(ExecError::NoConnection);
                return send_return(SEND_STATUS_ABORT, 0);
            };
            return match facility.send_remote(Term::from_raw(dest_pid), message_term) {
                Ok(()) => {
                    #[cfg(feature = "telemetry")]
                    crate::telemetry::metrics::record_message_sent();
                    send_return(SEND_STATUS_SENT, message)
                }
                Err(error) => {
                    process.set_jit_exec_error(
                        crate::interpreter::opcodes::messaging::distribution_send_error(error),
                    );
                    send_return(SEND_STATUS_ABORT, 0)
                }
            };
        }
        #[cfg(not(feature = "net"))]
        {
            process.set_jit_exec_error(ExecError::NoConnection);
            return send_return(SEND_STATUS_ABORT, 0);
        }
    }
    let target_pid = target.pid_number();
    let replay_driver = services.and_then(|services| services.replay_driver.clone());
    if target_pid == process.pid() {
        match send_to_self(process, message_term, replay_driver.as_ref()) {
            Ok(()) => send_return(SEND_STATUS_SENT, message),
            Err(error) => {
                process.set_jit_exec_error(error);
                send_return(SEND_STATUS_ABORT, 0)
            }
        }
    } else if let Some(facility) = services.and_then(|services| services.local_send.clone()) {
        match send_local_via_facility(
            process,
            facility.as_ref(),
            target_pid,
            message_term,
            replay_driver.as_ref(),
        ) {
            Ok(()) => {
                #[cfg(feature = "telemetry")]
                crate::telemetry::metrics::record_message_sent();
                send_return(SEND_STATUS_SENT, message)
            }
            Err(error) => {
                process.set_jit_exec_error(error);
                send_return(SEND_STATUS_ABORT, 0)
            }
        }
    } else {
        // No local-send facility: the interpreter's documented arm falls
        // through silently (facility-less embedders), preserving x0.
        send_return(SEND_STATUS_SENT, message)
    }
}

const fn send_return(status: u8, value: u64) -> JitReturn {
    JitReturn {
        status,
        _padding: [0; 7],
        value,
    }
}

pub(crate) extern "C" fn jit_receive_peek(process: *mut Process) -> JitReturn {
    let Some(process) = process_from_abi(process) else {
        return receive_return(RECEIVE_STATUS_EMPTY, 0);
    };
    match process.mailbox_mut().current_message() {
        Some(message) => receive_return(RECEIVE_STATUS_MESSAGE, message.raw()),
        None => receive_return(RECEIVE_STATUS_EMPTY, 0),
    }
}

const fn receive_return(status: u8, value: u64) -> JitReturn {
    JitReturn {
        status,
        _padding: [0; 7],
        value,
    }
}

pub(crate) extern "C" fn jit_receive_next(process: *mut Process) {
    let Some(process) = process_from_abi(process) else {
        return;
    };
    process.mailbox_mut().advance_save_pointer();
}

pub(crate) extern "C" fn jit_receive_accept(process: *mut Process) {
    let Some(process) = process_from_abi(process) else {
        return;
    };
    let _ = process.mailbox_mut().remove_current_message();
    process.set_receive_timeout(None);
    process.set_receive_timer_ref(None);
}

pub(crate) extern "C" fn jit_receive_wait(process: *mut Process) -> u8 {
    let Some(process) = process_from_abi(process) else {
        return WAIT_STATUS_WAITING;
    };
    if process.mailbox_mut().current_message().is_some() {
        return WAIT_STATUS_NEW_MESSAGE;
    }
    transition_process_to_waiting(process);
    process.set_jit_status(Some(JitStatus::Yield));
    WAIT_STATUS_WAITING
}

pub(crate) extern "C" fn jit_receive_wait_timeout(process: *mut Process, timeout: u64) -> u8 {
    let Some(process) = process_from_abi(process) else {
        return WAIT_STATUS_WAITING;
    };
    if process.mailbox_mut().current_message().is_some() {
        return WAIT_STATUS_NEW_MESSAGE;
    }
    let milliseconds = Term::from_raw(timeout)
        .as_small_int()
        .and_then(|value| u64::try_from(value).ok());
    if milliseconds == Some(0) {
        return WAIT_STATUS_TIMEOUT;
    }
    if let Some(milliseconds) = milliseconds
        && let Some(position) = process.code_position()
    {
        process.set_receive_timeout(Some(ReceiveTimeout {
            timeout_position: position,
            milliseconds,
        }));
        process.set_receive_timer_ref(None);
    }
    transition_process_to_waiting(process);
    process.set_jit_status(Some(JitStatus::Yield));
    WAIT_STATUS_WAITING
}

pub(crate) extern "C" fn jit_receive_timeout(process: *mut Process) {
    let Some(process) = process_from_abi(process) else {
        return;
    };
    process.mailbox_mut().reset_save_pointer();
    process.set_receive_timeout(None);
    process.set_receive_timer_ref(None);
}

fn transition_process_to_waiting(process: &mut Process) {
    if process.status() == ProcessStatus::New {
        let _ = process.transition_to(ProcessStatus::Running);
    }
    let _ = process.transition_to(ProcessStatus::Waiting);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::opcodes::messaging;
    use crate::native::local_send::{LocalSendError, LocalSendFacility, LocalSendRequest};
    use crate::process::JitRuntimeContext;
    use std::sync::{Arc, Mutex};

    /// (target_pid, sender_pid, raw message, sender_clock, replay_driver seen).
    type SeenRequest = (u64, u64, u64, u64, bool);

    /// Captures one delivery request so the walls can assert the clock/replay
    /// contract crossed the facility boundary intact.
    struct RecordingFacility {
        seen: Mutex<Option<SeenRequest>>,
        mismatch: bool,
    }

    impl RecordingFacility {
        fn recording() -> Self {
            Self {
                seen: Mutex::new(None),
                mismatch: false,
            }
        }

        fn mismatching() -> Self {
            Self {
                seen: Mutex::new(None),
                mismatch: true,
            }
        }
    }

    impl LocalSendFacility for RecordingFacility {
        fn send_local(&self, request: LocalSendRequest<'_>) -> Result<(), LocalSendError> {
            *self.seen.lock().unwrap() = Some((
                request.target_pid,
                request.sender_pid,
                request.message.raw(),
                request.sender_clock,
                request.replay_driver.is_some(),
            ));
            if self.mismatch {
                return Err(LocalSendError::ReplayMismatch("wall".into()));
            }
            Ok(())
        }
    }

    fn services_with(facility: Arc<dyn LocalSendFacility>) -> crate::interpreter::NativeServices {
        crate::interpreter::NativeServices {
            local_send: Some(facility),
            ..Default::default()
        }
    }

    fn install_context(process: &mut Process, services: &crate::interpreter::NativeServices) {
        process.set_jit_runtime_context(Some(JitRuntimeContext::new(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            services as *const crate::interpreter::NativeServices,
        )));
    }

    // Defect-2 wall: the interpreter ticks the sender clock on every self-send;
    // the pre-fix helper pushed to the mailbox without touching it.
    #[test]
    fn compiled_self_send_ticks_sender_clock_and_delivers() {
        let mut process = Process::new(1, 256);
        let clock_before = process.logical_clock();
        let message = Term::atom(crate::atom::Atom::OK);

        let returned = jit_send_message(&mut process, Term::pid(1).raw(), message.raw());

        assert_eq!(returned.status, SEND_STATUS_SENT);
        assert_eq!(returned.value, message.raw());
        assert_eq!(
            process.logical_clock(),
            clock_before + 2,
            "self-send must tick the sender clock and observe it as receiver"
        );
        assert_eq!(process.take_jit_exec_error(), None);
        assert_eq!(
            process.mailbox_mut().current_message(),
            Some(message),
            "self-send must land in the sender's own mailbox"
        );
    }

    // Cross-process wall at the helper level: the message must reach the
    // facility with the ticked clock, not be dropped on the floor.
    #[test]
    fn compiled_cross_process_send_routes_through_local_send_facility() {
        let facility = Arc::new(RecordingFacility::recording());
        let services = services_with(Arc::clone(&facility) as Arc<dyn LocalSendFacility>);
        let mut process = Process::new(1, 256);
        install_context(&mut process, &services);
        let clock_before = process.logical_clock();
        let message = Term::atom(crate::atom::Atom::OK);

        let returned = jit_send_message(&mut process, Term::pid(2).raw(), message.raw());

        assert_eq!(returned.status, SEND_STATUS_SENT);
        assert_eq!(returned.value, message.raw());
        assert_eq!(process.take_jit_exec_error(), None);
        let seen = facility
            .seen
            .lock()
            .unwrap()
            .expect("facility must be reached");
        assert_eq!(seen.0, 2, "target pid");
        assert_eq!(seen.1, 1, "sender pid");
        assert_eq!(seen.2, message.raw(), "message term");
        assert_eq!(
            seen.3,
            clock_before + 1,
            "sender clock ticked before delivery"
        );
    }

    // Interpreter-parity wall: a replay mismatch aborts with the clock restored,
    // exactly like `messaging::send`'s facility arm.
    #[test]
    fn replay_mismatch_restores_clock_and_aborts_with_interpreter_error() {
        let facility = Arc::new(RecordingFacility::mismatching());
        let services = services_with(Arc::clone(&facility) as Arc<dyn LocalSendFacility>);
        let mut process = Process::new(1, 256);
        install_context(&mut process, &services);
        let clock_before = process.logical_clock();

        let returned = jit_send_message(
            &mut process,
            Term::pid(2).raw(),
            Term::atom(crate::atom::Atom::OK).raw(),
        );

        assert_eq!(returned.status, SEND_STATUS_ABORT);
        assert_eq!(
            process.logical_clock(),
            clock_before,
            "clock restored on abort"
        );
        assert_eq!(
            process.take_jit_exec_error(),
            Some(ExecError::ReplayMismatch("wall".into()))
        );
    }

    // Parity two-arm in one test: the compiled helper and the interpreter must
    // produce the SAME error for a non-pid destination — no silent success.
    #[test]
    fn non_pid_destination_aborts_with_the_interpreter_badarg() {
        let mut process = Process::new(1, 256);
        let clock_before = process.logical_clock();
        let not_a_pid = Term::atom(crate::atom::Atom::OK);
        let message = Term::atom(crate::atom::Atom::ERROR);

        let returned = jit_send_message(&mut process, not_a_pid.raw(), message.raw());

        assert_eq!(returned.status, SEND_STATUS_ABORT);
        assert_eq!(process.take_jit_exec_error(), Some(ExecError::Badarg));
        assert_eq!(
            process.logical_clock(),
            clock_before,
            "abort arms mutate nothing"
        );
        assert_eq!(process.mailbox_mut().current_message(), None);

        // Interpreter arm on the same operands.
        let mut interpreted = Process::new(1, 256);
        interpreted.set_x_reg(0, not_a_pid);
        interpreted.set_x_reg(1, message);
        let outcome = messaging::send(&mut interpreted, None, None, None, None);
        assert!(matches!(outcome, Err(ExecError::Badarg)));
    }

    // Facility-absent parity: the interpreter's documented arm falls through
    // silently without ticking the clock; the helper must match it, not invent
    // its own behavior in either direction.
    #[test]
    fn missing_facility_falls_through_exactly_like_the_interpreter() {
        let mut process = Process::new(1, 256);
        let clock_before = process.logical_clock();
        let message = Term::atom(crate::atom::Atom::OK);

        let returned = jit_send_message(&mut process, Term::pid(2).raw(), message.raw());

        assert_eq!(returned.status, SEND_STATUS_SENT);
        assert_eq!(returned.value, message.raw());
        assert_eq!(process.take_jit_exec_error(), None);
        assert_eq!(process.logical_clock(), clock_before);

        // Interpreter arm on the same operands, no facilities supplied.
        let mut interpreted = Process::new(1, 256);
        let interpreted_clock_before = interpreted.logical_clock();
        interpreted.set_x_reg(0, Term::pid(2));
        interpreted.set_x_reg(1, message);
        let outcome = messaging::send(&mut interpreted, None, None, None, None);
        assert!(outcome.is_ok());
        assert_eq!(interpreted.logical_clock(), interpreted_clock_before);
    }

    // Remote-destination wall (net): without a distribution facility both tiers
    // must refuse with NoConnection — never a silent drop.
    #[cfg(feature = "net")]
    #[test]
    fn remote_destination_without_distribution_aborts_with_noconnection() {
        use crate::atom::Atom;
        use crate::term::boxed::write_external_pid;

        let mut heap = [0_u64; 4];
        let remote = write_external_pid(&mut heap, Atom::OK, 99, 7).expect("external pid fits");
        let mut process = Process::new(1, 256);

        let returned = jit_send_message(&mut process, remote.raw(), Term::atom(Atom::OK).raw());

        assert_eq!(returned.status, SEND_STATUS_ABORT);
        assert_eq!(process.take_jit_exec_error(), Some(ExecError::NoConnection));

        // Interpreter arm on the same operands.
        let mut interpreted = Process::new(1, 256);
        let mut interpreted_heap = [0_u64; 4];
        let interpreted_remote =
            write_external_pid(&mut interpreted_heap, Atom::OK, 99, 7).expect("fits");
        interpreted.set_x_reg(0, interpreted_remote);
        interpreted.set_x_reg(1, Term::atom(Atom::OK));
        let outcome = messaging::send(&mut interpreted, None, None, None, None);
        assert!(matches!(outcome, Err(ExecError::NoConnection)));
    }
}
