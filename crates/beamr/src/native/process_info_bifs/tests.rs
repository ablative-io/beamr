use std::collections::HashMap;
use std::sync::Arc;

use crate::atom::{Atom, AtomTable};
use crate::native::process_info_bifs::{
    DEFAULT_PROCESS_INFO_ITEMS, ProcessInfoFacility, ProcessInfoItem, ProcessInfoSnapshot,
    bif_process_info_1, bif_process_info_2, register_process_info_bifs,
};
use crate::native::{BifRegistryImpl, ProcessContext};
use crate::process::heap::DEFAULT_HEAP_SIZE;
use crate::process::{Monitor, Process};
use crate::term::Term;
use crate::term::boxed::{Cons, Tuple};

#[derive(Default)]
struct MockProcessInfoFacility {
    snapshots: HashMap<(u64, ProcessInfoItem), ProcessInfoSnapshot>,
}

impl MockProcessInfoFacility {
    fn with_complete_process(pid: u64) -> Self {
        let mut snapshots = HashMap::new();
        snapshots.insert(
            (pid, ProcessInfoItem::CurrentFunction),
            ProcessInfoSnapshot::CurrentFunction(Some((Atom::MODULE, Atom::INFO, 2))),
        );
        snapshots.insert(
            (pid, ProcessInfoItem::HeapSize),
            ProcessInfoSnapshot::HeapSize(7),
        );
        snapshots.insert(
            (pid, ProcessInfoItem::MessageQueueLen),
            ProcessInfoSnapshot::MessageQueueLen(3),
        );
        snapshots.insert(
            (pid, ProcessInfoItem::RegisteredName),
            ProcessInfoSnapshot::RegisteredName(None),
        );
        snapshots.insert(
            (pid, ProcessInfoItem::Status),
            ProcessInfoSnapshot::Status(crate::process::ProcessStatus::Waiting),
        );
        snapshots.insert(
            (pid, ProcessInfoItem::TrapExit),
            ProcessInfoSnapshot::TrapExit(true),
        );
        snapshots.insert(
            (pid, ProcessInfoItem::Links),
            ProcessInfoSnapshot::Links(vec![11, 12]),
        );
        snapshots.insert(
            (pid, ProcessInfoItem::Monitors),
            ProcessInfoSnapshot::Monitors {
                owner_pid: pid,
                monitors: vec![Monitor::new(99, pid, 13), Monitor::new(100, 14, pid)],
            },
        );
        Self { snapshots }
    }
}

impl ProcessInfoFacility for MockProcessInfoFacility {
    fn process_info(&self, pid: u64, item: ProcessInfoItem) -> Option<ProcessInfoSnapshot> {
        self.snapshots.get(&(pid, item)).cloned()
    }
}

#[test]
fn register_process_info_bifs_registers_all() {
    let registry = BifRegistryImpl::new();
    let atom_table = AtomTable::with_common_atoms();
    register_process_info_bifs(&registry, &atom_table).expect("process_info bifs register");
    let erlang = atom_table.intern("erlang");
    let process_info = atom_table.intern("process_info");

    assert!(registry.lookup(erlang, process_info, 1).is_some());
    assert!(registry.lookup(erlang, process_info, 2).is_some());
}

#[test]
fn process_info_2_returns_each_supported_item_tuple() {
    let pid = 42;
    let mut process = Process::new(pid, DEFAULT_HEAP_SIZE);
    let mut context = ProcessContext::new();
    context.set_pid(Some(1));
    context.set_process_info_facility(Some(Arc::new(
        MockProcessInfoFacility::with_complete_process(pid),
    )));
    context.attach_process(&mut process, 2);

    for item in DEFAULT_PROCESS_INFO_ITEMS {
        let result = bif_process_info_2(&[Term::pid(pid), Term::atom(item.atom())], &mut context)
            .expect("process_info/2 succeeds");
        let tuple = Tuple::new(result).expect("{Item, Value}");
        assert_eq!(tuple.len(), 2);
        assert_eq!(tuple.get(0), Some(Term::atom(item.atom())));
        assert_supported_value_shape(*item, tuple.get(1).expect("value"));
    }
}

