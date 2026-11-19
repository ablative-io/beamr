//! Where a process is in its code: the namespace it runs against, its code
//! position, and the module version it has pinned.

// `Arc` comes from std, which is absent under `--no-default-features`; guarded
// as the `Box` import in `mod.rs` is, so this file adds nothing to the no_std
// tally.
#[cfg(not(feature = "std"))]
use alloc::sync::Arc;
#[cfg(feature = "std")]
use std::sync::Arc;

use crate::atom::Atom;
use crate::module::Module;
use crate::namespace::NamespaceId;

use super::{CodePosition, Process};

impl Process {
    /// Namespace whose module registry this process executes against.
    #[must_use]
    pub const fn namespace_id(&self) -> NamespaceId {
        self.namespace_id
    }

    /// Set the namespace whose module registry this process executes against.
    pub const fn set_namespace_id(&mut self, namespace_id: NamespaceId) {
        self.namespace_id = namespace_id;
    }

    /// Current code position, if one has been assigned.
    #[must_use]
    pub const fn code_position(&self) -> Option<CodePosition> {
        self.code_position
    }

    /// Set the current code position.
    pub const fn set_code_position(&mut self, code_position: Option<CodePosition>) {
        self.code_position = code_position;
    }

    /// Current pinned module version, if one has been assigned.
    #[must_use]
    pub fn current_module(&self) -> Option<&Arc<Module>> {
        self.current_module.as_ref()
    }

    /// Returns true when the current module or any stack frame pins `module`.
    #[must_use]
    pub fn references_module(&self, module: &Arc<Module>) -> bool {
        self.current_module
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, module))
            || self
                .stack
                .pinned_modules()
                .any(|pinned| Arc::ptr_eq(pinned, module))
    }

    /// Set the currently executing module version.
    pub fn set_current_module(&mut self, module: Arc<Module>) {
        self.current_module = Some(module);
    }

    /// Clear the currently executing module version.
    pub fn clear_current_module(&mut self) {
        self.current_module = None;
    }

    /// Module/function/arity of the function this process is currently
    /// positioned in, DERIVED at read time from the pinned module's `func_info`
    /// bounds ([`Module::mfa_at_ip`]) and the current instruction pointer.
    ///
    /// Replaces a stored field that only one production writer (the `func_info`
    /// opcode, reached solely on a failed multi-clause dispatch) ever updated,
    /// so every diagnostic reader saw a stale or absent MFA. `None` for
    /// native-only and exited processes (no module/ip in reach) — derivation
    /// never invents provenance. Pairs `current_module` with
    /// `code_position().instruction_pointer`, the same pairing
    /// `capture_raw_stacktrace` already trusts for the stacktrace head.
    #[must_use]
    pub fn current_mfa(&self) -> Option<(Atom, Atom, u8)> {
        let module = self.current_module.as_ref()?;
        let ip = self.code_position?.instruction_pointer;
        module.mfa_at_ip(ip)
    }
}
