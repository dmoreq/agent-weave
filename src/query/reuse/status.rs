//! Validated target scope: one kind per scan invocation.

use std::collections::HashSet;

/// One validated target kind. MCP/CLI argument parsing never mixes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScopeKind {
    Repo,
    Files(Vec<String>),
    Symbols(Vec<i64>),
}

/// One validated scan scope. `paths` holds exact files and directory prefixes;
/// `symbols` holds resolved qualnames.
pub(crate) struct ResolvedScope {
    pub(super) kind: ScopeKind,
    pub(super) paths: Vec<String>,
    pub(super) symbols: HashSet<String>,
    /// Unique indexed files selected by the scope, in deterministic order.
    pub(super) files: Vec<String>,
    pub(super) path_matches: usize,
}

impl ResolvedScope {
    pub(super) fn is_repo(&self) -> bool {
        matches!(self.kind, ScopeKind::Repo)
    }

    pub(super) fn path_filter(&self) -> Option<&[String]> {
        (!self.paths.is_empty()).then_some(self.paths.as_slice())
    }

    pub(super) fn target_names(&self) -> Option<&HashSet<String>> {
        (!self.symbols.is_empty()).then_some(&self.symbols)
    }
}