#[test]
fn process_info_2_returns_undefined_for_missing_process() {
    let mut process = Process::new(1, DEFAULT_HEAP_SIZE);
    let mut context = ProcessContext::new();
    context.set_process_info_facility(Some(Arc::new(MockProcessInfoFacility::default())));
    context.attach_process(&mut process, 2);

    let result = bif_process_info_2(&[Term::pid(999), Term::atom(Atom::STATUS)], &mut context)
        .expect("missing pid returns undefined");
    assert_eq!(result, Term::atom(Atom::UNDEFINED));
}

#[test]
fn process_info_2_rejects_bad_arguments_and_unknown_item() {
    let mut process = Process::new(1, DEFAULT_HEAP_SIZE);
    let mut context = ProcessContext::new();
    context.set_process_info_facility(Some(Arc::new(
        MockProcessInfoFacility::with_complete_process(1),
    )));
    context.attach_process(&mut process, 2);

    assert_eq!(
        bif_process_info_2(
            &[Term::small_int(1), Term::atom(Atom::STATUS)],
            &mut context
        ),
        Err(Term::atom(Atom::BADARG))
    );
    assert_eq!(
        bif_process_info_2(&[Term::pid(1), Term::small_int(1)], &mut context),
        Err(Term::atom(Atom::BADARG))
    );
    assert_eq!(
        bif_process_info_2(&[Term::pid(1), Term::atom(Atom::OK)], &mut context),
        Err(Term::atom(Atom::BADARG))
    );
}

#[test]
fn process_info_1_returns_all_supported_items_in_order() {
    let pid = 42;
    let mut process = Process::new(pid, DEFAULT_HEAP_SIZE);
    let mut context = ProcessContext::new();
    context.set_process_info_facility(Some(Arc::new(
        MockProcessInfoFacility::with_complete_process(pid),
    )));
    context.attach_process(&mut process, 1);

    let result = bif_process_info_1(&[Term::pid(pid)], &mut context).expect("process_info/1");
    let items = list_to_vec(result);
    assert_eq!(items.len(), DEFAULT_PROCESS_INFO_ITEMS.len());

    for (tuple_term, item) in items.into_iter().zip(DEFAULT_PROCESS_INFO_ITEMS.iter()) {
        let tuple = Tuple::new(tuple_term).expect("{Item, Value}");
        assert_eq!(tuple.get(0), Some(Term::atom(item.atom())));
    }
}

#[test]
fn process_info_1_returns_undefined_for_missing_process() {
    let mut process = Process::new(1, DEFAULT_HEAP_SIZE);
    let mut context = ProcessContext::new();
    context.set_process_info_facility(Some(Arc::new(MockProcessInfoFacility::default())));
    context.attach_process(&mut process, 1);

    let result = bif_process_info_1(&[Term::pid(999)], &mut context).expect("missing pid");
    assert_eq!(result, Term::atom(Atom::UNDEFINED));
}

fn assert_supported_value_shape(item: ProcessInfoItem, value: Term) {
    match item {
        ProcessInfoItem::CurrentFunction => {
            let tuple = Tuple::new(value).expect("current_function tuple");
            assert_eq!(tuple.len(), 3);
            assert!(tuple.get(0).expect("module").is_atom());
            assert!(tuple.get(1).expect("function").is_atom());
            assert!(tuple.get(2).expect("arity").is_small_int());
        }
        ProcessInfoItem::HeapSize | ProcessInfoItem::MessageQueueLen => {
            assert!(value.is_small_int());
        }
        ProcessInfoItem::RegisteredName => {
            assert!(value.is_nil() || value.is_atom());
        }
        ProcessInfoItem::Status => {
            assert_eq!(value, Term::atom(Atom::WAITING));
        }
        ProcessInfoItem::TrapExit => {
            assert_eq!(value, Term::atom(Atom::TRUE));
        }
        ProcessInfoItem::Links => {
            let links = list_to_vec(value);
            assert_eq!(links, vec![Term::pid(11), Term::pid(12)]);
        }
        ProcessInfoItem::Monitors => {
            let monitors = list_to_vec(value);
            assert_eq!(monitors.len(), 1);
            let monitor = Tuple::new(monitors[0]).expect("monitor tuple");
            assert_eq!(monitor.get(0), Some(Term::atom(Atom::PROCESS)));
            assert_eq!(monitor.get(1), Some(Term::pid(13)));
        }
    }
}

fn list_to_vec(list: Term) -> Vec<Term> {
    let mut result = Vec::new();
    let mut current = list;
    while !current.is_nil() {
        let cons = Cons::new(current).expect("proper list");
        result.push(cons.head());
        current = cons.tail();
    }
    result
}
