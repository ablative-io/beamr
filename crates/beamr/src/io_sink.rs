//! The output-sink seam shared by both scheduler closures (WPORT-5 R2).
//!
//! [`IoSink`] and [`NullSink`] historically lived in the `threads`-gated `io`
//! module, which left the cooperative (`wasm32`) closure with an unconditional
//! no-op `write_to_io_sink` — the 9-member silent-output class the WPORT-5
//! profile brief documents. Only the sink *vocabulary* moves here: the trait
//! and its null default carry no thread machinery (no rings, no pools, no
//! blocking IO), so the cooperative closure can reach them without pulling in
//! anything from `crate::io`. The `io` module re-exports both names, so every
//! existing threaded import path and `IoSink` impl compiles unchanged.
//!
//! The sink is PUSH-ONLY (NO-POLLING law, `docs/design/beamr/WASM-PORT-ARC.md`
//! §laws): writes flush through [`IoSink::write_stream`]/[`IoSink::write`]
//! synchronously at the writing BIF. No flush timer, no recurring callback,
//! and no buffer-poll exist on any branch of this seam.

/// Output stream tag carried by tagged sink writes (WPORT-5 R2 item 4).
///
/// `Out` is the stdout-flavoured stream (`io:put_chars`, `io:format/3`,
/// `gleam_stdlib` `print`/`println`, `erlang:display/1` on the cooperative
/// path); `Err` is the stderr-flavoured stream (`gleam_stdlib` `print_error`/
/// `println_error`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoStream {
    /// Standard-output-flavoured writes.
    Out,
    /// Standard-error-flavoured writes.
    Err,
}

/// Output target for `io` module BIFs.
pub trait IoSink: Send + Sync {
    /// Write bytes to the sink.
    fn write(&self, bytes: &[u8]);

    /// Write bytes with a stream tag (WPORT-5 R2 item 4).
    ///
    /// ADDITIVE with a default: pre-existing threaded [`IoSink`] impls compile
    /// unchanged and keep their exact single-stream behaviour — the default
    /// discards the tag and routes through [`IoSink::write`]. Sinks that
    /// distinguish out/err (the browser console sink) override this.
    fn write_stream(&self, stream: IoStream, bytes: &[u8]) {
        let _ = stream;
        self.write(bytes);
    }
}

/// Default output sink that intentionally discards all bytes.
#[derive(Debug, Default)]
pub struct NullSink;

impl IoSink for NullSink {
    fn write(&self, _bytes: &[u8]) {}
}
