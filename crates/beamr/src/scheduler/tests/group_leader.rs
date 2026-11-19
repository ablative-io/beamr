//! The group leader of top-level and native root processes.

use super::*;

// ── Commit-5 round-1 pins ───────────────────────────────────────────────────

/// Round-1 major 2 (group leader): a top-level process answers to process 0 —
/// the standard-IO server — when the standard ring is Owned, and to the
/// no-such-pid sentinel when it is Disabled, so `io:*` takes the
/// `{error,noproc}` send-failure arm instead of self-queueing an io_request
/// and parking forever.
static GL_PROBE_SEEN: AtomicUsize = AtomicUsize::new(0);
static GL_PROBE_LEADER: AtomicU64 = AtomicU64::new(0);

fn gl_probe_native(
    _args: &[Term],
    context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    // Report the attached process's OWN group leader from inside the native —
    // the body slot is Executing while this runs, so an outside probe cannot
    // read it, but the context can.
    let leader = context
        .group_leader()
        .ok()
        .and_then(|term| term.as_pid())
        .unwrap_or(u64::MAX - 1);
    GL_PROBE_LEADER.store(leader, Ordering::Release);
    GL_PROBE_SEEN.fetch_add(1, Ordering::AcqRel);
    Ok(Term::atom(Atom::OK))
}

/// Round-1 major 2 (group leader): a top-level process answers to process 0 —
/// the standard-IO server — when the standard ring is Owned, and to the
/// no-such-pid sentinel when it is Disabled, so `io:*` takes the
/// `{error,noproc}` send-failure arm instead of self-queueing an io_request
/// and parking forever.
#[test]
fn top_level_group_leader_is_process_zero_when_owned_and_sentinel_when_disabled() {
    let registry = Arc::new(ModuleRegistry::new());
    let atoms = AtomTable::new();
    let name = atoms.intern("gl_probe");
    let module = native_call_module(&registry, name, gl_probe_native, None);
    GL_PROBE_SEEN.store(0, Ordering::Release);

    // Legacy/default profile: standard ring Owned, process 0 registered.
    let owned = single_thread_scheduler(&registry);
    let _owned_pid = match owned.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || GL_PROBE_SEEN.load(Ordering::Acquire) == 1);
    assert_eq!(
        GL_PROBE_LEADER.load(Ordering::Acquire),
        0,
        "top-level processes answer to process 0 when the standard ring is Owned"
    );
    // A message SEND to the group leader genuinely lands (process 0 exists
    // and has a mailbox; enqueue keys on the same process_bodies lookup the
    // io-message facility uses) — the positive control for the noproc pin.
    assert!(
        owned.enqueue_atom_message(0, Atom::OK),
        "a message to process 0 is deliverable on the Owned profile"
    );
    owned.shutdown();

    // minimal(): no standard ring, no process 0 — the leader is the sentinel,
    // and a send to it FAILS, which is exactly the io_bifs noproc arm.
    let minimal = Scheduler::with_services(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        SchedulerServices::minimal(),
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("minimal scheduler starts: {error}"));
    let _minimal_pid = match minimal.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || GL_PROBE_SEEN.load(Ordering::Acquire) == 2);
    assert_eq!(
        GL_PROBE_LEADER.load(Ordering::Acquire),
        NO_STANDARD_IO_PID,
        "with the standard ring Disabled the leader is the no-such-pid sentinel"
    );
    assert!(
        !minimal.enqueue_atom_message(NO_STANDARD_IO_PID, Atom::OK),
        "a send to the sentinel leader FAILS — io:* takes the {{error,noproc}} \
         arm before any suspension instead of parking forever"
    );
    minimal.shutdown();
}

/// Round-2 minor: the public `spawn_native` root (synthetic caller 0) is
/// seeded from `standard_io_pid` exactly like a top-level bytecode spawn —
/// process 0 when Owned, the no-such-pid sentinel when Disabled — never the
/// absent caller's own pid.
#[test]
fn spawn_native_root_leader_matches_the_standard_io_pid() {
    let registry = Arc::new(ModuleRegistry::new());

    let owned = single_thread_scheduler(&registry);
    let received = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let (factory_received, factory_slices) = (Arc::clone(&received), Arc::clone(&slices));
    let owned_pid = owned
        .spawn_native(Box::new(move || {
            Box::new(BusyPollHandler {
                received: Arc::clone(&factory_received),
                slices: Arc::clone(&factory_slices),
            })
        }))
        .unwrap_or_else(|error| panic!("spawn native: {error:?}"));
    wait_until(10_000, || {
        owned.test_group_leader(owned_pid) == Some(Term::pid(0))
    });
    owned.shutdown();

    let minimal = Scheduler::with_services(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        SchedulerServices::minimal(),
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("minimal scheduler starts: {error}"));
    let received = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let (factory_received, factory_slices) = (Arc::clone(&received), Arc::clone(&slices));
    let minimal_pid = minimal
        .spawn_native(Box::new(move || {
            Box::new(BusyPollHandler {
                received: Arc::clone(&factory_received),
                slices: Arc::clone(&factory_slices),
            })
        }))
        .unwrap_or_else(|error| panic!("spawn native: {error:?}"));
    wait_until(10_000, || {
        minimal.test_group_leader(minimal_pid) == Some(Term::pid(NO_STANDARD_IO_PID))
    });
    minimal.shutdown();
}
