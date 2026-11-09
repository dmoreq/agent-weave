//! EgError taxonomy: stable stderr prefix `eg: <class>: <detail>` and the
//! exit-code contract (0 ok; 1 no results / no index; 2 usage, ambiguous, or
//! security; 3 corrupt index; 4 locked; 5 schema mismatch; 6 internal;
//! 7 parse-quality).

use std::fmt;

/// Contract stderr text for a corrupt index (CUJ3, Amendment 11a). Single
/// source: open_for_query and the rusqlite corruption mapping both emit it.
pub const CORRUPT_INDEX_MSG: &str = "index.db failed integrity check — run eg build";

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
pub enum EgError {
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
    /// Missing `.engram/memory.db` on a read path (companion design exit contract:
    /// 1 = no results / unknown key / no memory store).
    NoMemory(String),
    /// `memory.db` reported corruption (SQLITE_NOTADB / SQLITE_CORRUPT): exit 3,
    /// same class of failure as a corrupt index but memory is not rebuildable.
    CorruptMemory(String),
    /// `SQLITE_BUSY` after the memory `busy_timeout` expired: exit 4.
    MemoryLocked,
    /// Memory schema skew: newer than the binary, or older but failing the
    /// compiled required-structure manifest. `detail` carries the message body
    /// after the `eg: memory-schema-mismatch:` class prefix.
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
    /// Untrusted repo-local state: `.engram/` carries git-tracked files, so a
    /// repository could ship attacker-authored memories or index rows. Exit 2
    /// (`eg: security: ...`), never a silent read.
    Security(String),
    Internal(String),
}

/// The journal-facing classification of a raw process exit code (the 0-7
/// contract; [`EgError::code_class`] is the error-to-code direction). One
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

impl EgError {
    fn code_class(&self) -> (i32, &'static str) {
        match self {
            EgError::NoResults(_) | EgError::NoResultsCaveated { .. } => (1, "no-results"),
            EgError::Usage(_) => (2, "usage"),
            EgError::Ambiguous { .. } => (2, "usage"),
            EgError::CorruptIndex(_) => (3, "corrupt-index"),
            EgError::NoMemory(_) => (1, "no-memory"),
            EgError::CorruptMemory(_) => (3, "corrupt-memory"),
            EgError::MemoryLocked => (4, "locked"),
            EgError::MemorySchema { .. } => (5, "memory-schema-mismatch"),
            EgError::Locked(_) => (4, "locked"),
            EgError::SchemaMismatch { .. } => (5, "schema-mismatch"),
            EgError::Security(_) => (2, "security"),
            EgError::Internal(_) => (6, "internal"),
            EgError::ParseQuality { .. } => (7, "parse-quality"),
        }
    }

    /// Shared not-indexed error (exit 1). Single source of the CLI-asserted
    /// `path '<file>' not indexed` string: the file gate in `query::indexed_file`
    /// and the `symbols` fall-through both construct it here, so the contract
    /// text cannot drift between surfaces.
    pub(crate) fn not_indexed(file: &str) -> EgError {
        EgError::NoResults(format!("path '{file}' not indexed"))
    }

    pub fn exit_code(&self) -> i32 {
        self.code_class().0
    }

    fn class(&self) -> &'static str {
        self.code_class().1
    }
}

impl fmt::Display for EgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EgError::NoResults(detail)
            | EgError::NoResultsCaveated { detail, .. }
            | EgError::Usage(detail)
            | EgError::CorruptIndex(detail)
            | EgError::NoMemory(detail)
            | EgError::CorruptMemory(detail)
            | EgError::MemorySchema { detail }
            | EgError::Security(detail)
            | EgError::Internal(detail) => write!(f, "eg: {}: {}", self.class(), detail),
            EgError::Ambiguous { name, total, role, .. } => write!(
                f,
                "eg: {}: ambiguous name '{name}' — {total} candidates{}; use the Class.method qualname form",
                self.class(),
                match role {
                    Some(r) => format!(" as the {r} endpoint"),
                    None => String::new(),
                }
            ),
            EgError::MemoryLocked => write!(
                f,
                "eg: locked: memory write in progress — retry"
            ),
            EgError::Locked(pid) => {
                write!(f, "eg: locked: build in progress (pid {pid}) — retry after it exits")
            }
            EgError::SchemaMismatch { have, want } => write!(
                f,
                "eg: schema-mismatch: index schema v{have}, binary expects v{want} — run eg build"
            ),
            EgError::ParseQuality { rate, files, threshold } => write!(
                f,
                "eg: parse-quality: {:.0}% of {files} files failed to parse (threshold {:.0}%) — inspect with eg doctor or override with --force",
                rate * 100.0,
                threshold * 100.0
            ),
        }
    }
}

impl std::error::Error for EgError {}

impl From<rusqlite::Error> for EgError {
    fn from(e: rusqlite::Error) -> Self {
        // Amendment 11a: corruption is detected where SQLite reports it (open
        // or any statement), replacing the per-open quick_check scan.
        if is_sqlite_corruption(&e) {
            EgError::CorruptIndex(CORRUPT_INDEX_MSG.to_string())
        } else {
            EgError::Internal(format!("database error: {e}"))
        }
    }
}

impl From<std::io::Error> for EgError {
    fn from(e: std::io::Error) -> Self {
        EgError::Internal(format!("io error: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, EgError>;

/// Transitional spelling kept for downstream callers compiled against the
/// pre-rename crate identity.
pub type AwError = EgError;
