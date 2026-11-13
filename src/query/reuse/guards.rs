//! Candidate guards and deterministic canonical ordering shared by D1-D4.

use super::corpus::SymbolData;

/// GUARD-LANG: Python, TsJs, Rust, or the exact lowercase extension.
pub(super) fn language_family(file: &str) -> String {
    let ext = file.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "py" | "pyi" => "python".to_string(),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => "tsjs".to_string(),
        "rs" => "rust".to_string(),
        other => other.to_ascii_lowercase(),
    }
}

pub(super) fn cross_language(a: &SymbolData, b: &SymbolData) -> bool {
    language_family(&a.file_path) != language_family(&b.file_path)
}

/// Deterministic canonical ordering: more callers wins; ties use the
/// ascending (file, line, qualname) key.
pub(super) fn is_more_canonical(candidate: &SymbolData, target: &SymbolData) -> bool {
    if candidate.inbound_callers != target.inbound_callers {
        candidate.inbound_callers > target.inbound_callers
    } else {
        (
            candidate.file_path.as_str(),
            candidate.line_start,
            candidate.qualname.as_str(),
        ) < (
            target.file_path.as_str(),
            target.line_start,
            target.qualname.as_str(),
        )
    }
}

/// Numbered siblings share a stem and are a deliberate family, not twins.
pub(super) fn strip_trailing_digits(name: &str) -> &str {
    name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_')
}

/// Weighted containment of the smaller deep footprint in the larger one.
pub(super) fn weighted_containment(target: &SymbolData, candidate: &SymbolData) -> f64 {
    let target_total: f64 = target.deep_weights.values().sum();
    let candidate_total: f64 = candidate.deep_weights.values().sum();
    if target_total <= 0.0 || candidate_total <= 0.0 {
        return 0.0;
    }
    let target_shared: f64 = target
        .deep_weights
        .iter()
        .filter(|(id, _)| candidate.deep_weights.contains_key(*id))
        .map(|(_, weight)| weight)
        .sum();
    let candidate_shared: f64 = candidate
        .deep_weights
        .iter()
        .filter(|(id, _)| target.deep_weights.contains_key(*id))
        .map(|(_, weight)| weight)
        .sum();
    (target_shared / target_total).max(candidate_shared / candidate_total)
}

/// Weighted size balance: `min(total_t, total_c) / max(total_t, total_c)`.
pub(super) fn weighted_balance(target: &SymbolData, candidate: &SymbolData) -> f64 {
    let target_total: f64 = target.deep_weights.values().sum();
    let candidate_total: f64 = candidate.deep_weights.values().sum();
    if target_total <= 0.0 || candidate_total <= 0.0 {
        return 0.0;
    }
    target_total.min(candidate_total) / target_total.max(candidate_total)
}
