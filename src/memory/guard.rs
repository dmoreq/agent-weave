//! Ambient memory retrieval: co-retrieve active gotchas/decisions
//! referencing target definitions, files, or symbols.

use std::collections::HashSet;
use std::path::Path;

/// Similarity threshold at which `mem write` suggests consolidating through
/// `--supersedes` instead of creating a near-duplicate note.
pub const CONSOLIDATION_THRESHOLD_PCT: u64 = 80;

/// One near-duplicate note found on the write path.
#[derive(Debug, Clone)]
pub struct ConsolidationMatch {
    pub key: String,
    pub rev: i64,
    pub similarity_pct: u64,
}

impl ConsolidationMatch {
    /// Advisory line appended to `mem write` stdout (CLI and MCP share it).
    pub fn hint_line(&self, new_key: &str) -> String {
        format!(
            "eg: mem hint: '{new_key}' is {}% similar to existing '{}' rev {} — consider --supersedes {}",
            self.similarity_pct, self.key, self.rev, self.key
        )
    }
}

/// English function words that carry no note identity. Removing them keeps a
/// paraphrase above the consolidation threshold without matching unrelated
/// prose on glue words alone.
const CONSOLIDATION_STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "been", "being", "but", "by", "for", "from", "if",
    "in", "into", "is", "it", "its", "no", "not", "of", "on", "or", "over", "than", "that", "the",
    "then", "this", "to", "was", "we", "were", "when", "where", "which", "while", "with",
];

/// Token set used by the consolidation guard: body, refs, and tags through the
/// single memory tokenizer, so the same words that rank a query also drive the
/// duplicate signal.
fn consolidation_tokens(body: &str, refs: &str, tags: &str) -> HashSet<String> {
    let mut set: HashSet<String> = crate::memory::mem_tokens(body)
        .into_iter()
        .filter(|token| !CONSOLIDATION_STOPWORDS.contains(&token.as_str()))
        .collect();
    set.extend(crate::memory::mem_ref_tag_tokens(refs, tags));
    set
}

/// Jaccard similarity in percent between a candidate note and one stored row.
/// Both sides pass through [`consolidation_tokens`]; 0 when either is empty.
pub fn consolidation_similarity(
    note: &crate::memory::NormalizedNote,
    row: &crate::db::MemRow,
) -> u64 {
    let left = consolidation_tokens(&note.body, &note.refs, &note.tags);
    let right = consolidation_tokens(&row.body, &row.refs, &row.tags);
    if left.is_empty() || right.is_empty() {
        return 0;
    }
    let shared = left.intersection(&right).count();
    let union = left.union(&right).count();
    (shared * 100).checked_div(union).unwrap_or(0) as u64
}

/// Search the current notes for the closest near-duplicate of a pending
/// write. The same key is excluded (same-key updates use the revision chain)
/// and `--supersedes` already expresses the user's intent, so the caller skips
/// this check in that case.
///
/// Candidate generation uses the memory FTS index (OR terms, top 32) instead
/// of scanning every current note: the guard runs at the `mem write` trust
/// boundary, where tokenizing the whole store would blow the 2 ms budget.
pub fn find_consolidation_match(
    conn: &rusqlite::Connection,
    note: &crate::memory::NormalizedNote,
) -> Option<ConsolidationMatch> {
    let mut terms = crate::memory::mem_tokens(&note.body);
    terms.extend(crate::memory::mem_tokens(&note.tags));
    if terms.is_empty() {
        return None;
    }
    let query = terms.into_iter().take(32).collect::<Vec<_>>().join(" OR ");
    let ids: Vec<i64> = crate::db::memory_fts_candidates(conn, &query, 32)
        .ok()?
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    if ids.is_empty() {
        return None;
    }
    let filter = crate::db::MemFilter {
        kind: None,
        scope: None,
        now: crate::db::unix_now(),
        include_superseded: false,
    };
    crate::db::memory_rows_by_ids(conn, &ids)
        .ok()?
        .into_iter()
        .filter(|row| row.key != note.key && crate::db::memory_row_is_current(row, &filter))
        .filter_map(|row| {
            let similarity_pct = consolidation_similarity(note, &row);
            (similarity_pct >= CONSOLIDATION_THRESHOLD_PCT).then_some(ConsolidationMatch {
                key: row.key,
                rev: row.rev,
                similarity_pct,
            })
        })
        .max_by(|a, b| {
            a.similarity_pct
                .cmp(&b.similarity_pct)
                .then_with(|| b.key.cmp(&a.key))
        })
}

