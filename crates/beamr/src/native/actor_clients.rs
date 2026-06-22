//! Transient one-shot native processes that back the external
//! [`super::SenderHandle`] cast/call helpers.
//!
//! Each performs exactly one envelope send through [`NativeContext::send`] (so
//! the sender-clock and replay discipline of NATIVE-001 holds — no side
//! channel), then stops. Neither is reachable from inside an actor handler, so
//! the call-deadlock hazard (see `super` module docs) cannot arise here.

use std::marker::PhantomData;

use super::{Actor, ActorMessage, TAG_CALL, TAG_CAST, decode_reply};
use crate::native::native_process::{NativeContext, NativeHandler, NativeOutcome};
use crate::process::ExitReason;
use crate::term::Term;

/// Build the one-shot cast sender handler (encapsulates the private struct so
/// the parent only constructs it through this seam).
pub(super) fn cast_handler<A: Actor>(target: u64, message: A::Cast) -> Box<dyn NativeHandler> {
    Box::new(CastClient::<A> {
        target,
        message,
        sent: false,
        _marker: PhantomData,
    })
}

/// Build the one-shot request/reply client handler.
pub(super) fn call_handler<A: Actor>(
    target: u64,
    request: A::Call,
    reference: u64,
    reply_tx: crossbeam_channel::Sender<A::Reply>,
) -> Box<dyn NativeHandler> {
    Box::new(CallClient::<A> {
        target,
        request,
        reference,
        reply_tx,
        sent: false,
    })
}

/// Transient native process that performs one fire-and-forget cast, then stops.
struct CastClient<A: Actor> {
    target: u64,
    message: A::Cast,
    sent: bool,
    _marker: PhantomData<fn() -> A>,
}

impl<A: Actor> NativeHandler for CastClient<A> {
    fn handle(&mut self, ctx: &mut NativeContext<'_>) -> NativeOutcome {
        if !self.sent {
            self.sent = true;
            if let Some(payload) = self.message.encode(ctx)
                && let Some(envelope) = ctx.alloc_tuple(&[Term::small_int(TAG_CAST), payload])
            {
                ctx.send(self.target, envelope);
            }
        }
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

/// Transient native process for one request/reply call: slice 1 sends the call
/// envelope; a later slice receives the ref-matched reply, forwards the decoded
/// reply to the waiting external thread, and stops.
struct CallClient<A: Actor> {
    target: u64,
    request: A::Call,
    reference: u64,
    reply_tx: crossbeam_channel::Sender<A::Reply>,
    sent: bool,
}

impl<A: Actor> NativeHandler for CallClient<A> {
    fn handle(&mut self, ctx: &mut NativeContext<'_>) -> NativeOutcome {
        if !self.sent {
            self.sent = true;
            // Build {1, ref, reply_to, request}. `reply_to` is our pid as an
            // integer scalar, not a pid term, so it survives the Executing/ETF
            // delivery path (see `super` module docs).
            let reference = Term::try_small_int(self.reference as i64);
            let reply_to = i64::try_from(ctx.self_pid())
                .ok()
                .and_then(Term::try_small_int);
            if let (Some(reference), Some(reply_to), Some(request)) =
                (reference, reply_to, self.request.encode(ctx))
                && let Some(envelope) =
                    ctx.alloc_tuple(&[Term::small_int(TAG_CALL), reference, reply_to, request])
            {
                ctx.send(self.target, envelope);
            }
            return NativeOutcome::Wait;
        }
        while let Some(message) = ctx.recv() {
            if let Some((reference, reply_term)) = decode_reply(message)
                && reference == self.reference as i64
            {
                if let Some(reply) = A::Reply::decode(reply_term) {
                    let _ = self.reply_tx.send(reply);
                }
                return NativeOutcome::Stop(ExitReason::Normal);
            }
        }
        NativeOutcome::Wait
    }
}
