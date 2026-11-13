//! Shared Phase 1 reuse types. Only `ReuseMode` and the base confidence tier
//! are wired first; the remaining types land with the behavior they describe,
//! so no dead code accumulates.

/// Whether a scan performs freshness I/O. Build hints use `Skip`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FreshnessMode {
    #[default]
    Full,
    Skip,
}

/// Which reuse surface produced a result. Phase 1 has `Scan` only; `Propose`
/// is added in Phase 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReuseMode {
    Scan,
    Propose,
    Coverage,
}

impl ReuseMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Propose => "propose",
            Self::Coverage => "coverage",
        }
    }
}

/// Phase 1 confidence. `Medium`/`High` are reserved for Phase 2 and are never
/// produced by the scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReuseConfidence {
    Low,
    Provisional,
    Medium,
    High,
}

impl ReuseConfidence {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Provisional => "provisional",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Parse the public confidence surface. `medium`/`high` are produced only
    /// when Phase 2 calibration signals meet their thresholds.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "provisional" => Some(Self::Provisional),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    pub const fn rank(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Provisional => 1,
            Self::Medium => 2,
            Self::High => 3,
        }
    }
}

/// Per-candidate freshness, derived from the target and canonical files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Freshness {
    #[default]
    Fresh,
    Stale,
    Unknown,
}

impl Freshness {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Unknown => "unknown",
        }
    }
}

/// Why a pair's freshness could not be established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownReason {
    NewFile,
    HashBudget,
    WalkError,
    UnsafePath,
    StatError,
    DeletedFile,
}

impl UnknownReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NewFile => "new-file",
            Self::HashBudget => "hash-budget",
            Self::WalkError => "walk-error",
            Self::UnsafePath => "unsafe-path",
            Self::StatError => "stat-error",
            Self::DeletedFile => "deleted-file",
        }
    }
}

/// Candidate-visible rationale. Counter-only reasons never enter this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReasonCode {
    CallerCallee,
    SameNameSubset,
    ZeroName,
    SizeImbalance,
    StaleIndex,
    FreshnessUnknown,
    UnsafePath,
    BelowThreshold,
    CrossLanguage,
    NoCanonical,
    ProposalPartial,
    ProposalTokenOnly,
    CoverageClass,
    CoverageUnresolved,
    CoverageTests,
}

impl ReasonCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CallerCallee => "caller-callee",
            Self::SameNameSubset => "same-name-subset",
            Self::ZeroName => "zero-name",
            Self::SizeImbalance => "size-imbalance",
            Self::StaleIndex => "stale-index",
            Self::FreshnessUnknown => "freshness-unknown",
            Self::UnsafePath => "unsafe-path",
            Self::BelowThreshold => "below-threshold",
            Self::CrossLanguage => "cross-language",
            Self::NoCanonical => "no-canonical",
            Self::ProposalPartial => "proposal-partial",
            Self::ProposalTokenOnly => "proposal-token-only",
            Self::CoverageClass => "coverage-class",
            Self::CoverageUnresolved => "coverage-unresolved",
            Self::CoverageTests => "coverage-tests",
        }
    }
}

/// Count partition for one scan. `total = shown + below_threshold + omitted`;
/// `capped` is the subset of `omitted` cut by the 30 KiB output cap.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReuseCounts {
    pub shown_provisional: usize,
    pub shown_low: usize,
    pub shown_medium: usize,
    pub shown_high: usize,
    pub suppressed: usize,
    pub guarded_out: usize,
    pub below_threshold: usize,
    pub omitted: usize,
    pub capped: usize,
    pub shown: usize,
    pub total: usize,
    pub truncated: bool,
}
