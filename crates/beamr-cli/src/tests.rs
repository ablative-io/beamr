use super::args::{parse_args, parse_entry};
use super::{CliError, CliSuccess, Command, EntryPoint, run_cli};
use beamr::error::{ExecError, LoadError};
use beamr::replay::ReplayLog;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn parses_help_flags() {
    assert_eq!(parse_args(["--help"]).expect("help parses"), Command::Help);
    assert_eq!(parse_args(["-h"]).expect("help parses"), Command::Help);
}

#[test]
fn parses_version_flags() {
    assert_eq!(
        parse_args(["--version"]).expect("version parses"),
        Command::Version
    );
    assert_eq!(
        parse_args(["-V"]).expect("version parses"),
        Command::Version
    );
}

#[test]
fn parses_beam_file_only_as_run_without_entry() {
    assert_eq!(
        parse_args(["hello.beam"]).expect("run parses"),
        Command::Run {
            path: "hello.beam".into(),
            entry: None,
            args: Vec::new(),
            dirs: Vec::new(),
        }
    );
}

#[test]
fn parses_beam_file_and_entry_as_run_with_entry() {
    assert_eq!(
        parse_args(["hello.beam", "hello:main/0"]).expect("run with entry parses"),
        Command::Run {
            path: "hello.beam".into(),
            entry: Some(EntryPoint {
                module: "hello".into(),
                function: "main".into(),
                arity: 0,
            }),
            args: Vec::new(),
            dirs: Vec::new(),
        }
    );
}

#[test]
fn parses_entry_flag_and_runtime_args() {
    assert_eq!(
        parse_args(["hello.beam", "--entry", "hello:add/2", "--", "17", "25"])
            .expect("run with --entry and args parses"),
        Command::Run {
            path: "hello.beam".into(),
            entry: Some(EntryPoint {
                module: "hello".into(),
                function: "add".into(),
                arity: 2,
            }),
            args: vec!["17".into(), "25".into()],
            dirs: Vec::new(),
        }
    );
}

#[test]
fn parses_record_command_with_log_and_runtime_args() {
    assert_eq!(
        parse_args([
            "record",
            "hello.beam",
            "--entry",
            "hello:add/2",
            "--log",
            "run.rlog",
            "--",
            "17",
            "25"
        ])
        .expect("record parses"),
        Command::Record {
            path: "hello.beam".into(),
            entry: EntryPoint {
                module: "hello".into(),
                function: "add".into(),
                arity: 2,
            },
            log: "run.rlog".into(),
            args: vec!["17".into(), "25".into()],
            dirs: Vec::new(),
        }
    );
}

#[test]
fn parses_replay_command() {
    assert_eq!(
        parse_args(["replay", "run.rlog"]).expect("replay parses"),
        Command::Replay {
            log: "run.rlog".into(),
        }
    );
}

#[test]
fn rejects_log_flag_outside_record_command() {
    let error =
        parse_args(["hello.beam", "--log", "run.rlog"]).expect_err("--log should be record-only");

    assert!(matches!(error, CliError::Usage(_)));
    assert!(
        error
            .to_string()
            .contains("--log is only supported with record")
    );
}

#[test]
fn rejects_record_log_without_value() {
    let error = parse_args(["record", "hello.beam", "--entry", "hello:main/0", "--log"])
        .expect_err("--log without value should fail");

    assert!(matches!(error, CliError::MissingLogValue(_)));
}

#[test]
fn parses_imports_command() {
    assert_eq!(
        parse_args(["imports", "hello.beam"]).expect("imports parses"),
        Command::Imports {
            path: "hello.beam".into(),
            dirs: Vec::new(),
        }
    );
}

#[test]
fn parses_imports_command_carrying_dir_flags() {
    assert_eq!(
        parse_args([
            "imports",
            "hello.beam",
            "--dir",
            "/tmp/a",
            "--dir",
            "/tmp/b"
        ])
        .expect("imports with --dir parses"),
        Command::Imports {
            path: "hello.beam".into(),
            dirs: vec!["/tmp/a".into(), "/tmp/b".into()],
        }
    );
}

#[test]
fn parses_compile_command_with_verbose() {
    assert_eq!(
        parse_args(["compile", "/tmp/beams", "--verbose"]).expect("compile verbose parses"),
        Command::Compile {
            dir: "/tmp/beams".into(),
            verbose: true,
        }
    );
    assert_eq!(
        parse_args(["compile", "/tmp/beams"]).expect("compile parses"),
        Command::Compile {
            dir: "/tmp/beams".into(),
            verbose: false,
        }
    );
}

