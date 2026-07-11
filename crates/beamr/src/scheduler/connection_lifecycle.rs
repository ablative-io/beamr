//! Scheduler-side connection-event subscriber: node-death cleanup in a FIXED
//! structural order (registry purges before exit-signal delivery). One
//! composed subscriber, not several, so the cleanup ordering is sequential
//! statements — not registration-order-dependent (the replace-on-register
//! eviction hazard cannot recur one level up).

use std::sync::{Arc, Weak};

use super::{SharedState, supervision_integration};
use crate::distribution::connection_events::ConnectionEvent;

/// Register the scheduler's composed connection-event subscriber. Called once
/// from `Scheduler` construction, before the embedder can subscribe, so
/// embedder subscribers always observe post-purge, post-noconnection state
/// (INV-SCHED-FIRST). Captures `Weak<SharedState>`: the closure is stored
/// inside `ConnectionManagerInner`, which `SharedState` owns — a strong
/// capture would leak every scheduler forever (mirror
/// `register_distribution_control_handler`).
pub(super) fn register_scheduler_connection_subscriber(shared: &Arc<SharedState>) {
    // No manager to subscribe to when distribution is Disabled (spec §3.6): the
    // node-death cleanup this installs has nothing to observe.
    let Some(dist) = shared.distribution() else {
        return;
    };
    let weak: Weak<SharedState> = Arc::downgrade(shared);
    dist.connections()
        .subscribe_connection_events(move |event| {
            if let Some(shared) = weak.upgrade() {
                handle_connection_event(&shared, event);
            }
        });
    // SubscriberId intentionally discarded: scheduler-lifetime subscription.
}

/// Structural ordering — do not reorder (trap-exit handlers must observe
/// post-purge pg state):
///   1. pg purge: a trap-exit handler receiving `{'EXIT', _, noconnection}`
///      must never observe the dead node's members (purge semantics
///      unchanged from the pre-hub closure).
///   2. [seam: global-name purge — DEFERRED: `GlobalNameRegistry` is never
///      constructed in production; every `::new` site is test code. Wire only
///      after a registry lands on `SharedState`.]
///   3. [seam: dead-node control-lane cleanup — work item A.]
///   4. noconnection delivery to every local process remote-linked to the
///      node (`supervision_integration::connection_down`).
///
/// Up is reserved for work item A control-lane (re)initialization; the match
/// is exhaustive WITHOUT a `_` arm (non_exhaustive is inert in-crate; a
/// wildcard would trip unreachable_patterns under -D warnings).
pub(super) fn handle_connection_event(shared: &Arc<SharedState>, event: ConnectionEvent) {
    match event {
        ConnectionEvent::Down(down) => {
            shared.pg_registry.purge_remote_node(down.node);
            supervision_integration::connection_down(shared, down.node);
        }
        ConnectionEvent::Up(_) => {}
    }
}
