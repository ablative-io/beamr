use std::fmt;

/// Readiness directions a registration arms (contract §3.1).
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Interest(pub(super) u8);

impl Interest {
    pub const READABLE: Self = Self(1);
    pub const WRITABLE: Self = Self(2);

    #[must_use]
    pub const fn both() -> Self {
        Self(Self::READABLE.0 | Self::WRITABLE.0)
    }

    #[must_use]
    pub const fn is_readable(self) -> bool {
        self.0 & Self::READABLE.0 != 0
    }

    #[must_use]
    pub const fn is_writable(self) -> bool {
        self.0 & Self::WRITABLE.0 != 0
    }

    pub(super) fn as_mio(self) -> mio::Interest {
        match (self.is_readable(), self.is_writable()) {
            (true, true) => mio::Interest::READABLE | mio::Interest::WRITABLE,
            (true, false) => mio::Interest::READABLE,
            (false, true) => mio::Interest::WRITABLE,
            (false, false) => mio::Interest::READABLE,
        }
    }
}

pub(super) type SlotIndex = u32;

/// Per-slot monotonic generation minted by registration.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Generation(pub(super) u64);

/// Opaque registration identity with a registration-minted generation.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct ReadinessToken {
    pub(super) slot: SlotIndex,
    pub(super) generation: Generation,
}

impl ReadinessToken {
    pub(super) fn mio_token(self) -> mio::Token {
        mio::Token(((self.slot as u64) << 32 | (self.generation.0 & u64::from(u32::MAX))) as usize)
    }

    pub(super) fn decode(token: mio::Token) -> (SlotIndex, u32) {
        let raw = token.0 as u64;
        ((raw >> 32) as SlotIndex, raw as u32)
    }
}

/// Typed refusal for readiness registration and rearm operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadinessError {
    Disabled,
    Register { errno: i32 },
    UnknownToken,
    ServiceFailed,
    TeardownInProgress,
}

impl fmt::Display for ReadinessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => f.write_str("readiness service is disabled"),
            Self::Register { errno } => {
                write!(f, "kernel readiness registration failed: errno {errno}")
            }
            Self::UnknownToken => f.write_str("readiness token is no longer registered"),
            Self::ServiceFailed => f.write_str("readiness poll thread has failed"),
            Self::TeardownInProgress => {
                f.write_str("scheduler teardown has closed readiness admission")
            }
        }
    }
}

impl std::error::Error for ReadinessError {}

/// Construction-time refusal when the poll set or waker cannot be allocated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadinessBuildError {
    PollSetUnavailable { errno: i32 },
}

impl fmt::Display for ReadinessBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PollSetUnavailable { errno } => {
                write!(f, "readiness poll set is unavailable: errno {errno}")
            }
        }
    }
}

impl std::error::Error for ReadinessBuildError {}

pub(super) fn errno(error: &std::io::Error) -> i32 {
    error.raw_os_error().unwrap_or(libc::EIO)
}
