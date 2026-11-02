//! KgError taxonomy: stable stderr prefix `kg: <class>: <detail>` and the
//! exit-code contract (0 ok; 1 no results / no index; 2 usage or ambiguous;
//! 3 corrupt index; 4 locked; 5 schema mismatch; 6 internal; 7 parse-quality).

use std::fmt;

/// Contract stderr text for a corrupt index (CUJ3, Amendment 11a). Single
/// source: open_for_query and the rusqlite corruption mapping both emit it.
pub const CORRUPT_INDEX_MSG: &str = "index.db failed integrity check — run kg build";

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
pub enum KgError {
    NoResults(String),
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
    SchemaMismatch { have: i32, want: i32 },
    ParseQuality { rate: f64, files: usize, threshold: f64 },
    Internal(String),
}

impl KgError {
    pub fn exit_code(&self) -> i32 {
        match self {
            KgError::NoResults(_) => 1,
            KgError::Usage(_) => 2,
            KgError::Ambiguous { .. } => 2,
            KgError::CorruptIndex(_) => 3,
            KgError::Locked(_) => 4,
            KgError::SchemaMismatch { .. } => 5,
            KgError::Internal(_) => 6,
            KgError::ParseQuality { .. } => 7,
        }
    }

    fn class(&self) -> &'static str {
        match self {
            KgError::NoResults(_) => "no-results",
            KgError::Usage(_) => "usage",
            KgError::Ambiguous { .. } => "usage",
            KgError::CorruptIndex(_) => "corrupt-index",
            KgError::Locked(_) => "locked",
            KgError::SchemaMismatch { .. } => "schema-mismatch",
            KgError::ParseQuality { .. } => "parse-quality",
            KgError::Internal(_) => "internal",
        }
    }
}

impl fmt::Display for KgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KgError::NoResults(detail) => write!(f, "kg: {}: {}", self.class(), detail),
            KgError::Usage(detail) => write!(f, "kg: {}: {}", self.class(), detail),
            KgError::Ambiguous { name, total, role, .. } => write!(
                f,
                "kg: {}: ambiguous name '{name}' — {total} candidates{}; use the Class.method qualname form",
                self.class(),
                match role {
                    Some(r) => format!(" as the {r} endpoint"),
                    None => String::new(),
                }
            ),
            KgError::CorruptIndex(detail) => write!(f, "kg: {}: {}", self.class(), detail),
            KgError::Locked(pid) => {
                write!(f, "kg: locked: build in progress (pid {pid}) — retry after it exits")
            }
            KgError::SchemaMismatch { have, want } => write!(
                f,
                "kg: schema-mismatch: index schema v{have}, binary expects v{want} — run kg build"
            ),
            KgError::ParseQuality { rate, files, threshold } => write!(
                f,
                "kg: parse-quality: {:.0}% of {files} files failed to parse (threshold {:.0}%) — inspect with kg doctor or override with --force",
                rate * 100.0,
                threshold * 100.0
            ),
            KgError::Internal(detail) => write!(f, "kg: {}: {}", self.class(), detail),
        }
    }
}

impl std::error::Error for KgError {}

impl From<rusqlite::Error> for KgError {
    fn from(e: rusqlite::Error) -> Self {
        // Amendment 11a: corruption is detected where SQLite reports it (open
        // or any statement), replacing the per-open quick_check scan.
        if is_index_corruption(&e) {
            KgError::CorruptIndex(CORRUPT_INDEX_MSG.to_string())
        } else {
            KgError::Internal(format!("database error: {e}"))
        }
    }
}

impl From<std::io::Error> for KgError {
    fn from(e: std::io::Error) -> Self {
        KgError::Internal(format!("io error: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, KgError>;
