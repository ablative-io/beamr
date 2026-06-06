//! Namespace identifiers for scheduler-scoped module registries.
//!
//! Namespaces isolate module registries while sharing atom tables, BIFs, and
//! the global process table.

use std::sync::Arc;

use dashmap::DashMap;

use crate::module::ModuleRegistry;

/// Identifier for a scheduler module namespace.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct NamespaceId(pub u64);

impl NamespaceId {
    /// Default namespace used by backwards-compatible scheduler APIs.
    pub const DEFAULT: Self = Self(0);
}

/// Store of module registries keyed by namespace id.
pub type NamespaceStore = DashMap<NamespaceId, Arc<ModuleRegistry>>;
