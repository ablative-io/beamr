//! WPORT-4 R3: subscription/snapshot acceptance walls for the browser
//! connection-event hub, mirroring the native precedent corpus
//! (`crates/beamr/src/distribution/connection_events_tests.rs`) at the wasm
//! seam. Every wall is fully SYNCHRONOUS — no await, no timer, no poll —
//! which is itself the told-not-polled proof: every observation is made
//! immediately after the ingress call that produced it (NO-POLLING, R11).

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::Function;
use serde_json::Value;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_test::wasm_bindgen_test;

use crate::WasmVm;

/// Parse one delivered event payload (a JSON string) into a `Value`.
fn parse_event(event: &JsValue) -> Value {
    serde_json::from_str(
        event
            .as_string()
            .expect("hub events are delivered as JSON strings")
            .as_str(),
    )
    .expect("hub event JSON parses")
}

/// A recording subscriber: returns the shared log and the JS callback that
/// appends every delivered event to it.
fn event_log() -> (Rc<RefCell<Vec<Value>>>, Function) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let callback = {
        let log = Rc::clone(&log);
        Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
            log.borrow_mut().push(parse_event(&event));
        })
        .into_js_value()
        .unchecked_into::<Function>()
    };
    (log, callback)
}

/// Build the expected Up event value.
fn up(node: &str, generation: u64, peer_creation: u32) -> Value {
    serde_json::json!({
        "type": "up",
        "node": node,
        "generation": generation,
        "peer_creation": peer_creation,
    })
}

/// Build the expected Down event value.
fn down(node: &str, generation: u64, reason: &str) -> Value {
    serde_json::json!({
        "type": "down",
        "node": node,
        "generation": generation,
        "reason": reason,
    })
}

/// Assert an error is the hub's loud typed protocol error of `kind`.
fn assert_protocol_error(error: &JsValue, kind: &str) {
    let error = error
        .dyn_ref::<js_sys::Error>()
        .expect("protocol violations raise a typed js_sys::Error");
    assert_eq!(
        error.name(),
        "ConnectionEventProtocolError",
        "the error carries the hub's typed name"
    );
    let message = String::from(error.message());
    assert!(
        message.starts_with(kind),
        "the message names the violation kind {kind:?}: {message}"
    );
}

/// Verify one subscriber's per-node stream: starts with an Up, strictly
/// alternates Up/Down, generations dense (a gap is a missed session, a repeat
/// a double-see), and only the two vocabulary shapes appear — the wasm mirror
/// of the native churn verifier.
fn assert_alternating_dense(events: &[Value], node: &str) {
    assert!(
        matches!(
            events.first().map(|event| event["type"].as_str()),
            Some(Some("up"))
        ),
        "the stream must start with an Up (synthetic catch-up or the next real session)"
    );
    let mut open: Option<u64> = None;
    let mut last_closed: Option<u64> = None;
    for event in events {
        assert_eq!(event["node"], node);
        let generation = event["generation"].as_u64().expect("generation is numeric");
        match event["type"].as_str() {
            Some("up") => {
                assert_eq!(
                    open, None,
                    "Up({generation}) delivered while a generation is open (double-see)"
                );
                if let Some(closed) = last_closed {
                    assert_eq!(
                        generation,
                        closed + 1,
                        "generations are dense: a gap is a missed session"
                    );
                }
                open = Some(generation);
            }
            Some("down") => {
                assert_eq!(
                    open,
                    Some(generation),
                    "Down({generation}) must close the currently open generation"
                );
                last_closed = Some(generation);
                open = None;
            }
            other => panic!("no third event variant may exist: {other:?}"),
        }
    }
}

