//! Phase 3 opt-in allowlist. A current memory note with key
//! `reuse-allow/<target>-><canonical>` suppresses an intentional duplicate.
//! High-confidence pairs are never suppressed. Notes with refs are ignored
//! once any referenced file changed since the index build.

use std::collections::HashSet;

use super::*;

pub(super) fn load(ctx: &QueryCtx<'_>) -> HashSet<(String, String)> {
    let Ok(conn) = crate::db::open_memory_query(ctx.root) else {
        return HashSet::new();
    };
    let filter = crate::db::MemFilter {
        kind: None,
        scope: None,
        now: crate::db::unix_now(),
        include_superseded: false,
        window: crate::memory::TimeWindow {
            since: None,
            until: None,
        },
    };
    let Ok(rows) = crate::db::memory_current_rows(&conn, &filter, None) else {
        return HashSet::new();
    };
    let mut pairs = HashSet::new();
    for row in rows {
        let Some(pair) = row.key.strip_prefix("reuse-allow/") else {
            continue;
        };
        let Some((target, canonical)) = pair.split_once("->") else {
            continue;
        };
        let refs: Vec<&str> = row.refs.split_whitespace().collect();
        if !refs.is_empty() && !refs.iter().all(|path| ref_is_fresh(ctx, path)) {
            continue;
        }
        pairs.insert((target.to_string(), canonical.to_string()));
    }
    pairs
}

fn ref_is_fresh(ctx: &QueryCtx<'_>, path: &str) -> bool {
    if !crate::query::staleness::safe_index_path(path) {
        return false;
    }
    let indexed: Option<i64> = ctx
        .conn
        .query_row("SELECT id FROM files WHERE path = ?1", [path], |row| {
            row.get(0)
        })
        .ok();
    indexed.is_some() && !crate::query::staleness::file_changed(ctx, path)
}

pub(super) fn is_allowed(pairs: &HashSet<(String, String)>, candidate: &ReuseCandidate) -> bool {
    candidate.confidence != ReuseConfidence::High
        && pairs.contains(&(candidate.symbol.clone(), candidate.canonical.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(confidence: ReuseConfidence) -> ReuseCandidate {
        ReuseCandidate {
            detector: DetectorId::D2,
            symbol: "b.parse".into(),
            file: "b.py".into(),
            line: 1,
            canonical: "a.parse".into(),
            canonical_file: "a.py".into(),
            canonical_line: 1,
            canonical_callers: 0,
            overlap_pct: Some(100),
            shared_items: Vec::new(),
            raw_client: None,
            confidence,
            reasons: Vec::new(),
            freshness: Freshness::Fresh,
        }
    }

    #[test]
    fn high_confidence_is_never_suppressed() {
        let pairs = HashSet::from([("b.parse".into(), "a.parse".into())]);
        assert!(!is_allowed(&pairs, &candidate(ReuseConfidence::High)));
        assert!(is_allowed(&pairs, &candidate(ReuseConfidence::Medium)));
        assert!(is_allowed(&pairs, &candidate(ReuseConfidence::Low)));
    }
}
