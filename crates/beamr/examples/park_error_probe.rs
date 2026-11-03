//! B-184 R1: the park's error return, reproduced outside liminal.
//!
//! Runs the park in liminal-open's shape: `--waiters` threads, each in a loop
//! taking one shared std `Mutex` and waiting on one shared std `Condvar`, for
//! `--timeout-ms` with `wait_timeout` (`--mode timed`, the park of beamr
//! 0.21.x) or with no timeout (`--mode untimed`, B-181's park of every worker
//! but worker 0), while `--notifiers` threads call `notify_all` under the
//! mutex, as beamr's wake edge does. Every waiter counts its waits.
//!
//! The run ends on the first waiter whose wait panicked (std's condvar
//! assert, caught at the waiter's top and reported with its text), or when
//! the waits of all waiters together reach `--count`. Never on a time. It
//! prints the OS, the Rust toolchain, the count reached, the wait and notify
//! rates (each tenth of the count, so the count can be scaled to a time box),
//! and whether a wait panicked.
//!
//! Run it with, on every venue:
//!
//! ```text
//! cargo run --release --example park_error_probe -- \
//!     [--mode timed|untimed] [--count N] [--waiters 15] [--notifiers 2] [--timeout-ms 5]
//! ```
//!
//! Defaults: `--mode timed --count 100000000 --waiters 15 --notifiers 2
//! --timeout-ms 5`. Scale `--count` by the printed waits per second: the 30
//! October liminal run made at most about 1.1e9 timed waits (15 workers,
//! 200 parks a second, about 369,500 s).
//!
//! What it shows: a panic line naming the waiter and std's assert text
//! ("assertion failed: r == libc::ETIMEDOUT || r == 0" on macOS) means the
//! kernel returned an error from a wait on valid arguments in this shape,
//! outside liminal and beamr. Reaching the count with no panic means this
//! shape did not reproduce it within that many waits on that venue.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Timed,
    Untimed,
}

struct Args {
    mode: Mode,
    count: u64,
    waiters: usize,
    notifiers: usize,
    timeout: Duration,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        mode: Mode::Timed,
        count: 100_000_000,
        waiters: 15,
        notifiers: 2,
        timeout: Duration::from_millis(5),
    };
    let mut given = std::env::args().skip(1);
    while let Some(flag) = given.next() {
        let value = given
            .next()
            .ok_or_else(|| format!("{flag} needs a value"))?;
        let number = || {
            value
                .parse::<u64>()
                .map_err(|error| format!("{flag} {value}: {error}"))
        };
        match flag.as_str() {
            "--mode" => {
                args.mode = match value.as_str() {
                    "timed" => Mode::Timed,
                    "untimed" => Mode::Untimed,
                    other => return Err(format!("--mode {other}: timed or untimed")),
                }
            }
            "--count" => args.count = number()?,
            "--waiters" => {
                args.waiters = usize::try_from(number()?).map_err(|error| error.to_string())?;
            }
            "--notifiers" => {
                args.notifiers = usize::try_from(number()?).map_err(|error| error.to_string())?;
            }
            "--timeout-ms" => args.timeout = Duration::from_millis(number()?),
            other => return Err(format!("unknown flag {other}")),
        }
    }
    if args.waiters == 0 || args.count == 0 {
        return Err("--waiters and --count must be above zero".to_string());
    }
    Ok(args)
}

/// One line from a command, or what failed.
fn command_line(program: &str, arguments: &[&str]) -> String {
    std::process::Command::new(program)
        .args(arguments)
        .output()
        .map_or_else(
            |error| format!("{program}: {error}"),
            |output| {
                String::from_utf8_lossy(&output.stdout)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            },
        )
}

fn os_version() -> String {
    if cfg!(target_os = "macos") {
        format!(
            "{} {} ({})",
            command_line("sw_vers", &["-productName"]),
            command_line("sw_vers", &["-productVersion"]),
            command_line("sw_vers", &["-buildVersion"])
        )
    } else {
        command_line("uname", &["-srvm"])
    }
}

/// What ends the run, sent by the waiters to the main thread.
enum Event {
    /// The total reached this tenth of the count.
    Tenth(u64),
    /// The total reached the count.
    Reached,
    /// A waiter's wait panicked, with the panic's text.
    Panicked(usize, String),
}

struct Shared {
    lock: Mutex<u64>,
    condvar: Condvar,
    stop: AtomicBool,
    waits: AtomicU64,
    notifies: AtomicU64,
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a non-text panic payload".to_string())
}

