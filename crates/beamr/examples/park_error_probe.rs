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
//!     [--mode timed|untimed] [--count N] [--waiters 15] [--notifiers 2] [--timeout-ms 5] \
//!     [--notify all|one|mixed] [--notify-lock held|released|mixed] \
//!     [--outside-waiters 0] [--outside-timeout-ms 10]
//! ```
//!
//! Defaults: `--mode timed --count 100000000 --waiters 15 --notifiers 2
//! --timeout-ms 5 --notify all --notify-lock held --outside-waiters 0
//! --outside-timeout-ms 10`. Scale `--count` by the printed waits per second: the 30
//! October liminal run made at most about 1.1e9 timed waits (15 workers,
//! 200 parks a second, about 369,500 s).
//!
//! What it shows: a panic line naming the waiter and std's assert text
//! ("assertion failed: r == libc::ETIMEDOUT || r == 0" on macOS) means the
//! kernel returned an error from a wait on valid arguments in this shape,
//! outside liminal and beamr. Reaching the count with no panic means this
//! shape did not reproduce it within that many waits on that venue.
//!
//! Widened on 4 November 2026 by Artemis (beamr lead), relayed by Apollo:
//! `--notify mixed` gives the notifiers both kinds of wake, even-numbered
//! ones `notify_all` and odd-numbered ones `notify_one`, racing the waiters'
//! timeouts. The probe prints the load average and the swap line as it
//! starts and as it ends, so a run on a loaded machine says how loaded, and
//! counts the timed waits that returned on their timeout. `--notify all`
//! (the default) is the shape of the first two runs.
//!
//! Widened again the same day by Artemis, relayed by Apollo, for the wakes
//! beamr gives that the first shape did not: `--notify-lock released`
//! notifies after dropping the mutex (beamr 0.21.1's spawn path and timer
//! wakes, and main's supervision wakes, "W9: advance inside the critical
//! section, notify after it"), `held` under it (the default), and `mixed`
//! even-numbered notifiers under it and odd-numbered after it.
//! `--outside-waiters N` adds N threads waiting on the same mutex and
//! condvar for `--outside-timeout-ms` each time (default 10), as
//! `run_until_exit` waits from a thread that is not a worker; their waits
//! are counted apart from `--count`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Timed,
    Untimed,
}

/// Which wake each notifier gives.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Notify {
    /// Every notifier calls `notify_all`, as beamr's wake edge does.
    All,
    /// Every notifier calls `notify_one`.
    One,
    /// Even-numbered notifiers call `notify_all`, odd-numbered `notify_one`.
    Mixed,
}

impl Notify {
    fn one_for(self, notifier: usize) -> bool {
        match self {
            Self::All => false,
            Self::One => true,
            Self::Mixed => notifier % 2 == 1,
        }
    }
}

/// Whether each notifier notifies holding the mutex or after dropping it.
#[derive(Clone, Copy, PartialEq, Debug)]
enum NotifyLock {
    /// Under the mutex.
    Held,
    /// After dropping it.
    Released,
    /// Even-numbered notifiers under it, odd-numbered after it.
    Mixed,
}

impl NotifyLock {
    fn released_for(self, notifier: usize) -> bool {
        match self {
            Self::Held => false,
            Self::Released => true,
            Self::Mixed => notifier % 2 == 1,
        }
    }
}

struct Args {
    mode: Mode,
    count: u64,
    waiters: usize,
    notifiers: usize,
    timeout: Duration,
    notify: Notify,
    notify_lock: NotifyLock,
    outside_waiters: usize,
    outside_timeout: Duration,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        mode: Mode::Timed,
        count: 100_000_000,
        waiters: 15,
        notifiers: 2,
        timeout: Duration::from_millis(5),
        notify: Notify::All,
        notify_lock: NotifyLock::Held,
        outside_waiters: 0,
        outside_timeout: Duration::from_millis(10),
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
            "--notify" => {
                args.notify = match value.as_str() {
                    "all" => Notify::All,
                    "one" => Notify::One,
                    "mixed" => Notify::Mixed,
                    other => return Err(format!("--notify {other}: all, one or mixed")),
                }
            }
            "--notify-lock" => {
                args.notify_lock = match value.as_str() {
                    "held" => NotifyLock::Held,
                    "released" => NotifyLock::Released,
                    "mixed" => NotifyLock::Mixed,
                    other => {
                        return Err(format!("--notify-lock {other}: held, released or mixed"));
                    }
                }
            }
            "--outside-waiters" => {
                args.outside_waiters =
                    usize::try_from(number()?).map_err(|error| error.to_string())?;
            }
            "--outside-timeout-ms" => args.outside_timeout = Duration::from_millis(number()?),
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

/// The load average and the swap line, read from `sysctl` (macOS), or
/// `/proc/loadavg` and `free` elsewhere.
fn load_and_swap() -> String {
    if cfg!(target_os = "macos") {
        format!(
            "load average {}, {}",
            command_line("sysctl", &["-n", "vm.loadavg"]),
            command_line("sysctl", &["vm.swapusage"])
        )
    } else {
        format!(
            "load average {}, swap {}",
            command_line("cat", &["/proc/loadavg"]),
            command_line("free", &["-b"])
        )
    }
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
    /// A wait panicked, with the waiting thread's name and the panic's text.
    Panicked(String, String),
}

struct Shared {
    lock: Mutex<u64>,
    condvar: Condvar,
    stop: AtomicBool,
    waits: AtomicU64,
    notifies: AtomicU64,
    /// Timed waits that returned on their timeout.
    timeouts: AtomicU64,
    /// Waits made by the outside waiters, counted apart.
    outside_waits: AtomicU64,
}

/// The name of the current thread, for a panic report.
fn thread_name() -> String {
    std::thread::current()
        .name()
        .unwrap_or("an unnamed thread")
        .to_string()
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a non-text panic payload".to_string())
}

fn waiter(shared: &Shared, args: &Args, events: &mpsc::Sender<Event>) -> u64 {
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
                    Ok((guard, timeout)) => {
                        if timeout.timed_out() {
                            shared.timeouts.fetch_add(1, Ordering::Relaxed);
                        }
                        guard
                    }
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
        let _sent = events.send(Event::Panicked(thread_name(), panic_text(payload.as_ref())));
    }
    mine
}