/// Compact guarded note info for ambient retrieval.
#[derive(Debug, Clone)]
pub struct GuardedNote {
    pub key: String,
    pub kind: String,
    pub summary: String,
    pub target: String,
}

/// Check if a note's `refs` matches a target path or symbol.
pub fn ref_matches_target(refs: &str, target: &str) -> bool {
    let target = target.trim();
    if target.is_empty() {
        return false;
    }
    let target_norm = target.trim_start_matches("./");
    refs.split_whitespace().any(|r| {
        let r_norm = r.trim_start_matches("./");
        r_norm == target_norm
            || r_norm.ends_with(&format!("/{target_norm}"))
            || target_norm.ends_with(&format!("/{r_norm}"))
            || r_norm.ends_with(&format!("::{target_norm}"))
            || target_norm.ends_with(&format!("::{r_norm}"))
            || r_norm.ends_with(&format!(".{target_norm}"))
            || target_norm.ends_with(&format!(".{r_norm}"))
            || target_norm.split("::").any(|seg| seg == r_norm)
            || target_norm.split('.').any(|seg| seg == r_norm)
            || r_norm.split("::").any(|seg| seg == target_norm)
            || r_norm.split('.').any(|seg| seg == target_norm)
    })
}

/// First line of body, capped safely at 80 characters.
pub fn compact_summary(body: &str) -> String {
    let first_line = body.lines().next().unwrap_or("").trim();
    if first_line.chars().count() > 80 {
        let truncated: String = first_line.chars().take(77).collect();
        format!("{truncated}...")
    } else {
        first_line.to_string()
    }
}

/// Find active gotchas and decisions in local and global memory stores
/// that reference any of the provided paths or symbols.
pub fn find_guarding_notes(root: &Path, paths: &[&str], symbols: &[&str]) -> Vec<GuardedNote> {
    let now = crate::db::unix_now();
    let filter = crate::db::MemFilter {
        kind: None,
        scope: None,
        now,
        include_superseded: false,
    };

    let mut results = Vec::new();
    let mut seen_keys = HashSet::new();

    if let Ok(local_conn) = crate::db::open_memory_query(root) {
        collect_guards(
            &local_conn,
            &filter,
            paths,
            symbols,
            &mut seen_keys,
            &mut results,
        );
    }

    if let Ok(global_conn) = crate::db::open_global_memory_query() {
        collect_guards(
            &global_conn,
            &filter,
            paths,
            symbols,
            &mut seen_keys,
            &mut results,
        );
    }

    results
}

