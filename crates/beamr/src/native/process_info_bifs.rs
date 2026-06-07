//! Process introspection BIFs — process_info/1,2.
//!
//! This module exposes a small OTP-facing subset of Erlang's process_info BIFs.
//! Scheduler state is read through [`ProcessInfoFacility`] snapshots; all boxed
//! return terms are allocated on the caller's heap after scheduler locks have
//! been released.

use crate::atom::{Atom, AtomTable};
use crate::native::{
    BifRegistryImpl, Capability, NativeFn, NativeRegistrationError, ProcessContext,
};
use crate::process::{Monitor, ProcessStatus};
use crate::term::Term;

/// Supported process_info item atoms in deterministic process_info/1 order.
pub const DEFAULT_PROCESS_INFO_ITEMS: &[ProcessInfoItem] = &[
    ProcessInfoItem::CurrentFunction,
    ProcessInfoItem::HeapSize,
    ProcessInfoItem::MessageQueueLen,
    ProcessInfoItem::RegisteredName,
    ProcessInfoItem::Status,
    ProcessInfoItem::TrapExit,
    ProcessInfoItem::Links,
    ProcessInfoItem::Monitors,
];

type ProcessInfoBif = (&'static str, u8, Capability, NativeFn);

const PROCESS_INFO_BIFS: &[ProcessInfoBif] = &[
    ("process_info", 1, Capability::Pure, bif_process_info_1),
    ("process_info", 2, Capability::Pure, bif_process_info_2),
];

/// Registers process introspection BIFs into the VM-owned BIF registry.
pub fn register_process_info_bifs(
    registry: &BifRegistryImpl,
    atom_table: &AtomTable,
) -> Result<(), NativeRegistrationError> {
    let erlang = atom_table.intern("erlang");

    for &(function_name, arity, capability, native_function) in PROCESS_INFO_BIFS {
        let function = atom_table.intern(function_name);
        registry.register(erlang, function, arity, native_function, capability)?;
    }

    Ok(())
}

/// Native-facing process introspection service.
pub trait ProcessInfoFacility: Send + Sync {
    /// Snapshot one supported process_info item for `pid`.
    fn process_info(&self, pid: u64, item: ProcessInfoItem) -> Option<ProcessInfoSnapshot>;
}

/// Supported process_info item names.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum ProcessInfoItem {
    /// `current_function` -> `{Module, Function, Arity}`.
    CurrentFunction,
    /// `heap_size` -> words used on the process heap.
    HeapSize,
    /// `message_queue_len` -> mailbox message count.
    MessageQueueLen,
    /// `registered_name` -> atom name or `[]`.
    RegisteredName,
    /// `status` -> `running | waiting | suspended`.
    Status,
    /// `trap_exit` -> boolean atom.
    TrapExit,
    /// `links` -> list of PIDs.
    Links,
    /// `monitors` -> list of monitor info tuples.
    Monitors,
}

impl ProcessInfoItem {
    /// Atom for this item.
    #[must_use]
    pub const fn atom(self) -> Atom {
        match self {
            Self::CurrentFunction => Atom::CURRENT_FUNCTION,
            Self::HeapSize => Atom::HEAP_SIZE,
            Self::MessageQueueLen => Atom::MESSAGE_QUEUE_LEN,
            Self::RegisteredName => Atom::REGISTERED_NAME,
            Self::Status => Atom::STATUS,
            Self::TrapExit => Atom::TRAP_EXIT,
            Self::Links => Atom::LINKS,
            Self::Monitors => Atom::MONITORS,
        }
    }

    fn from_atom(atom: Atom) -> Option<Self> {
        if atom == Atom::CURRENT_FUNCTION {
            Some(Self::CurrentFunction)
        } else if atom == Atom::HEAP_SIZE {
            Some(Self::HeapSize)
        } else if atom == Atom::MESSAGE_QUEUE_LEN {
            Some(Self::MessageQueueLen)
        } else if atom == Atom::REGISTERED_NAME {
            Some(Self::RegisteredName)
        } else if atom == Atom::STATUS {
            Some(Self::Status)
        } else if atom == Atom::TRAP_EXIT {
            Some(Self::TrapExit)
        } else if atom == Atom::LINKS {
            Some(Self::Links)
        } else if atom == Atom::MONITORS {
            Some(Self::Monitors)
        } else {
            None
        }
    }
}