#[test]
fn rejects_non_beam_path() {
    let error = parse_args(["hello.txt"]).expect_err("non-beam path should fail");

    assert!(matches!(error, CliError::InvalidBeamPath(_)));
    assert!(error.to_string().contains(".beam"));
}

#[test]
fn validates_entry_format() {
    assert_eq!(
        parse_entry("hello:main/255").expect("valid entry parses"),
        EntryPoint {
            module: "hello".into(),
            function: "main".into(),
            arity: 255,
        }
    );

    for invalid in [
        "bad-entry",
        ":main/0",
        "hello:/0",
        "hel/lo:main/0",
        "hello:main/+1",
        "hello:main/",
        "hello:main/256",
        "hello:main/not-a-number",
        "hello:main/0/1",
        "hello:main:again/0",
    ] {
        assert!(
            matches!(parse_entry(invalid), Err(CliError::InvalidEntry(_))),
            "{invalid} should be rejected"
        );
    }
}

#[test]
fn rejects_invalid_run_entry() {
    let error = parse_args(["hello.beam", "bad-entry"]).expect_err("invalid entry should fail");

    assert!(matches!(error, CliError::InvalidEntry(_)));
    assert!(error.to_string().contains("invalid entry point"));
}

#[test]
fn rejects_unknown_flag() {
    let error = parse_args(["--unknown"]).expect_err("unknown flag should fail");

    assert!(matches!(&error, CliError::UnknownFlag(flag) if flag == "--unknown"));
    assert!(error.to_string().contains("--unknown"));
}

#[test]
fn rejects_unknown_flag_after_imports_as_flag() {
    let error = parse_args(["imports", "--unknown"])
        .expect_err("unknown flag should be detected before path validation");

    assert!(matches!(&error, CliError::UnknownFlag(flag) if flag == "--unknown"));
}

#[test]
fn error_display_formats_io_load_and_exec_errors() {
    let io_error = CliError::Io {
        path: "missing.beam".into(),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory"),
    };
    assert_eq!(
        io_error.to_string(),
        "cannot read 'missing.beam': No such file or directory"
    );
    assert_eq!(io_error.exit_code(), 2);

    let load_error = CliError::Load(LoadError::InvalidFormat);
    assert_eq!(load_error.to_string(), "load: invalid BEAM file format");
    assert_eq!(load_error.exit_code(), 2);

    let exec_error = CliError::Exec(ExecError::Badarith.to_string());
    assert_eq!(exec_error.to_string(), "exec: arithmetic operation failed");
    assert_eq!(exec_error.exit_code(), 1);
}

#[test]
fn malformed_beam_bytes_return_load_error_without_panicking() {
    let path = write_temp_beam("not a valid beam file");

    let error = run_cli([path.to_string_lossy().into_owned()])
        .expect_err("garbage .beam bytes should fail as a load error");

    assert!(matches!(error, CliError::Load(_)));
    assert_eq!(error.exit_code(), 2);
    let _ = std::fs::remove_file(path);
}

/// `record` runs the module live and writes a loadable log carrying the run's
/// transcript. Replaying that log must NOT hand the transcript back as the
/// answer: this test previously asserted `replayed == recorded`, which passed
/// only because `replay` reprinted the recorded stdout without replaying
/// anything. The recorded transcript is the thing compared against, never the
/// thing printed.
#[test]
fn record_writes_a_loadable_log_and_replay_refuses_to_reprint_it() {
    let fixture = fixture_path("hello.beam");
    let log_path = temp_replay_log_path("success");

    let recorded = run_cli([
        "record".to_owned(),
        fixture.to_string_lossy().into_owned(),
        "--entry".to_owned(),
        "hello:main/0".to_owned(),
        "--log".to_owned(),
        log_path.to_string_lossy().into_owned(),
    ])
    .expect("record fixture run");

    let CliSuccess::WithExitCode {
        stdout: recorded_stdout,
        exit_code: 0,
    } = recorded
    else {
        panic!("record should report the live run's stdout and exit code: {recorded:?}");
    };
    assert!(
        !recorded_stdout.is_empty(),
        "the live run should produce output"
    );

    let loaded = ReplayLog::load(&log_path).expect("recorded log should load");
    let transcript = loaded
        .cli_result()
        .expect("record should store the CLI transcript");
    assert_eq!(transcript.output(), recorded_stdout);
    assert_eq!(transcript.exit_code(), 0);
    drop(loaded);

    let outcome = run_cli(["replay".to_owned(), log_path.to_string_lossy().into_owned()]);
    let _ = std::fs::remove_file(&log_path);

    let error = match outcome {
        Ok(success) => panic!("replay must not reprint the recorded transcript: {success:?}"),
        Err(error) => error,
    };
    assert_ne!(error.exit_code(), 0);
    assert!(
        !error.to_string().contains(recorded_stdout.trim()),
        "the refusal must not smuggle the recorded transcript back out: {error}"
    );
}

