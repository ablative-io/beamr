#![cfg(target_os = "linux")]

use beamr::io::{CompletionRing, IoOp, IoResult, IoUringRing};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::time::Duration;

#[test]
fn io_uring_nop_completes() {
    let ring = IoUringRing::new(8).expect("create io_uring ring");
    let op_id = ring.submit(IoOp::Nop);
    let completions = ring.poll_completions(Duration::from_secs(2));
    let completion = completions
        .into_iter()
        .find(|completion| completion.op_id == op_id)
        .expect("nop completion");
    assert!(completion.result.is_ok());
    ring.shutdown();
}

#[test]
fn io_uring_pipe_read_completes() {
    let ring = IoUringRing::new(8).expect("create io_uring ring");
    let (reader, mut writer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
    let fd = reader.as_raw_fd();
    let op_id = ring.submit(IoOp::Read {
        fd,
        buf_len: 5,
        offset: u64::MAX,
    });
    let writer_thread = std::thread::spawn(move || writer.write_all(b"beamr"));
    let completions = ring.poll_completions(Duration::from_secs(2));
    writer_thread
        .join()
        .expect("writer thread joins")
        .expect("writer writes");
    let completion = completions
        .into_iter()
        .find(|completion| completion.op_id == op_id)
        .expect("read completion");
    match completion.result.expect("read succeeds") {
        IoResult::BytesRead(count, bytes) => {
            assert_eq!(count, 5);
            assert_eq!(bytes, b"beamr");
        }
        other => panic!("unexpected completion: {other:?}"),
    }
    drop(reader);
    ring.shutdown();
}
