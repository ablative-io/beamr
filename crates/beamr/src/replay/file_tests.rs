use std::io::Write;
use std::time::Instant;

use crate::atom::Atom;
use crate::native::ExceptionClass;
use crate::replay::{
    NativeOutcome, RecordedDeliveryKind, RecordedMessageDelivery, RecordedNativeCall,
    RecordedSchedule, RecordedSelect, RecordedTimerExpiry, ReplayEvent, ReplayLog,
};
use crate::term::Term;
use crate::timer::{ExpiredTimer, TimerRef};

const MAGIC: &[u8; 8] = b"BMRRPLY\0";

#[test]
fn replay_log_save_load_round_trips_all_event_variants() {
    let path = std::env::temp_dir().join(format!(
        "beamr-replay-{}-{}.rlog",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let now = Instant::now();
    let log = ReplayLog::new(vec![
        ReplayEvent::Select(RecordedSelect {
            pid: 1,
            index: 0,
            message: Term::small_int(10),
        }),
        ReplayEvent::MessageDelivery(RecordedMessageDelivery {
            order: 2,
            kind: RecordedDeliveryKind::RuntimeMessage,
            sender_pid: None,
            receiver_pid: 3,
            sender_clock: 0,
            receiver_clock: 4,
            message: Term::atom(Atom::OK),
        }),
        ReplayEvent::Schedule(RecordedSchedule {
            pid: 3,
            scheduler_index: 0,
            reduction_budget: 100,
            reductions_consumed: 7,
        }),
        ReplayEvent::TimerExpiry(RecordedTimerExpiry {
            now,
            expired: vec![
                ExpiredTimer {
                    reference: TimerRef::from_id(9),
                    target_pid: 3,
                    message: Term::small_int(20),
                    expires_at: now,
                    kind: crate::timer::TimerKind::ReceiveTimeout,
                },
                ExpiredTimer {
                    reference: TimerRef::from_id(10),
                    target_pid: 4,
                    message: Term::small_int(21),
                    expires_at: now,
                    kind: crate::timer::TimerKind::Deliver,
                },
            ],
        }),
        ReplayEvent::NativeCall(RecordedNativeCall {
            pid: 3,
            module: Atom::MODULE,
            function: Atom::OK,
            arity: 0,
            outcome: NativeOutcome::err(Term::atom(Atom::BADARG), ExceptionClass::Error, Term::NIL),
        }),
    ]);

    log.save(&path).expect("save replay log");
    let loaded = ReplayLog::load(&path).expect("load replay log");
    let _ = std::fs::remove_file(path);

    assert_eq!(loaded.len(), log.len());
    assert_eq!(loaded.events()[0], log.events()[0]);
    assert_eq!(loaded.events()[1], log.events()[1]);
    assert_eq!(loaded.events()[2], log.events()[2]);
    assert_timer_fields_round_trip(&loaded.events()[3], &log.events()[3]);
    assert_eq!(loaded.events()[4], log.events()[4]);
}

#[test]
fn replay_log_save_load_preserves_cli_transcript() {
    let path = std::env::temp_dir().join(format!(
        "beamr-replay-cli-{}-{}.rlog",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let log = ReplayLog::with_cli_result(Vec::new(), "ok\n".to_owned(), 7);

    log.save(&path).expect("save replay log with transcript");
    let loaded = ReplayLog::load(&path).expect("load replay log with transcript");
    let _ = std::fs::remove_file(path);
    let result = loaded.cli_result().expect("transcript is present");

    assert_eq!(result.output(), "ok\n");
    assert_eq!(result.exit_code(), 7);
}

#[test]
fn replay_log_load_rejects_unknown_header_flags() {
    let path = temp_replay_path("unknown-flags");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&ReplayLog::format_version().to_le_bytes());
    bytes.push(0x80);
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    std::fs::File::create(&path)
        .and_then(|mut file| file.write_all(&bytes))
        .expect("write malformed replay log");

    let error = ReplayLog::load(&path).expect_err("unknown flags should be rejected");
    let _ = std::fs::remove_file(path);

    assert!(error.to_string().contains("unknown replay log flags"));
}

fn assert_timer_fields_round_trip(loaded: &ReplayEvent, original: &ReplayEvent) {
    match (loaded, original) {
        (ReplayEvent::TimerExpiry(loaded), ReplayEvent::TimerExpiry(original)) => {
            assert_eq!(loaded.expired.len(), original.expired.len());
            for (loaded_timer, original_timer) in loaded.expired.iter().zip(original.expired.iter())
            {
                assert_eq!(loaded_timer.reference, original_timer.reference);
                assert_eq!(loaded_timer.target_pid, original_timer.target_pid);
                assert_eq!(loaded_timer.message, original_timer.message);
                // The timer-kind discriminant must survive the round-trip:
                // misrouting a Deliver timer to the receive-timeout path (or
                // vice versa) during replay would break determinism.
                assert_eq!(loaded_timer.kind, original_timer.kind);
            }
        }
        other => panic!("unexpected timer events: {other:?}"),
    }
}

fn temp_replay_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "beamr-replay-{label}-{}-{}.rlog",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

/// THE WALL for the decoded scratch heaps' resource discipline: a loaded
/// replay log whose events carry refcounted binaries must release those
/// Arcs when the log drops. The scratch heaps never run a GC, so the
/// release walk at drop is the ONLY release those allocations ever get —
/// without it every large decoded binary leaks for the process lifetime.
#[test]
fn decoded_proc_bin_arcs_release_when_the_log_drops() {
    use crate::replay::RecordedSelect;
    use crate::term::boxed::ProcBin;
    use crate::term::shared_binary::{REFC_BINARY_THRESHOLD, SharedBinary, write_proc_bin};

    let path = temp_replay_path("procbin-release");
    // Comfortably past the inline threshold so the decode allocates a
    // ProcBin (leaked-Arc layout), never an inline binary.
    let bytes = vec![0xAB_u8; REFC_BINARY_THRESHOLD * 4];
    let source = SharedBinary::new(bytes);
    let mut proc_bin_words = [0_u64; 3];
    let message = write_proc_bin(&mut proc_bin_words, &source).expect("proc bin writes");

    ReplayLog::new(vec![ReplayEvent::Select(RecordedSelect {
        pid: 1,
        index: 0,
        message,
    })])
    .save(&path)
    .expect("replay log saves");

    let loaded = ReplayLog::load(&path).expect("replay log loads");
    let ReplayEvent::Select(select) = &loaded.events()[0] else {
        panic!("loaded log carries the recorded select event");
    };
    let decoded = ProcBin::new(select.message).expect("decoded message is a ProcBin");
    let handle = decoded.shared_binary();
    assert_eq!(handle.as_bytes()[0], 0xAB, "decoded bytes round-tripped");
    assert_eq!(
        handle.ref_count(),
        2,
        "the scratch heap retains one Arc, this test the other"
    );

    drop(loaded);
    assert_eq!(
        handle.ref_count(),
        1,
        "dropping the log releases the scratch heap's Arc — a survivor is the leak"
    );
    std::fs::remove_file(&path).ok();
}
