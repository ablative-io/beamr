//! Public Beamr-owned JIT value types.

/// A GC root location described by a future stack map entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RootLocation {
    /// A live root held in a machine register.
    Register(u16),
    /// A live root held in a stack slot relative to the frame layout.
    StackSlot(i32),
}

/// Stack map metadata for one native-code safepoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StackMapEntry {
    /// Machine-code offset from the function entry point.
    pub offset_from_entry: u32,
    /// Live roots known at this safepoint.
    pub live_roots: Vec<RootLocation>,
}

/// Immutable native code emitted by the JIT compiler.
#[derive(Clone, Debug)]
pub struct NativeCode {
    call_addr: usize,
    stack_maps: Vec<StackMapEntry>,
}

impl NativeCode {
    /// Creates a native-code handle from compiler-owned code memory.
    pub(crate) fn new(call_ptr: *const u8, stack_maps: Vec<StackMapEntry>) -> Self {
        Self {
            call_addr: call_ptr as usize,
            stack_maps,
        }
    }

    /// Raw entry pointer for the compiled `extern "C"` function.
    #[must_use]
    pub fn call_ptr(&self) -> *const u8 {
        self.call_addr as *const u8
    }

    /// Stack map entries for GC cooperation.
    #[must_use]
    pub fn stack_maps(&self) -> &[StackMapEntry] {
        &self.stack_maps
    }
}