fn collect_guards(
    conn: &rusqlite::Connection,
    filter: &crate::db::MemFilter,
    paths: &[&str],
    symbols: &[&str],
    seen_keys: &mut HashSet<String>,
    out: &mut Vec<GuardedNote>,
) {
    let Ok(rows) = crate::db::memory_current_rows(conn, filter, None) else {
        return;
    };
    for row in rows {
        if (row.kind != "decision" && row.kind != "gotcha") || row.refs.is_empty() {
            continue;
        }
        if seen_keys.contains(&row.key) {
            continue;
        }
        let matched = paths
            .iter()
            .chain(symbols.iter())
            .find(|target| ref_matches_target(&row.refs, target));
        if let Some(target) = matched {
            seen_keys.insert(row.key.clone());
            out.push(GuardedNote {
                key: row.key,
                kind: row.kind,
                summary: compact_summary(&row.body),
                target: target.to_string(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(body: &str, refs: &str, tags: &str) -> crate::memory::NormalizedNote {
        crate::memory::NormalizedNote {
            key: "new-note".into(),
            kind: crate::memory::MemKind::Fact,
            tags: tags.into(),
            refs: refs.into(),
            scope: crate::memory::MemScope::Durable,
            body: body.into(),
            ttl_secs: 0,
        }
    }

    fn row(key: &str, body: &str, refs: &str, tags: &str) -> crate::db::MemRow {
        crate::db::MemRow {
            id: 1,
            key: key.into(),
            rev: 1,
            kind: "fact".into(),
            scope: "durable".into(),
            session: String::new(),
            body: body.into(),
            tags: tags.into(),
            refs: refs.into(),
            source: "cli".into(),
            created_at: 1,
            expires_at: 0,
            superseded_by: None,
        }
    }

    #[test]
    #[ignore = "release gate: cargo test --release --lib consolidation_scan -- --ignored --nocapture"]
    fn consolidation_scan_meets_2ms_on_256_notes() {
        use std::time::{Duration, Instant};
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../schema/memory.sql"))
            .unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        for i in 0..256_i64 {
            let key = format!("note-{i}");
            let body = format!(
                "resolver keeps confidence tiers over resolved edges with invariant {i} and stable behavior"
            );
            let id: i64 = tx
                .query_row(
                    "INSERT INTO memories
                     (key, rev, kind, scope, session, body, tags, refs, content_hash, source, created_at, expires_at)
                     VALUES (?1, 1, 'fact', 'durable', '', ?2, 'resolver', 'src/query.rs', 0, 'cli', 1, 0)
                     RETURNING id",
                    rusqlite::params![key, body],
                    |r| r.get(0),
                )
                .unwrap();
            tx.execute(
                "INSERT INTO memories_fts(rowid, text) VALUES (?1, ?2)",
                rusqlite::params![id, crate::memory::mem_fts_text(&key, "resolver", &body)],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        let candidate = note(
            "the resolver keeps confidence tiers on resolved edges with stable invariants",
            "src/query.rs",
            "resolver",
        );
        let start = Instant::now();
        let found = find_consolidation_match(&conn, &candidate).is_some();
        let elapsed = start.elapsed();
        println!("consolidation scan on 256 notes: {elapsed:?} (match={found})");
        assert!(
            elapsed < Duration::from_millis(2),
            "C3 latency gate: {elapsed:?} >= 2ms"
        );
    }

    #[test]
    fn identical_notes_score_one_hundred() {
        let note = note(
            "resolver keeps confidence tiers over edges",
            "src/query.rs",
            "resolver",
        );
        let row = row(
            "old",
            "resolver keeps confidence tiers over edges",
            "src/query.rs",
            "resolver",
        );
        assert_eq!(consolidation_similarity(&note, &row), 100);
    }

    #[test]
    fn paraphrase_clears_the_threshold_while_unrelated_prose_does_not() {
        let candidate = note(
            "the resolver keeps confidence tiers over resolved edges",
            "src/query/callgraph.rs",
            "resolver",
        );
        let paraphrase = row(
            "old",
            "resolver keeps confidence tiers on resolved edges",
            "src/query/callgraph.rs",
            "resolver",
        );
        assert!(consolidation_similarity(&candidate, &paraphrase) >= CONSOLIDATION_THRESHOLD_PCT);

        let unrelated = row(
            "other",
            "dashboard renders latency sparklines with ANSI colors",
            "src/dashboard.rs",
            "telemetry",
        );
        assert!(consolidation_similarity(&candidate, &unrelated) < 20);
    }

    #[test]
    fn synthetic_pair_gate_meets_acceptance_thresholds() {
        // 100 paraphrase pairs (one glue word changed) against 100 unrelated
        // pairs: detection must stay >= 80% and false alarms <= 10%.
        let base = [
            "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
        ];
        let mut detected = 0;
        let mut false_alarms = 0;
        for i in 0..100 {
            let body = base.join(" ");
            let variant = body.replace(base[i % base.len()], "lambda");
            let candidate = note(&format!("{body} the note"), "", "");
            let stored = row("old", &format!("{variant} a note"), "", "");
            if consolidation_similarity(&candidate, &stored) >= CONSOLIDATION_THRESHOLD_PCT {
                detected += 1;
            }

            let unrelated_body = [
                "omega", "sigma", "tau", "upsilon", "phi", "chi", "psi", "rho", "nu", "mu",
            ]
            .join(" ");
            let unrelated = row("other", &unrelated_body, "", "");
            if consolidation_similarity(&candidate, &unrelated) >= CONSOLIDATION_THRESHOLD_PCT {
                false_alarms += 1;
            }
        }
        assert!(detected >= 80, "detection rate {detected}% below gate");
        assert!(
            false_alarms <= 10,
            "false alarms {false_alarms}% above gate"
        );
    }
}
