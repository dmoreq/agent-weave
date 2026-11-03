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
fn is_index_corruption(e: &rusqlite::Error) -> bool {
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

#[derive(Debug)]
pub enum AwError {
    NoResults(String),
    /// Empty result for a tool whose contract requires a first-line caveat
    /// on stdout even when nothing is found (dead, clones).
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
        candidates: Vec<(String, i64, String)>,
        /// Total matches before the render cap.
        total: usize,
        role: Option<&'static str>,
    },
    CorruptIndex(String),
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
    /// Batch surface whose stdout text is already complete; MCP must return
    /// it with `isError` instead of routing through the error renderer.
    BatchOutput {
        text: String,
        code: i32,
    },
}

impl AwError {
    pub fn exit_code(&self) -> i32 {
        match self {
            AwError::NoResults(_) | AwError::NoResultsCaveated { .. } => 1,
            AwError::Usage(_) => 2,
            AwError::Ambiguous { .. } => 2,
            AwError::CorruptIndex(_) => 3,
            AwError::Locked(_) => 4,
            AwError::SchemaMismatch { .. } => 5,
            AwError::Internal(_) => 6,
            AwError::BatchOutput { code, .. } => *code,
            AwError::ParseQuality { .. } => 7,
        }
    }

    fn class(&self) -> &'static str {
        match self {
            AwError::NoResults(_) | AwError::NoResultsCaveated { .. } => "no-results",
            AwError::Usage(_) => "usage",
            AwError::Ambiguous { .. } => "usage",
            AwError::CorruptIndex(_) => "corrupt-index",
            AwError::Locked(_) => "locked",
            AwError::SchemaMismatch { .. } => "schema-mismatch",
            AwError::ParseQuality { .. } => "parse-quality",
            AwError::Internal(_) => "internal",
            AwError::BatchOutput { .. } => "batch",
        }
    }
}

impl fmt::Display for AwError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AwError::NoResults(detail) => write!(f, "aw: {}: {}", self.class(), detail),
            AwError::NoResultsCaveated { detail, .. } => {
                write!(f, "aw: {}: {}", self.class(), detail)
            }
            AwError::Usage(detail) => write!(f, "aw: {}: {}", self.class(), detail),
            AwError::Ambiguous { name, total, role, .. } => write!(
                f,
                "aw: {}: ambiguous name '{name}' — {total} candidates{}; use the Class.method qualname form",
                self.class(),
                match role {
                    Some(r) => format!(" as the {r} endpoint"),
                    None => String::new(),
                }
            ),
            AwError::CorruptIndex(detail) => write!(f, "aw: {}: {}", self.class(), detail),
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
            AwError::Internal(detail) => write!(f, "aw: {}: {}", self.class(), detail),
            AwError::BatchOutput { text, .. } => write!(f, "{text}"),
        }
    }
}

impl std::error::Error for AwError {}

impl From<rusqlite::Error> for AwError {
    fn from(e: rusqlite::Error) -> Self {
        // Amendment 11a: corruption is detected where SQLite reports it (open
        // or any statement), replacing the per-open quick_check scan.
        if is_index_corruption(&e) {
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
