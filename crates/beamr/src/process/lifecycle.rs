//! A process's lifecycle: the status graph it moves through, and the
//! teardown that releases what it owns when it exits.

use crate::mailbox::Mailbox;
use crate::process::heap::Heap;
use crate::process::stack::Stack;
use crate::term::Term;
#[cfg(feature = "threads")]
use crate::term::boxed::BoxedTag;

use super::{ExitReason, Process, ProcessError, ProcessStatus, X_REG_COUNT};

impl Process {
    /// Current lifecycle status.
    #[must_use]
    pub const fn status(&self) -> ProcessStatus {
        self.status
    }

    /// Transition this process to `next` if the lifecycle graph allows it.
    pub fn transition_to(&mut self, next: ProcessStatus) -> Result<(), ProcessError> {
        if Self::can_transition(self.status, next) {
            self.status = next;
            Ok(())
        } else {
            Err(ProcessError::InvalidStatusTransition {
                from: self.status,
                to: next,
            })
        }
    }

    const fn can_transition(from: ProcessStatus, to: ProcessStatus) -> bool {
        matches!(
            (from, to),
            (ProcessStatus::New, ProcessStatus::Running)
                | (ProcessStatus::Running, ProcessStatus::Yielded)
                | (ProcessStatus::Running, ProcessStatus::Waiting)
                | (ProcessStatus::Running, ProcessStatus::Suspended)
                | (ProcessStatus::Running, ProcessStatus::Exited(_))
                | (ProcessStatus::Yielded, ProcessStatus::Running)
                | (ProcessStatus::Yielded, ProcessStatus::Suspended)
                | (ProcessStatus::Waiting, ProcessStatus::Running)
                | (ProcessStatus::Waiting, ProcessStatus::Suspended)
                | (ProcessStatus::Suspended, ProcessStatus::Yielded)
                | (ProcessStatus::Suspended, ProcessStatus::Waiting)
        )
    }

    /// Mark the process exited and release owned runtime state that can keep
    /// heap terms alive after process death.
    pub fn terminate(&mut self, reason: ExitReason) {
        self.close_owned_fd_resources();
        self.status = ProcessStatus::Exited(reason);
        crate::gc::release_all_refcounted_resources(self);
        self.virtual_binary_heap = 0;
        self.heap = Heap::new(1);
        self.stack = Stack::new();
        self.mailbox = Mailbox::new();
        self.handlers.clear();
        self.nested_handler_floor = 0;
        self.current_exception = None;
        self.dictionary.clear();
        self.receive_timeout = None;
        self.receive_timer_ref = None;
        self.suspension = None;
        self.x_regs = [Term::NIL; X_REG_COUNT];
        self.float_regs = [0.0; 16];
        self.native_continuations.clear();
        self.native_roots.clear();
        self.reduction_counter = 0;
        self.code_position = None;
        self.current_module = None;
    }

    /// Close FD-backed resources owned by this process. The `io` subsystem only
    /// exists in the threaded (native) build; in the cooperative/wasm build there
    /// are no host file descriptors, so this is a no-op.
    #[cfg(feature = "threads")]
    fn close_owned_fd_resources(&mut self) {
        let owner_pid = self.pid;
        self.heap().visit_boxed_objects(|ptr, tag, _words| {
            if tag == BoxedTag::FdResource {
                crate::io::resource::close_owned_resource_at(ptr, owner_pid);
            }
        });
    }

    #[cfg(not(feature = "threads"))]
    fn close_owned_fd_resources(&mut self) {}
}