fn waiter(shared: &Shared, args: &Args, index: usize, events: &mpsc::Sender<Event>) -> u64 {
    let tenth = (args.count / 10).max(1);
    let mut mine = 0u64;
    let waited = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut guard = shared
            .lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while !shared.stop.load(Ordering::Acquire) {
            guard = match args.mode {
                Mode::Timed => match shared.condvar.wait_timeout(guard, args.timeout) {
                    Ok((guard, _timeout)) => guard,
                    Err(error) => error.into_inner().0,
                },
                Mode::Untimed => match shared.condvar.wait(guard) {
                    Ok(guard) => guard,
                    Err(error) => error.into_inner(),
                },
            };
            mine += 1;
            let total = shared.waits.fetch_add(1, Ordering::Relaxed) + 1;
            if total.is_multiple_of(tenth) {
                let _sent = events.send(Event::Tenth(total));
            }
            if total == args.count {
                let _sent = events.send(Event::Reached);
            }
        }
    }));
    if let Err(payload) = waited {
        let _sent = events.send(Event::Panicked(index, panic_text(payload.as_ref())));
    }
    mine
}

fn notifier(shared: &Shared) {
    while !shared.stop.load(Ordering::Acquire) {
        let guard = shared
            .lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        shared.condvar.notify_all();
        drop(guard);
        shared.notifies.fetch_add(1, Ordering::Relaxed);
    }
}

fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("park_error_probe: {error}");
            std::process::exit(2);
        }
    };
    let args = Arc::new(args);
    let mode = match args.mode {
        Mode::Timed => format!("timed {:?}", args.timeout),
        Mode::Untimed => "untimed".to_string(),
    };
    println!("park_error_probe: os {}", os_version());
    println!(
        "park_error_probe: rust {} (the toolchain in this directory), arch {}",
        command_line("rustc", &["-V"]),
        std::env::consts::ARCH
    );
    println!(
        "park_error_probe: mode {mode}, waiters {}, notifiers {}, count {}",
        args.waiters, args.notifiers, args.count
    );
    let shared = Arc::new(Shared {
        lock: Mutex::new(0),
        condvar: Condvar::new(),
        stop: AtomicBool::new(false),
        waits: AtomicU64::new(0),
        notifies: AtomicU64::new(0),
    });
    let (events, ending) = mpsc::channel();
    let started = Instant::now();
    let waiters: Vec<_> = (0..args.waiters)
        .map(|index| {
            let shared = Arc::clone(&shared);
            let args = Arc::clone(&args);
            let events = events.clone();
            std::thread::Builder::new()
                .name(format!("probe-waiter-{index}"))
                .spawn(move || waiter(&shared, &args, index, &events))
                .unwrap_or_else(|error| panic!("spawn waiter {index}: {error}"))
        })
        .collect();
    drop(events);
    let notifiers: Vec<_> = (0..args.notifiers)
        .map(|index| {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name(format!("probe-notifier-{index}"))
                .spawn(move || notifier(&shared))
                .unwrap_or_else(|error| panic!("spawn notifier {index}: {error}"))
        })
        .collect();

    let mut panicked = None;
    while let Ok(event) = ending.recv() {
        match event {
            Event::Tenth(total) => {
                let seconds = started.elapsed().as_secs_f64();
                println!(
                    "park_error_probe: progress {total} waits in {seconds:.1} s: {:.0} waits/s, \
                     {:.0} notifies/s",
                    total as f64 / seconds,
                    shared.notifies.load(Ordering::Relaxed) as f64 / seconds
                );
            }
            Event::Reached => break,
            Event::Panicked(index, text) => {
                panicked = Some((index, text));
                break;
            }
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    {
        let _guard = shared
            .lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        shared.stop.store(true, Ordering::Release);
        shared.condvar.notify_all();
    }
    for notifier in notifiers {
        let _joined = notifier.join();
    }
    let per_waiter: Vec<u64> = waiters
        .into_iter()
        .map(|waiter| waiter.join().unwrap_or(0))
        .collect();
    let total = shared.waits.load(Ordering::Relaxed);
    let notifies = shared.notifies.load(Ordering::Relaxed);
    println!(
        "park_error_probe: reached {total} waits of {} in {seconds:.1} s ({:.0} waits/s), \
         {notifies} notifies ({:.0}/s); per waiter min {} max {}",
        args.count,
        total as f64 / seconds,
        notifies as f64 / seconds,
        per_waiter.iter().min().copied().unwrap_or(0),
        per_waiter.iter().max().copied().unwrap_or(0)
    );
    match panicked {
        Some((index, text)) => {
            println!("park_error_probe: PANICKED: probe-waiter-{index}: {text}");
            std::process::exit(1);
        }
        None => println!("park_error_probe: no panic"),
    }
}
