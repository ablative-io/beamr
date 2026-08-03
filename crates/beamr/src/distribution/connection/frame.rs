use crate::distribution::etf::MAX_DIST_FRAME_BYTES;

/// Idle-link liveness keepalive frame: the 8-byte all-zero header the data-frame
/// read loop already accepts as `control_len = 0, payload_len = 0` — a no-op that
/// invokes no control handler. Each peer's periodic net-tick writes this so the
/// other side's read loop refreshes its last-inbound timestamp, letting a
/// silently-partitioned (black-holed) peer be detected by a missed deadline
/// rather than only by a TCP FIN/RST.
pub(super) const KEEPALIVE_FRAME: [u8; 8] = [0_u8; 8];

/// Why the framing read path refused an inbound data frame.
///
/// Every variant is terminal for the link: a refusal happens before the frame
/// body is consumed, so the read loop has no frame boundary left to
/// resynchronise on and marks the connection down.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum FrameError {
    /// `control_len + payload_len` overflowed `usize`. Reachable only where
    /// `usize` is narrower than 64 bits (wasm32); kept unconditionally because
    /// both header fields are entirely peer-controlled.
    LengthOverflow,
    /// The declared frame length exceeds [`MAX_DIST_FRAME_BYTES`].
    FrameTooLarge {
        /// Total bytes the header declared (`control_len + payload_len`).
        frame_bytes: usize,
        /// The cap in force, i.e. [`MAX_DIST_FRAME_BYTES`].
        max_frame_bytes: usize,
    },
    /// The frame is within the cap, but the allocator could not supply it.
    AllocationFailed {
        /// Total bytes the header declared.
        frame_bytes: usize,
    },
}

/// Decode an 8-byte data-frame header into `(control_len, frame_buffer)`.
///
/// Every refusal here precedes the allocation of any peer-named byte count, in
/// order: overflow, then the [`MAX_DIST_FRAME_BYTES`] cap, then a fallible
/// allocation. The returned buffer is exactly `control_len + payload_len` bytes
/// and is the caller's read target.
pub(super) fn frame_buffer_for_header(header: [u8; 8]) -> Result<(usize, Vec<u8>), FrameError> {
    let control_len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
    let payload_len = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
    let Some(total_len) = control_len.checked_add(payload_len) else {
        return Err(FrameError::LengthOverflow);
    };
    if total_len > MAX_DIST_FRAME_BYTES {
        return Err(FrameError::FrameTooLarge {
            frame_bytes: total_len,
            max_frame_bytes: MAX_DIST_FRAME_BYTES,
        });
    }
    // Fallible allocation behind the cap: an in-cap frame the allocator still
    // cannot supply must surface as a refusal, never as an abort.
    let mut frame = Vec::new();
    frame
        .try_reserve_exact(total_len)
        .map_err(|_| FrameError::AllocationFailed {
            frame_bytes: total_len,
        })?;
    frame.resize(total_len, 0_u8);
    Ok((control_len, frame))
}
