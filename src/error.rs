//! AwError taxonomy: stable stderr prefix `aw: <class>: <detail>` and the
//! exit-code contract (0 ok; 1 no results / no index; 2 usage or ambiguous;
//! 3 corrupt index; 4 locked; 5 schema mismatch; 6 internal; 7 parse-quality).

use std::fmt;

/// Contract stderr text for a corrupt index (CUJ3, Amendment 11a). Single
/// source: open_for_query and the rusqlite corruption mapping both emit it.
pub const CORRUPT_INDEX_MSG: &str = "index.db failed integrity check — run aw build";

/// SQLite reports corruption as SQLITE_NOTADB (header unreadable at open) or
/// SQLITE_CORRUPT (malformed page hit during a statement). Either maps to the
/// exit-3 contract; everything else stays an internal error (exit 6).
pub(crate) fn is_sqlite_corruption(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(ffi, _)
            if matches!(
                ffi.code,
                rusqlite::ffi::ErrorCode::DatabaseCorrupt
                    | rusqlite::ffi::ErrorCode::NotADatabase
            )
    )
}

/// One resolved name candidate: (path, line, qualname). Distinct from
/// `memory::Candidate` (a fused search-signal row), so one name keeps one
/// meaning per module. Lives here beside the `Ambiguous` payload it fills,
/// so the error and the resolvers that construct it share one spelling.
pub type NameCandidate = (String, i64, String);

#[derive(Debug)]
pub enum AwError {
    NoResults(String),
    /// Empty result for a tool whose contract requires a first-line caveat
    /// on stdout even when nothing is found (dead).
    NoResultsCaveated {
        caveat: &'static str,
        detail: String,
    },
    Usage(String),
    /// Ambiguous bare name: candidates print on stdout (CUJ4), message on
    /// stderr; exit 2. Carries the candidate list so non-CLI surfaces (MCP)
    /// can render the same contract without touching stdout directly.
    /// `role` names the failing endpoint for two-target queries (`path`,
    /// Amendment 13); `None` for single-name tools keeps their bytes
    /// unchanged.
    Ambiguous {
        name: String,
        /// Candidates rendered to the caller, capped for bounded output
        /// (R6f: a common method name could print megabytes on stdout).
        candidates: Vec<NameCandidate>,
        /// Total matches before the render cap.
        total: usize,
        role: Option<&'static str>,
    },
    CorruptIndex(String),
    /// Missing `.aw/memory.db` on a read path (companion design exit contract:
    /// 1 = no results / unknown key / no memory store).
    NoMemory(String),
    /// `memory.db` reported corruption (SQLITE_NOTADB / SQLITE_CORRUPT): exit 3,
    /// same class of failure as a corrupt index but memory is not rebuildable.
    CorruptMemory(String),
    /// `SQLITE_BUSY` after the memory `busy_timeout` expired: exit 4.
    MemoryLocked,
    /// Memory schema skew: newer than the binary, or older but failing the
    /// compiled required-structure manifest. `detail` carries the message body
    /// after the `aw: memory-schema-mismatch:` class prefix.
    MemorySchema {
        detail: String,
    },
    Locked(u32),
    SchemaMismatch {
        have: i32,
        want: i32,
    },
    ParseQuality {
        rate: f64,
        files: usize,
        threshold: f64,
    },
    Internal(String),
}

/// The journal-facing classification of a raw process exit code (the 0-7
/// contract; [`AwError::code_class`] is the error-to-code direction). One
/// home for the thresholds so telemetry surfaces bucket outcomes identically:
/// 0 ok, 1 empty result, 2 ambiguous usage, 3+ operational errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitClass {
    Ok,
    Empty,
    Ambiguous,
    Error,
}

impl ExitClass {
    /// Bucket a journaled exit code.
    pub fn of(code: i64) -> Self {
        match code {
            0 => ExitClass::Ok,
            1 => ExitClass::Empty,
            2 => ExitClass::Ambiguous,
            _ => ExitClass::Error,
        }
    }
}

impl AwError {
    fn code_class(&self) -> (i32, &'static str) {
        match self {
            AwError::NoResults(_) | AwError::NoResultsCaveated { .. } => (1, "no-results"),
            AwError::Usage(_) => (2, "usage"),
            AwError::Ambiguous { .. } => (2, "usage"),
            AwError::CorruptIndex(_) => (3, "corrupt-index"),
            AwError::NoMemory(_) => (1, "no-memory"),
            AwError::CorruptMemory(_) => (3, "corrupt-memory"),
            AwError::MemoryLocked => (4, "locked"),
            AwError::MemorySchema { .. } => (5, "memory-schema-mismatch"),
            AwError::Locked(_) => (4, "locked"),
            AwError::SchemaMismatch { .. } => (5, "schema-mismatch"),
            AwError::Internal(_) => (6, "internal"),
            AwError::ParseQuality { .. } => (7, "parse-quality"),
        }
    }

    /// Shared not-indexed error (exit 1). Single source of the CLI-asserted
    /// `path '<file>' not indexed` string: the file gate in `query::indexed_file`
    /// and the `symbols` fall-through both construct it here, so the contract
    /// text cannot drift between surfaces.
    pub(crate) fn not_indexed(file: &str) -> AwError {
        AwError::NoResults(format!("path '{file}' not indexed"))
    }

    pub fn exit_code(&self) -> i32 {
        self.code_class().0
    }

    fn class(&self) -> &'static str {
        self.code_class().1
    }
}

impl fmt::Display for AwError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AwError::NoResults(detail)
            | AwError::NoResultsCaveated { detail, .. }
            | AwError::Usage(detail)
            | AwError::CorruptIndex(detail)
            | AwError::NoMemory(detail)
            | AwError::CorruptMemory(detail)
            | AwError::MemorySchema { detail }
            | AwError::Internal(detail) => write!(f, "aw: {}: {}", self.class(), detail),
            AwError::Ambiguous { name, total, role, .. } => write!(
                f,
                "aw: {}: ambiguous name '{name}' — {total} candidates{}; use the Class.method qualname form",
                self.class(),
                match role {
                    Some(r) => format!(" as the {r} endpoint"),
                    None => String::new(),
                }
            ),
            AwError::MemoryLocked => write!(
                f,
                "aw: locked: memory write in progress — retry"
            ),
            AwError::Locked(pid) => {
                write!(f, "aw: locked: build in progress (pid {pid}) — retry after it exits")
            }
            AwError::SchemaMismatch { have, want } => write!(
                f,
                "aw: schema-mismatch: index schema v{have}, binary expects v{want} — run aw build"
            ),
            AwError::ParseQuality { rate, files, threshold } => write!(
                f,
                "aw: parse-quality: {:.0}% of {files} files failed to parse (threshold {:.0}%) — inspect with aw doctor or override with --force",
                rate * 100.0,
                threshold * 100.0
            ),
        }
    }
}

impl std::error::Error for AwError {}

impl From<rusqlite::Error> for AwError {
    fn from(e: rusqlite::Error) -> Self {
        // Amendment 11a: corruption is detected where SQLite reports it (open
        // or any statement), replacing the per-open quick_check scan.
        if is_sqlite_corruption(&e) {
            AwError::CorruptIndex(CORRUPT_INDEX_MSG.to_string())
        } else {
            AwError::Internal(format!("database error: {e}"))
        }
    }
}

impl From<std::io::Error> for AwError {
    fn from(e: std::io::Error) -> Self {
        AwError::Internal(format!("io error: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, AwError>;
