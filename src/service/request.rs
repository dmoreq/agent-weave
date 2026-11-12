//! The dispatchable command enum shared by the CLI and MCP surfaces.
//!
//! It lives in its own module so the surface mappers and the dispatch layer can
//! grow without pushing `service.rs` past the 500-line hygiene cap.

/// One dispatchable command with its resolved arguments.
#[derive(Debug)]
pub enum CommandRequest {
    Build {
        force: bool,
        max_parse_error_rate: f64,
    },
    Callers {
        name: String,
        limit: usize,
        depth: usize,
    },
    Callees {
        name: String,
        limit: usize,
        depth: usize,
    },
    Context {
        name: String,
        limit: usize,
    },
    Imports {
        file: String,
        limit: usize,
    },
    Path {
        from: String,
        to: String,
        depth: usize,
    },
    Impact {
        file: String,
        sites: bool,
        depth: usize,
        /// Validated `--edges` kinds (parsed at the surface boundary, so a bad
        /// spec is a usage error before dispatch).
        edges: Vec<crate::extract::EdgeKind>,
        limit: usize,
    },
    Unresolved {
        names: bool,
        path: Option<String>,
        limit: usize,
    },
    Symbols {
        file: String,
        outline: bool,
        limit: usize,
    },
    Reuse {
        paths: Vec<String>,
        limit: usize,
        deep_callee: bool,
        judge: bool,
    },
    Search {
        terms: String,
        limit: usize,
    },
    Stats {
        path: Option<String>,
    },
    Doctor {
        invalidate: bool,
    },
    Usage {
        quality: bool,
        global: bool,
    },
    Audit {
        json: bool,
        global: bool,
        days: u32,
        baseline_days: u32,
        all_checks: bool,
        reveal_paths: bool,
        fail_on: Option<String>,
    },
    MemWrite {
        key: String,
        body: String,
        kind: String,
        tags: Option<String>,
        refs: Option<String>,
        scope: String,
        ttl: Option<u64>,
        supersedes: Option<String>,
        global: bool,
        touch: bool,
    },
    MemQuery {
        terms: Vec<String>,
        all: bool,
        kind: Option<crate::memory::MemKind>,
        scope: Option<crate::memory::MemScope>,
        limit: usize,
        include_superseded: bool,
        global: bool,
        reference: Option<String>,
        since: Option<i64>,
        until: Option<i64>,
        local: bool,
        explain: bool,
        legacy: bool,
    },
    MemTouch {
        key: String,
        global: bool,
    },
    MemGet {
        key: String,
        global: bool,
    },
    MemDelete {
        key: String,
        global: bool,
    },
    MemGc {
        global: bool,
    },
    MemExport {
        format: Option<String>,
        limit: Option<usize>,
        global: bool,
        since: Option<i64>,
        until: Option<i64>,
        scan: bool,
    },
    MemImport {
        path: String,
        format: Option<String>,
        force: bool,
        global: bool,
    },
    MemReindex {
        global: bool,
    },
}
