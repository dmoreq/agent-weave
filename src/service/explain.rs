//! Causal `why:` labels for ranked memory results. Labels are fixed strings
//! derived from the already-computed candidate signals; raw bm25 values,
//! normalized scores, weights, and note bodies never leave this module.

use std::collections::{BTreeSet, HashMap};

use crate::memory::Candidate;

/// One explanation per `(is_global, row id)` pair. Store identity is part of
/// the key because local and global row ids are independent sequences.
pub(super) fn build(
    candidates: &[Candidate],
    query_terms: &str,
    now: i64,
    index_present: bool,
    legacy: bool,
) -> HashMap<(bool, i64), String> {
    let query: BTreeSet<String> = if legacy {
        crate::memory::mem_tokens_legacy(query_terms)
    } else {
        crate::memory::mem_tokens(query_terms)
    }
    .into_iter()
    .collect();
    let mut map = HashMap::new();
    for candidate in candidates {
        let mut labels: Vec<String> = Vec::new();
        if candidate.from_fts {
            labels.push("lexical match".to_string());
        }
        if let Some(reference) = matching_ref(candidate, &query, legacy) {
            if crate::memory::secret::suspected_secret(&reference).is_some() {
                labels.push("ref <redacted>".to_string());
            } else {
                labels.push(format!("ref {}", truncate(&reference, 60)));
            }
        } else if let Some(tag) = matching_tag(candidate, &query, legacy) {
            labels.push(format!("tag {}", truncate(&tag, 40)));
        }
        if index_present && candidate.graph_raw > 0.0 {
            let depth = (-candidate.graph_raw.log2()).round().clamp(1.0, 3.0) as i32;
            labels.push(format!("graph depth {depth}"));
        }
        let age_days = (now - candidate.row.created_at).max(0) / 86_400;
        labels.push(format!("age {age_days}d"));
        map.insert((candidate.is_global, candidate.row.id), labels.join("; "));
    }
    map
}

fn matching_ref(candidate: &Candidate, query: &BTreeSet<String>, legacy: bool) -> Option<String> {
    candidate.row.refs.split_whitespace().find_map(|reference| {
        let tokens: BTreeSet<String> = if legacy {
            crate::memory::mem_tokens_legacy(reference)
        } else {
            crate::memory::mem_ref_tokens(reference)
        }
        .into_iter()
        .collect();
        (!tokens.is_disjoint(query)).then(|| reference.to_string())
    })
}

fn matching_tag(candidate: &Candidate, query: &BTreeSet<String>, legacy: bool) -> Option<String> {
    candidate.row.tags.split_whitespace().find_map(|tag| {
        let tokens: BTreeSet<String> = if legacy {
            crate::memory::mem_tokens_legacy(tag)
        } else {
            crate::memory::mem_tokens(tag)
        }
        .into_iter()
        .collect();
        (!tokens.is_disjoint(query)).then(|| tag.to_string())
    })
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let head: String = text.chars().take(max_chars.saturating_sub(3)).collect();
    format!("{head}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(from_fts: bool, graph_raw: f64, refs: &str, tags: &str) -> Candidate {
        Candidate {
            row: crate::db::MemRow {
                id: 7,
                key: "note".into(),
                rev: 1,
                kind: "fact".into(),
                scope: "durable".into(),
                session: String::new(),
                body: "resolver".into(),
                tags: tags.into(),
                refs: refs.into(),
                source: "cli".into(),
                created_at: 1_000,
                expires_at: 0,
                superseded_by: None,
            },
            lex_raw: 1.0,
            ref_raw: 0.5,
            graph_raw,
            age_days: 0.0,
            score: 0.0,
            is_global: false,
            from_fts,
        }
    }

    #[test]
    fn labels_are_causal_and_ordered() {
        let map = build(
            &[candidate(true, 0.5, "src/memory/rank.rs", "")],
            "resolver rank",
            1_000,
            true,
            false,
        );
        assert_eq!(
            map[&(false, 7)],
            "lexical match; ref src/memory/rank.rs; graph depth 1; age 0d"
        );
    }

    #[test]
    fn graph_label_requires_an_index() {
        let map = build(
            &[candidate(true, 0.5, "", "")],
            "resolver",
            1_000,
            false,
            false,
        );
        assert!(!map[&(false, 7)].contains("graph"));
    }

    #[test]
    fn secret_ref_is_redacted() {
        let secret = format!("src/sk-{}", "a".repeat(24));
        let map = build(
            &[candidate(false, 0.0, &secret, "")],
            "sk",
            1_000,
            false,
            false,
        );
        assert!(map[&(false, 7)].contains("ref <redacted>"));
    }
}
