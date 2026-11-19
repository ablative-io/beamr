//! Tests for the inbound link-control apply path and the DC-4 kind-gating:
//! wire LINK/EXIT/EXIT2 frames through the real dispatcher and read loop,
//! the LINK-to-dead noproc reply over a real socket, the D9 Executing-arm
//! regression, and the ManualDisconnect-from-a-plain-thread backstop.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::connection_lifecycle::register_scheduler_connection_subscriber;
use super::connection_lifecycle_tests::{
    assert_noconnection_exit, install_peer, mailbox_message_count, socket_pair,
};
use super::remote_supervision::{RemoteExitKind, apply_inbound_link};
use super::supervision_integration::{
    SchedulerDistributionControlFacility, SchedulerDistributionSendFacility, establish_remote_link,
    register_distribution_control_handler, remove_remote_link,
};
use super::supervision_tests::{
    add_remote_link, insert_process, is_alive, make_executing, make_shared_state,
    make_shared_state_with_dist_sender, make_shared_state_with_dist_sender_named,
    read_mailbox_tuple, set_trap_exit,
};
use super::*;
use crate::atom::Atom;
use crate::distribution::control::{
    ControlError, ControlMessage, decode_control, encode_send_frame, split_frame,
};
use crate::distribution::control_link::{
    ControlOp, ControlSinks, dispatch_frame, encode_exit_frame, encode_link_frame,
};
use crate::distribution::remote_link::{DistributionControlFacility, RemoteLinkError};
use crate::distribution::sender::ControlOutbound;
use crate::ets::{EtsTableMetadata, EtsTableType, Protection};
use crate::process::{ExitReason, RemotePid};
use crate::scheduler::execution::{
    cleanup_exited_process, cleanup_if_tombstoned_after_store, store_runnable_process,
};
use crate::term::Term;
use crate::term::boxed::{ExternalPid, write_external_pid};

mod connection_down;
mod control_lane;
mod executing_links;
mod exit_reasons;
mod facility;
mod frame_validation;
mod inbound_links;
mod wire_exits;

/// Route a raw control frame through `dispatch_frame` with the exact sink
/// bundle `register_distribution_control_handler` wires up: the scheduler
/// facility on every sink, the authenticated `origin`, and the local node.
fn dispatch_wire_frame(
    shared: &Arc<SharedState>,
    origin: Atom,
    frame: &[u8],
) -> Result<bool, ControlError> {
    let facility = SchedulerDistributionSendFacility {
        shared: Arc::clone(shared),
    };
    let sinks = ControlSinks {
        delivery: &facility,
        registry: Some(&facility),
        pg: Some(&facility),
        links: Some(&facility),
        origin_node: Some(origin),
        local_node: Some(shared.local_node.name),
    };
    let (control, payload) = split_frame(frame).expect("frame splits");
    dispatch_frame(control, payload, &shared.atom_table, &sinks)
}

fn remote_links_of(shared: &SharedState, pid: u64) -> Vec<RemotePid> {
    let entry = shared
        .process_bodies
        .get(&pid)
        .unwrap_or_else(|| panic!("process {pid} exists"));
    let slot = lock_or_recover(&entry);
    match &*slot {
        ProcessSlot::Present(ScheduledProcess(process)) => process.remote_links().to_vec(),
        ProcessSlot::Executing(metadata) => metadata.relations.remote_links.as_slice().to_vec(),
        ProcessSlot::Absent => Vec::new(),
    }
}

fn add_local_link(shared: &SharedState, a: u64, b: u64) {
    for (pid, linked) in [(a, b), (b, a)] {
        let entry = shared
            .process_bodies
            .get(&pid)
            .unwrap_or_else(|| panic!("process {pid} exists"));
        let mut slot = lock_or_recover(&entry);
        let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
            panic!("process {pid} is present");
        };
        process.add_link(linked);
    }
}

/// Read one `[control_len|payload_len]`-framed message from the peer side of
/// a test socket; returns `(control, payload)` ETF bytes.
pub(super) fn read_peer_frame(peer: &mut std::net::TcpStream) -> (Vec<u8>, Vec<u8>) {
    peer.set_read_timeout(Some(Duration::from_secs(10)))
        .expect("peer read timeout");
    let mut header = [0_u8; 8];
    peer.read_exact(&mut header).expect("frame header");
    let control_len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
    let payload_len = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
    let mut control = vec![0_u8; control_len];
    peer.read_exact(&mut control).expect("frame control");
    let mut payload = vec![0_u8; payload_len];
    peer.read_exact(&mut payload).expect("frame payload");
    (control, payload)
}

/// All mailbox messages of a Present process, in mailbox order.
fn mailbox_terms(shared: &SharedState, pid: u64) -> Vec<Term> {
    let entry = shared
        .process_bodies
        .get(&pid)
        .unwrap_or_else(|| panic!("process {pid} exists"));
    let mut slot = lock_or_recover(&entry);
    let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
        panic!("process {pid} is present");
    };
    process.mailbox_mut().drain_arrival();
    process.mailbox().scan_iter().copied().collect()
}

fn control_facility(shared: &Arc<SharedState>) -> SchedulerDistributionControlFacility {
    SchedulerDistributionControlFacility {
        shared: Arc::clone(shared),
    }
}
