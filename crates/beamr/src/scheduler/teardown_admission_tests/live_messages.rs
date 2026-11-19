//! Sender admission for live message targets, measured by allocation and
//! clone observers.

use super::live_harness::{LIVE_COMPONENTS, LiveMeasurement};
use super::*;

struct LiveAllocator;

#[global_allocator]
static LIVE_ALLOCATOR: LiveAllocator = LiveAllocator;

// SAFETY: allocation and deallocation delegate to the same system allocator.
unsafe impl std::alloc::GlobalAlloc for LiveAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        record_live_operation(4);
        // SAFETY: the caller supplies the allocator contract's valid layout.
        unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        record_live_operation(4);
        // SAFETY: the caller supplies the allocator contract's valid layout.
        unsafe { std::alloc::GlobalAlloc::alloc_zeroed(&std::alloc::System, layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        // SAFETY: the allocation originated in this same system allocator.
        unsafe { std::alloc::GlobalAlloc::dealloc(&std::alloc::System, pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        record_live_operation(4);
        // SAFETY: the original allocation and replacement size satisfy the caller's contract.
        unsafe { std::alloc::GlobalAlloc::realloc(&std::alloc::System, pointer, layout, size) }
    }
}

fn count_live_messages(mode: LiveTargetMode, count: usize) -> [usize; 11] {
    count_live_messages_with_setup(mode, count, |_| ()).0
}

#[test]
fn sender_admission_count_vectors_for_present_executing_and_result_gated_targets() {
    println!("B179_COMPONENTS {LIVE_COMPONENTS:?}");
    for mode in [
        LiveTargetMode::Present,
        LiveTargetMode::Executing,
        LiveTargetMode::ResultGated,
    ] {
        for count in [128, 256] {
            let vector = count_live_messages(mode, count);
            assert_eq!(vector[7], count);
            assert_eq!(vector[8], count);
        }
    }
}

#[test]
fn live_message_allocation_and_clone_observers_have_positive_controls() {
    let mut heap = crate::process::heap::Heap::new(32);
    let mut mailbox = crate::mailbox::Mailbox::new();
    let measurement = LiveMeasurement::start();
    let allocation = Box::new([0_u64; 1024]);
    std::hint::black_box(&allocation);
    assert!(heap.alloc_slice(2).is_ok());
    mailbox.push_owned(Term::atom(Atom::OK));
    drop(std::hint::black_box(heap.clone()));
    drop(std::hint::black_box(mailbox.clone()));
    drop(allocation);
    let vector = measurement.finish();
    assert!(vector[4] > 0);
    assert_eq!(vector[5], 1);
    assert_eq!(vector[7], 1);
    assert_eq!(vector[10], 2);
}