/// LOAD-BEARING WALL (task 3aecb622).
///
/// **A log recorded against build A, replayed after the behaviour changes,
/// MUST FAIL.**
///
/// `beamr replay` is a reproduction, not a transcript player. The recorded
/// `cli_result` is the claim to be CHECKED against the replayed run — never
/// the answer to be printed. Here a genuine recording is taken and only its
/// recorded transcript is rewritten, standing in for "the build changed
/// underneath the log": the events still describe run A, the transcript no
/// longer matches what this build produces. Replay must refuse loudly.
///
/// Before the fix this passes GREEN by printing the rewritten transcript and
/// exiting 0 — a log recorded against a working build still reports success
/// after the build is broken. That is the defect this task removes.
#[test]
fn replay_of_a_log_recorded_against_different_behaviour_fails() {
    let fixture = fixture_path("hello.beam");
    let log_path = temp_replay_log_path("build-a-drift");

    let _recorded = run_cli([
        "record".to_owned(),
        fixture.to_string_lossy().into_owned(),
        "--entry".to_owned(),
        "hello:main/0".to_owned(),
        "--log".to_owned(),
        log_path.to_string_lossy().into_owned(),
    ])
    .expect("record fixture run");

    // Rewrite ONLY the transcript, preserving the recorded events. `loaded`
    // must outlive the `save` below: cloned events' boxed terms point into the
    // loaded log's decoded heaps, which are released when it drops.
    let loaded = ReplayLog::load(&log_path).expect("recorded log should load");
    let tampered = ReplayLog::with_cli_result(
        loaded.events().to_vec(),
        "OUTPUT FROM BUILD A THAT THIS BUILD DOES NOT PRODUCE\n".to_owned(),
        0,
    );
    tampered
        .save(&log_path)
        .expect("rewritten replay log should be writable");
    drop(loaded);

    let outcome = run_cli(["replay".to_owned(), log_path.to_string_lossy().into_owned()]);
    let _ = std::fs::remove_file(&log_path);

    let error = match outcome {
        Ok(success) => panic!(
            "replay must not succeed on a run it cannot reproduce; \
             it reprinted the recorded transcript instead: {success:?}"
        ),
        Err(error) => error,
    };
    assert_ne!(
        error.exit_code(),
        0,
        "a replay divergence must exit non-zero, got 0 from {error}"
    );
    assert!(
        !error.to_string().contains("OUTPUT FROM BUILD A"),
        "replay must never emit the recorded transcript as its answer: {error}"
    );
}

/// R4 wall: a log carrying a transcript but no drivable events cannot be
/// reproduced, so replay must fail loudly and name the divergence rather than
/// falling back to printing the stored stdout.
#[test]
fn replay_of_a_log_with_no_drivable_events_fails_loudly() {
    let log_path = temp_replay_log_path("no-events");
    let transcript = "STORED TRANSCRIPT THAT MUST NEVER BE PRINTED AS THE ANSWER\n";
    ReplayLog::with_cli_result(Vec::new(), transcript.to_owned(), 0)
        .save(&log_path)
        .expect("replay log fixture should be writable");

    let outcome = run_cli(["replay".to_owned(), log_path.to_string_lossy().into_owned()]);
    let _ = std::fs::remove_file(&log_path);

    let error = match outcome {
        Ok(success) => panic!("replay must not succeed with nothing to drive; got {success:?}"),
        Err(error) => error,
    };
    assert_ne!(error.exit_code(), 0, "divergence must exit non-zero");
    let message = error.to_string();
    assert!(
        !message.contains("STORED TRANSCRIPT"),
        "replay must never degrade into a transcript reprint: {message}"
    );
    assert!(
        message.contains("replay"),
        "the failure must name the divergence: {message}"
    );
}

/// R5 wall: a replay reconstructs a recorded run, so its module context comes
/// from the RECORDING and never from a flag supplied at replay time. Accepting
/// `--dir` would be a supported way to replay against different code than was
/// recorded. Refuse it, mirroring the `compile` guard.
#[test]
fn replay_refuses_dir_flag() {
    let error = parse_args(["replay", "run.rlog", "--dir", "/tmp/beams"])
        .expect_err("--dir must be refused for replay");

    assert!(
        error
            .to_string()
            .contains("--dir is not supported with replay"),
        "the refusal must name the flag and the command: {error}"
    );
}