/// Non-allocating process information snapshot copied out of scheduler state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessInfoSnapshot {
    /// Current module/function/arity, if func_info has run.
    CurrentFunction(Option<(Atom, Atom, u8)>),
    /// Process heap words in use.
    HeapSize(usize),
    /// Number of queued mailbox messages.
    MessageQueueLen(usize),
    /// Registered atom name, when known.
    RegisteredName(Option<Atom>),
    /// Lifecycle status.
    Status(ProcessStatus),
    /// Trap-exit flag.
    TrapExit(bool),
    /// Linked process IDs in deterministic process-local order.
    Links(Vec<u64>),
    /// Monitor metadata attached to the process.
    Monitors {
        owner_pid: u64,
        monitors: Vec<Monitor>,
    },
}

/// erlang:process_info/1 — return all default process info items.
pub fn bif_process_info_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [pid_term] = args else {
        return Err(badarg());
    };
    let pid = pid_term.as_pid().ok_or_else(badarg)?;
    let facility = context.process_info_facility().ok_or_else(badarg)?;

    let snapshots: Option<Vec<_>> = DEFAULT_PROCESS_INFO_ITEMS
        .iter()
        .copied()
        .map(|item| {
            facility
                .process_info(pid, item)
                .map(|snapshot| (item, snapshot))
        })
        .collect();
    let Some(snapshots) = snapshots else {
        return Ok(Term::atom(Atom::UNDEFINED));
    };

    let mut tuples = Vec::with_capacity(snapshots.len());
    for (item, snapshot) in snapshots {
        tuples.push(allocate_item_tuple(context, item, &snapshot)?);
    }
    context.alloc_list(&tuples)
}

/// erlang:process_info/2 — return one process info item.
pub fn bif_process_info_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [pid_term, item_term] = args else {
        return Err(badarg());
    };
    let pid = pid_term.as_pid().ok_or_else(badarg)?;
    let item_atom = item_term.as_atom().ok_or_else(badarg)?;
    let item = ProcessInfoItem::from_atom(item_atom).ok_or_else(badarg)?;
    let facility = context.process_info_facility().ok_or_else(badarg)?;
    let Some(snapshot) = facility.process_info(pid, item) else {
        return Ok(Term::atom(Atom::UNDEFINED));
    };

    allocate_item_tuple(context, item, &snapshot)
}

fn allocate_item_tuple(
    context: &mut ProcessContext,
    item: ProcessInfoItem,
    snapshot: &ProcessInfoSnapshot,
) -> Result<Term, Term> {
    let value = allocate_value(context, snapshot)?;
    context.alloc_tuple(&[Term::atom(item.atom()), value])
}

fn allocate_value(
    context: &mut ProcessContext,
    snapshot: &ProcessInfoSnapshot,
) -> Result<Term, Term> {
    match snapshot {
        ProcessInfoSnapshot::CurrentFunction(current_mfa) => {
            let (module, function, arity) =
                current_mfa.unwrap_or((Atom::UNDEFINED, Atom::UNDEFINED, 0));
            let arity = Term::try_small_int(i64::from(arity)).ok_or_else(badarg)?;
            context.alloc_tuple(&[Term::atom(module), Term::atom(function), arity])
        }
        ProcessInfoSnapshot::HeapSize(words) | ProcessInfoSnapshot::MessageQueueLen(words) => {
            let value = i64::try_from(*words).map_err(|_| badarg())?;
            Term::try_small_int(value).ok_or_else(badarg)
        }
        ProcessInfoSnapshot::RegisteredName(name) => Ok(name.map_or(Term::NIL, Term::atom)),
        ProcessInfoSnapshot::Status(status) => Ok(Term::atom(status_atom(*status))),
        ProcessInfoSnapshot::TrapExit(value) => Ok(bool_atom(*value)),
        ProcessInfoSnapshot::Links(links) => {
            let terms: Option<Vec<_>> = links.iter().copied().map(Term::try_pid).collect();
            context.alloc_list(&terms.ok_or_else(badarg)?)
        }
        ProcessInfoSnapshot::Monitors {
            owner_pid,
            monitors,
        } => {
            let mut tuples = Vec::new();
            for monitor in monitors {
                if monitor.watcher() == *owner_pid {
                    let target = Term::try_pid(monitor.target()).ok_or_else(badarg)?;
                    tuples.push(context.alloc_tuple(&[Term::atom(Atom::PROCESS), target])?);
                }
            }
            context.alloc_list(&tuples)
        }
    }
}

fn status_atom(status: ProcessStatus) -> Atom {
    match status {
        ProcessStatus::Waiting => Atom::WAITING,
        ProcessStatus::Suspended => Atom::SUSPENDED,
        ProcessStatus::Exited(_) => Atom::SUSPENDED,
        ProcessStatus::New | ProcessStatus::Running | ProcessStatus::Yielded => Atom::RUNNING,
    }
}

const fn bool_atom(value: bool) -> Term {
    if value {
        Term::atom(Atom::TRUE)
    } else {
        Term::atom(Atom::FALSE)
    }
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

#[cfg(test)]
mod tests;
