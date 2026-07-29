//! Integration wall for finding 6 (fix-wave 3aecb622 leg B): a `.beam` file
//! in a `--dir` directory that fails to load must be reported on stderr —
//! named, with a reason, plus an aggregate count. The lenience (skip and
//! continue) is deliberate and stays; the silence is the defect. Runs the
//! real binary because the contract is about the process's stderr, and
//! stdout must stay byte-identical to the no-failure case (replay/record
//! transcripts must not change).

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../beamr/tests/fixtures")
        .join(name)
}

/// Unique-per-invocation fixture directory (pid + nanos), per the RAIL 6
/// audit's parallel-safety requirement for this crate's tests.
fn unique_fixture_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("beamr-cli-{label}-{}-{nanos}", std::process::id()))
}

#[test]
fn dir_load_failure_is_named_on_stderr_with_reason_and_aggregate_count() {
    let dir = unique_fixture_dir("dir-load-stderr-wall");
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::copy(
        fixture_path("hot_code/counter_v1.beam"),
        dir.join("counter_v1.beam"),
    )
    .expect("copy healthy module into --dir directory");
    std::fs::write(dir.join("corrupt.beam"), "not a valid beam file")
        .expect("write corrupt .beam fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_beamr"))
        .arg("imports")
        .arg(fixture_path("hot_code/deferred_caller.beam"))
        .arg("--dir")
        .arg(&dir)
        .output()
        .expect("spawn beamr binary");
    let _ = std::fs::remove_dir_all(&dir);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Lenience preserved: the healthy module still loads, the deferred
    // import still resolves, and the run still succeeds.
    assert!(
        output.status.success(),
        "imports should succeed despite the corrupt file; stderr: {stderr}"
    );
    assert!(
        stdout.is_empty(),
        "stdout must stay byte-identical to the no-failure case; got: {stdout}"
    );

    // Silence killed: the skipped file is named on stderr with a reason…
    assert!(
        stderr.contains("corrupt.beam"),
        "stderr must name the skipped file; got: {stderr:?}"
    );
    // …and an aggregate count closes the report.
    assert!(
        stderr.contains("skipped 1 .beam file"),
        "stderr must carry an aggregate skip count; got: {stderr:?}"
    );
}
