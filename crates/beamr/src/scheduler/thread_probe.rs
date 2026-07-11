//! Platform-aware OS-thread probe — the ground truth the service inventory is
//! checked against (spec §5, lens Q3).
//!
//! [`process_thread_names`] enumerates the *named* OS threads live in this
//! process right now. Tests snapshot it before and after constructing a
//! scheduler and diff the two multisets: the new named threads are exactly what
//! that scheduler spawned, which is what makes assertion 6
//! (`service_inventory()` agrees with the OS) mechanical rather than
//! eyeballed. It is deliberately dependency-light — `std` on Linux, raw
//! `libc`/mach on macOS — and compiled only for tests and the `test-support`
//! feature.

use std::collections::BTreeMap;

/// Multiset of thread names: name → count. Two threads with the same OS name
/// (e.g. the three-way `beamr-io-thread-pool-*` collision, spec §5) are counted
/// separately, so a multiset — not a set — is the faithful comparison.
#[must_use]
pub fn thread_name_multiset(names: &[String]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for name in names {
        *counts.entry(name.clone()).or_insert(0) += 1;
    }
    counts
}

/// The named OS threads currently live in this process.
///
/// Unnamed threads (the process main thread, tokio's lazily-spawned blocking
/// pool) are omitted: only threads a service explicitly named are attributable,
/// and only those appear in the inventory. Name truncation is platform-defined
/// — see the Linux note below.
#[cfg(target_os = "macos")]
#[must_use]
pub fn process_thread_names() -> Vec<String> {
    use std::ffi::CStr;

    // `mach_task_self_` (the libc re-export is deprecated in favour of `mach2`,
    // which we deliberately do not depend on) and `mach_port_deallocate` (not
    // surfaced by `libc` at all) are declared directly against libSystem —
    // always linked on macOS — so the probe stays dependency-light and the send
    // rights `task_threads` hands us are released rather than leaked per call.
    unsafe extern "C" {
        static mach_task_self_: libc::mach_port_t;
        fn mach_port_deallocate(
            task: libc::mach_port_t,
            name: libc::mach_port_t,
        ) -> libc::kern_return_t;
    }

    let mut names = Vec::new();
    // SAFETY: `task_threads` writes a kernel-allocated array of thread-port
    // send rights into `act_list`/`count`; each right is released with
    // `mach_port_deallocate` and the array is freed with `vm_deallocate`.
    unsafe {
        let task = mach_task_self_;
        let mut act_list: libc::thread_act_array_t = std::ptr::null_mut();
        let mut count: libc::mach_msg_type_number_t = 0;
        // KERN_SUCCESS is 0.
        if libc::task_threads(task, &mut act_list, &mut count) != 0 {
            return names;
        }
        for index in 0..count as usize {
            let port = *act_list.add(index);
            // `pthread_t` is `uintptr_t` on macOS; 0 means no pthread mapping.
            let pthread = libc::pthread_from_mach_thread_np(port);
            if pthread != 0 {
                // MAXTHREADNAMESIZE is 64.
                let mut buffer = [0_i8; libc::MAXTHREADNAMESIZE];
                if libc::pthread_getname_np(pthread, buffer.as_mut_ptr(), buffer.len()) == 0 {
                    let raw = CStr::from_ptr(buffer.as_ptr());
                    if let Ok(name) = raw.to_str()
                        && !name.is_empty()
                    {
                        names.push(name.to_owned());
                    }
                }
            }
            let _ = mach_port_deallocate(task, port);
        }
        let _ = libc::vm_deallocate(
            task,
            act_list as libc::vm_address_t,
            count as usize * std::mem::size_of::<libc::thread_act_t>(),
        );
    }
    names
}

/// The named OS threads currently live in this process.
///
/// Linux truncates `comm` to 15 bytes, so a name like
/// `beamr-io-thread-pool-0` reads back as `beamr-io-thread`. Exact-name
/// assertions against the inventory are therefore macOS-only; the Linux path
/// still supports count-level checks.
#[cfg(target_os = "linux")]
#[must_use]
pub fn process_thread_names() -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc/self/task") else {
        return names;
    };
    for entry in entries.flatten() {
        if let Ok(name) = std::fs::read_to_string(entry.path().join("comm")) {
            let trimmed = name.trim_end_matches('\n');
            if !trimmed.is_empty() {
                names.push(trimmed.to_owned());
            }
        }
    }
    names
}

/// The named OS threads currently live in this process.
///
/// No probe implementation on this platform; callers degrade to inventory-only
/// checks.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[must_use]
pub fn process_thread_names() -> Vec<String> {
    Vec::new()
}