/// A `run_until_exit`-style waiter: a thread that is not a worker, waiting
/// on the same mutex and condvar for `--outside-timeout-ms` at a time.
fn outside_waiter(shared: &Shared, args: &Args, events: &mpsc::Sender<Event>) {
    let waited = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        while !shared.stop.load(Ordering::Acquire) {
            let guard = shared
                .lock
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if shared.stop.load(Ordering::Acquire) {
                break;
            }
            let _woken = shared.condvar.wait_timeout(guard, args.outside_timeout);
            shared.outside_waits.fetch_add(1, Ordering::Relaxed);
        }
    }));
    if let Err(payload) = waited {
        let _sent = events.send(Event::Panicked(thread_name(), panic_text(payload.as_ref())));
    }
}

fn notifier(shared: &Shared, one: bool, released: bool) {
    let notify = || {
        if one {
            shared.condvar.notify_one();
        } else {
            shared.condvar.notify_all();
        }
    };
    while !shared.stop.load(Ordering::Acquire) {
        let mut guard = shared
            .lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // What beamr's wake edge advances inside its critical section.
        *guard = guard.wrapping_add(1);
        if released {
            drop(guard);
            notify();
        } else {
            notify();
            drop(guard);
        }
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
        "park_error_probe: mode {mode}, waiters {}, notifiers {} (notify {:?}, notify-lock \
         {:?}), outside waiters {} ({:?} each), count {}",
        args.waiters,
        args.notifiers,
        args.notify,
        args.notify_lock,
        args.outside_waiters,
        args.outside_timeout,
        args.count
    );
    for index in 0..args.notifiers {
        println!(
            "park_error_probe: probe-notifier-{index} calls {} {} the mutex",
            if args.notify.one_for(index) {
                "notify_one"
            } else {
                "notify_all"
            },
            if args.notify_lock.released_for(index) {
                "after dropping"
            } else {
                "holding"
            }
        );
    }
    println!("park_error_probe: START {}", load_and_swap());
    let shared = Arc::new(Shared {
        lock: Mutex::new(0),
        condvar: Condvar::new(),
        stop: AtomicBool::new(false),
        waits: AtomicU64::new(0),
        notifies: AtomicU64::new(0),
        timeouts: AtomicU64::new(0),
        outside_waits: AtomicU64::new(0),
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
                .spawn(move || waiter(&shared, &args, &events))
                .unwrap_or_else(|error| panic!("spawn waiter {index}: {error}"))
        })
        .collect();
    let outside: Vec<_> = (0..args.outside_waiters)
        .map(|index| {
            let shared = Arc::clone(&shared);
            let args = Arc::clone(&args);
            let events = events.clone();
            std::thread::Builder::new()
                .name(format!("probe-outside-{index}"))
                .spawn(move || outside_waiter(&shared, &args, &events))
                .unwrap_or_else(|error| panic!("spawn outside waiter {index}: {error}"))
        })
        .collect();
    drop(events);
    let notifiers: Vec<_> = (0..args.notifiers)
        .map(|index| {
            let shared = Arc::clone(&shared);
            let one = args.notify.one_for(index);
            let released = args.notify_lock.released_for(index);
            std::thread::Builder::new()
                .name(format!("probe-notifier-{index}"))
                .spawn(move || notifier(&shared, one, released))
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
            Event::Panicked(thread, text) => {
                panicked = Some((thread, text));
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
    for waiter in outside {
        let _joined = waiter.join();
    }
    let per_waiter: Vec<u64> = waiters
        .into_iter()
        .map(|waiter| waiter.join().unwrap_or(0))
        .collect();
    let total = shared.waits.load(Ordering::Relaxed);
    let notifies = shared.notifies.load(Ordering::Relaxed);
    let timeouts = shared.timeouts.load(Ordering::Relaxed);
    println!(
        "park_error_probe: reached {total} waits of {} in {seconds:.1} s ({:.0} waits/s), \
         {notifies} notifies ({:.0}/s); per waiter min {} max {}",
        args.count,
        total as f64 / seconds,
        notifies as f64 / seconds,
        per_waiter.iter().min().copied().unwrap_or(0),
        per_waiter.iter().max().copied().unwrap_or(0)
    );
    println!(
        "park_error_probe: {timeouts} timed waits returned on their timeout ({:.4}% of waits)",
        timeouts as f64 * 100.0 / total.max(1) as f64
    );
    println!(
        "park_error_probe: outside waiters made {} waits",
        shared.outside_waits.load(Ordering::Relaxed)
    );
    println!("park_error_probe: END {}", load_and_swap());
    match panicked {
        Some((thread, text)) => {
            println!("park_error_probe: PANICKED: {thread}: {text}");
            std::process::exit(1);
        }
        None => println!("park_error_probe: no panic"),
    }
}