#[wasm_bindgen_test]
fn subscription_snapshot_delivers_synthetic_ups_then_live_events_in_shared_vocabulary() {
    let vm = WasmVm::new().expect("VM constructs");
    vm.connection_up("alpha@browser", 7)
        .expect("alpha comes up");
    vm.connection_up("beta@browser", 9).expect("beta comes up");

    // A bare double-Up without an intervening Down is the loud typed error,
    // never a silent coercion.
    let duplicate = vm
        .connection_up("alpha@browser", 7)
        .expect_err("a bare double-Up is a loud typed error");
    assert_protocol_error(&duplicate, "bare_double_up");

    // A fate outside the ruled seven-variant mapping is the loud typed error
    // routing the host to the native vocabulary owner — never a local variant.
    let unmapped = vm
        .connection_down("alpha@browser", "wifi_flaky")
        .expect_err("an unmapped down reason is a loud typed error");
    assert_protocol_error(&unmapped, "unmapped_down_reason");

    // Late subscription with snapshot: the synthetic catch-up is delivered
    // SYNCHRONOUSLY, before the subscribe call returns — one Up per live
    // peer, fields round-tripping the shared vocabulary exactly.
    let (log, callback) = event_log();
    let _id = vm.subscribe_connection_events_with_snapshot(callback);
    assert_eq!(
        *log.borrow(),
        vec![up("alpha@browser", 1, 7), up("beta@browser", 1, 9)],
        "synthetic catch-up arrived synchronously, one Up per live peer"
    );

    // Live events land strictly AFTER the synthetic catch-up, and each Down's
    // generation matches its session's Up.
    vm.connection_down("alpha@browser", "peer_closed")
        .expect("alpha goes down");
    assert_eq!(
        log.borrow().last(),
        Some(&down("alpha@browser", 1, "peer_closed")),
        "real events land strictly after the synthetic catch-up"
    );

    // A replacement expands ATOMICALLY into Down(g, reason) then Up(g+1),
    // with peer_creation — not generation — discriminating restart from blip.
    vm.connection_replaced("beta@browser", 11, "heartbeat_timeout")
        .expect("beta is replaced by a restarted peer");
    {
        let events = log.borrow();
        assert_eq!(
            &events[events.len() - 2..],
            &[
                down("beta@browser", 1, "heartbeat_timeout"),
                up("beta@browser", 2, 11),
            ],
            "replaced = Down(g) then Up(g+1) atomically, new peer_creation on the Up"
        );
    }

    // Every one of the seven ruled reason mappings round-trips, generations
    // staying dense per peer across the churn.
    let reasons = [
        "peer_closed",
        "read_error",
        "write_error",
        "write_timeout",
        "manual_disconnect",
        "heartbeat_timeout",
        "control_overflow",
    ];
    for (index, reason) in reasons.iter().enumerate() {
        let generation = index as u64 + 1;
        vm.connection_up("gamma@browser", 3)
            .expect("gamma session opens");
        assert_eq!(
            log.borrow().last(),
            Some(&up("gamma@browser", generation, 3))
        );
        vm.connection_down("gamma@browser", reason)
            .expect("gamma session closes with the mapped reason");
        assert_eq!(
            log.borrow().last(),
            Some(&down("gamma@browser", generation, reason)),
            "the ruled mapping round-trips {reason}"
        );
    }

    // No third variant exists anywhere in the serialized shapes.
    for event in log.borrow().iter() {
        let kind = event["type"].as_str().expect("every event carries a type");
        assert!(
            kind == "up" || kind == "down",
            "only the two native vocabulary shapes may appear: {kind}"
        );
    }
}

#[wasm_bindgen_test]
fn subscription_synthetics_are_subscriber_local_and_unsubscribe_stops_delivery() {
    let vm = WasmVm::new().expect("VM constructs");
    let (early_log, early_callback) = event_log();
    let _early_id = vm.subscribe_connection_events(early_callback);

    vm.connection_up("peer@browser", 5).expect("peer comes up");
    assert_eq!(*early_log.borrow(), vec![up("peer@browser", 1, 5)]);

    // The late subscriber catches up on the in-force session; the synthetic
    // Up is subscriber-local — nothing is replayed to the early subscriber.
    let (late_log, late_callback) = event_log();
    let late_id = vm.subscribe_connection_events_with_snapshot(late_callback);
    assert_eq!(
        *late_log.borrow(),
        vec![up("peer@browser", 1, 5)],
        "the late subscriber catches up on the in-force session"
    );
    assert_eq!(
        *early_log.borrow(),
        vec![up("peer@browser", 1, 5)],
        "the synthetic Up is subscriber-local: nothing replayed to others"
    );

    // A plain (no-snapshot) subscription receives NO catch-up.
    let (plain_log, plain_callback) = event_log();
    let _plain_id = vm.subscribe_connection_events(plain_callback);
    assert!(
        plain_log.borrow().is_empty(),
        "plain subscription starts with no synthetic catch-up"
    );

    // A real event reaches all three.
    vm.connection_down("peer@browser", "manual_disconnect")
        .expect("peer goes down");
    assert_eq!(early_log.borrow().len(), 2);
    assert_eq!(late_log.borrow().len(), 2);
    assert_eq!(
        *plain_log.borrow(),
        vec![down("peer@browser", 1, "manual_disconnect")]
    );

    // Unsubscribe with the numeric SubscriberId: true once, false after, and
    // the unsubscribed callback observes NO later event.
    assert!(
        vm.unsubscribe_connection_events(late_id),
        "the numeric id identifies a live subscription"
    );
    assert!(
        !vm.unsubscribe_connection_events(late_id),
        "a second unsubscribe of the same id returns false"
    );
    vm.connection_up("peer@browser", 6)
        .expect("peer comes back up");
    assert_eq!(
        late_log.borrow().len(),
        2,
        "an unsubscribed subscriber observes no later event"
    );
    assert_eq!(
        early_log.borrow().last(),
        Some(&up("peer@browser", 2, 6)),
        "live subscribers observe the new session; peer_creation 6 != 5 marks \
         a peer restart, not a link blip"
    );
}

