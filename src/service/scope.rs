//! One store-scope decision for `mem query`, shared by CLI and MCP surfaces so
//! the conflict message and the fallback rules cannot drift.

use crate::error::{EgError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StoreScope {
    /// Local and global stores, local first (the default).
    Merged,
    /// Project store only; never falls back to global.
    Local,
    /// Global store only.
    Global,
}

impl StoreScope {
    /// Reject the mutually exclusive pair before any store is opened. The
    /// message is deliberately surface-neutral for byte parity.
    pub(super) fn from_flags(local: bool, global: bool) -> Result<Self> {
        match (local, global) {
            (true, true) => Err(EgError::Usage(
                "mem query: --local and --global are mutually exclusive".into(),
            )),
            (true, false) => Ok(StoreScope::Local),
            (false, true) => Ok(StoreScope::Global),
            (false, false) => Ok(StoreScope::Merged),
        }
    }
}
