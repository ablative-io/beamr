//// Minimal real `gleam_otp` actor exercised by the beamr OTP-actor spike.
////
//// `run/0` is the round-trip proof spine: it starts a real `gleam_otp` actor
//// (`actor.new |> actor.on_message |> actor.start`), fires two casts
//// (`actor.send`), then does a synchronous `actor.call` — which internally
//// monitors the actor and selective-receives the reply on the bound monitor
//// reference — and returns the observed count. A Rust integration test spawns
//// `actor_spike:run/0`, runs it to exit, and asserts the returned integer,
//// proving the call round-trip completed across two real scheduler processes.
////
//// `subject_probe/0` and `receive_probe/0` are focused regression probes for
//// the two hardest capabilities the round-trip depends on: cross-process local
//// message delivery (a spawned closure that captures a subject and sends to it),
//// received once via the compound-key selector path (`subject_probe`) and once
//// via a plain `{Ref, Message}` receive (`receive_probe`).

import gleam/erlang/process.{type Subject}
import gleam/otp/actor

/// Messages the counter actor accepts. `Inc` is a fire-and-forget cast; `Get`
/// carries a reply subject and drives the synchronous `actor.call` round-trip.
pub type Msg {
  Inc
  Get(reply: Subject(Int))
}

fn handle(state: Int, msg: Msg) -> actor.Next(Int, Msg) {
  case msg {
    Inc -> actor.continue(state + 1)
    Get(reply) -> {
      process.send(reply, state)
      actor.continue(state)
    }
  }
}

/// Spawn a closure that captures a subject, send across the process boundary,
/// and selector-receive the reply — the exact ack path `actor.start` relies on,
/// with no gleam_otp layered on top. Returns the delivered value (77).
pub fn subject_probe() -> Int {
  let subject = process.new_subject()
  let selector =
    process.new_selector()
    |> process.select_map(subject, fn(x) { x })
  let _child = process.spawn(fn() { process.send(subject, 77) })
  case process.selector_receive(selector, 2000) {
    Ok(n) -> n
    Error(_) -> -1
  }
}

/// Same cross-process spawn+send, but received via `process.receive` (a plain
/// `{Ref, Message}` receive, no selector). Returns the delivered value (88).
pub fn receive_probe() -> Int {
  let subject = process.new_subject()
  let _child = process.spawn(fn() { process.send(subject, 88) })
  case process.receive(subject, 2000) {
    Ok(n) -> n
    Error(_) -> -1
  }
}

/// Start the actor, cast two increments, then synchronously call for the count.
/// Returns the count observed by the caller (expected `2`).
pub fn run() -> Int {
  let assert Ok(started) =
    actor.new(0)
    |> actor.on_message(handle)
    |> actor.start
  let subject = started.data
  actor.send(subject, Inc)
  actor.send(subject, Inc)
  actor.call(subject, waiting: 3000, sending: Get)
}