#[wasm_bindgen_test]
fn subscription_during_churn_misses_no_session_and_double_sees_none() {
    let vm = WasmVm::new().expect("VM constructs");
    let hub = Rc::clone(&vm.connection_events);

    // Session 1 opens and closes before any subscriber exists: history is
    // never replayed (INV-NO-REPLAY).
    vm.connection_up("zeta@browser", 1).expect("session 1 up");
    vm.connection_down("zeta@browser", "read_error")
        .expect("session 1 down");

    // Session 2 opens; a watcher registers that, on the FIRST Down it sees,
    // reentrantly registers subscriber B through the SNAPSHOT path from
    // inside the callback — which must degrade to plain registration without
    // synthetic catch-up (the native reentrancy rule, verbatim).
    vm.connection_up("zeta@browser", 1).expect("session 2 up");
    let b_log = Rc::new(RefCell::new(Vec::<Value>::new()));
    let watcher = {
        let hub = Rc::clone(&hub);
        let b_log = Rc::clone(&b_log);
        let registered = Rc::new(std::cell::Cell::new(false));
        Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
            let parsed = parse_event(&event);
            if parsed["type"] == "down" && !registered.get() {
                registered.set(true);
                let b_callback = {
                    let b_log = Rc::clone(&b_log);
                    Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
                        b_log.borrow_mut().push(parse_event(&event));
                    })
                    .into_js_value()
                    .unchecked_into::<Function>()
                };
                let _id = hub.subscribe_with_snapshot(b_callback);
            }
        })
        .into_js_value()
        .unchecked_into::<Function>()
    };
    let _watcher_id = vm.subscribe_connection_events(watcher);

    // Subscriber C joins mid-churn through the snapshot path while session 2
    // is open: one synthetic Up(2), then live events.
    let (c_log, c_callback) = event_log();
    let _c_id = vm.subscribe_connection_events_with_snapshot(c_callback);
    assert_eq!(*c_log.borrow(), vec![up("zeta@browser", 2, 1)]);

    // Host-fed churn: close 2 (B registers reentrantly inside this dispatch),
    // open 3 with a changed peer_creation (restart), replace 3 with 4
    // atomically, close 4.
    vm.connection_down("zeta@browser", "write_error")
        .expect("session 2 down");
    vm.connection_up("zeta@browser", 2).expect("session 3 up");
    vm.connection_replaced("zeta@browser", 3, "peer_closed")
        .expect("session 3 replaced by session 4");
    vm.connection_down("zeta@browser", "manual_disconnect")
        .expect("session 4 down");

    // C observed every session from its snapshot forward: alternating, dense,
    // no gap (missed session), no repeat (double-see).
    {
        let events = c_log.borrow();
        assert_alternating_dense(&events, "zeta@browser");
        assert_eq!(events.len(), 6, "Up2 Down2 Up3 Down3 Up4 Down4");
        assert_eq!(events[0]["generation"], 2);
        assert_eq!(
            events.last().map(|event| event["generation"].as_u64()),
            Some(Some(4)),
            "C observed every session through the very last one"
        );
    }

    // B was registered from inside the Down(2) dispatch: it received NO
    // synthetic catch-up (reentrant snapshot degrades to plain registration)
    // and its stream starts at the next queued event — an Up — alternating
    // and dense from there: no session missed, none double-seen.
    {
        let events = b_log.borrow();
        assert!(
            !events.is_empty(),
            "the reentrantly-registered subscriber received the later events"
        );
        assert_eq!(
            events[0],
            up("zeta@browser", 3, 2),
            "B's first event is the next real Up — no synthetic rows mid-drain"
        );
        assert_alternating_dense(&events, "zeta@browser");
        assert_eq!(events.len(), 4, "Up3 Down3 Up4 Down4");
    }
}