#[test]
fn imports_report_for_fixture_is_informational_and_omits_gate1_bifs() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../beamr/tests/fixtures/hello.beam")
        .to_string_lossy()
        .into_owned();
    let result =
        run_cli(["imports", fixture.as_str()]).expect("imports report should be informational");

    let CliSuccess::Stdout(output) = result else {
        panic!("imports should return stdout-only success");
    };
    assert!(!output.contains("erlang:get_module_info/1"));
    assert!(!output.contains("erlang:get_module_info/2"));
    assert!(!output.contains("erlang:display/1"));
    assert!(output.lines().all(|line| {
        let Some((_module, function_and_arity)) = line.split_once(':') else {
            return false;
        };
        function_and_arity.split_once('/').is_some()
    }));
}

#[test]
fn parses_dir_flag_with_beam_file() {
    assert_eq!(
        parse_args(["hello.beam", "--dir", "/tmp/beams"]).expect("--dir with beam file parses"),
        Command::Run {
            path: "hello.beam".into(),
            entry: None,
            args: Vec::new(),
            dirs: vec!["/tmp/beams".into()],
        }
    );
}

#[test]
fn parses_multiple_dir_flags() {
    assert_eq!(
        parse_args([
            "hello.beam",
            "--dir",
            "/tmp/a",
            "--dir",
            "/tmp/b",
            "hello:main/0"
        ])
        .expect("multiple --dir flags parse"),
        Command::Run {
            path: "hello.beam".into(),
            entry: Some(EntryPoint {
                module: "hello".into(),
                function: "main".into(),
                arity: 0,
            }),
            args: Vec::new(),
            dirs: vec!["/tmp/a".into(), "/tmp/b".into()],
        }
    );
}

#[test]
fn rejects_dir_without_value() {
    let error = parse_args(["hello.beam", "--dir"]).expect_err("--dir without value should fail");

    assert!(matches!(error, CliError::MissingDirValue(_)));
}

// WALL (finding 5, fix-wave 3aecb622): `beamr imports` must honor --dir.
// deferred_caller.beam's only deferred import is counter:version/0; with the
// counter module supplied via --dir the report must be empty. Against code
// that drops --dir on the imports path, the report still lists the import
// and this test is red.
#[test]
fn imports_with_dir_satisfying_deferred_import_prints_empty_report() {
    let dir = temp_fixture_dir("imports-dir-wall");
    std::fs::create_dir_all(&dir).expect("create temp fixture dir");
    std::fs::copy(
        fixture_path("hot_code/counter_v1.beam"),
        dir.join("counter_v1.beam"),
    )
    .expect("copy counter fixture into --dir directory");
    let caller = fixture_path("hot_code/deferred_caller.beam");

    let result = run_cli([
        "imports".to_owned(),
        caller.to_string_lossy().into_owned(),
        "--dir".to_owned(),
        dir.to_string_lossy().into_owned(),
    ])
    .expect("imports with --dir should succeed");
    let _ = std::fs::remove_dir_all(&dir);

    let CliSuccess::Stdout(output) = result else {
        panic!("imports should return stdout-only success");
    };
    assert!(
        output.is_empty(),
        "imports must print an empty report when --dir satisfies every \
         deferred import; unresolved imports still reported:\n{output}"
    );
}

// WALL (finding 5, fix-wave 3aecb622): compile never consumed --dir; the
// defect is the silent accept. It must be refused with a usage error, not
// implemented. Against code that silently drops the flag, parse_args
// returns Ok and this test is red.
#[test]
fn compile_refuses_dir_flag_instead_of_silently_dropping_it() {
    let error = parse_args(["compile", "/tmp/beams", "--dir", "/tmp/extra"])
        .expect_err("compile must refuse --dir rather than silently dropping it");

    assert!(matches!(error, CliError::Usage(_)));
    assert!(
        error.to_string().contains("--dir"),
        "usage error must name the refused flag; got: {error}"
    );
}

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../beamr/tests/fixtures")
        .join(name)
}

fn temp_replay_log_path(label: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    path.push(format!(
        "beamr-cli-replay-{label}-{}-{nanos}.rlog",
        std::process::id()
    ));
    path
}

/// Unique-per-invocation fixture directory (pid + nanos), per the RAIL 6
/// audit's requirement that this crate's tests stay parallel-safe: no shared
/// fixture directories, no runtime mutation of a path another test can see.
fn temp_fixture_dir(label: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("beamr-cli-{label}-{}-{nanos}", std::process::id()))
}

fn write_temp_beam(contents: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    path.push(format!("beamr-cli-test-{nanos}.beam"));
    std::fs::write(&path, contents).expect("temp .beam fixture should be writable");
    path
}
