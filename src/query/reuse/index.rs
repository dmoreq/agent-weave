//! Inverted posting lists for the reuse detectors.
//!
//! The flat detector cascade scans every corpus symbol once per target, which
//! is O(N × M) on large repositories. This module indexes the signals the
//! detectors actually consume — resolved callees, the weighted 1-hop/2-hop
//! footprint, interface token pairs, and raw external clients — so a target
//! only visits the symbols that share its minimum signal keys. The flat scan
//! remains the default below [`INVERTED_INDEX_MIN_SYMBOLS`] because building
//! the index costs more than it saves on small corpora.

use std::collections::HashMap;

use super::corpus::SymbolData;
use super::detectors::is_raw_client;

/// Corpora at or below this size keep the allocation-free flat scan.
pub(super) const INVERTED_INDEX_MIN_SYMBOLS: usize = 1_000;

/// Which posting list a detector needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PostingKind {
    /// Resolved callee names (D1's two-resolved-callee gate).
    Callee,
    /// Weighted 1-hop/2-hop callee footprint (D1 `--deep-callee`).
    DeepCallee,
    /// Interface signature/name token pairs (D2's two-token gate). Indexing
    /// pairs instead of single tokens keeps the exact `>= 2 shared tokens`
    /// predicate while avoiding a scan of every symbol that carries one
    /// frequent token such as `filler`.
    TokenPair,
    /// Raw external client names (D3's one-client gate).
    RawClient,
}

/// One posting list per signal key: `key -> symbol ids`.
#[derive(Debug, Default)]
pub(super) struct CorpusIndex {
    by_callee: HashMap<String, Vec<i64>>,
    by_deep_callee: HashMap<String, Vec<i64>>,
    by_token_pair: HashMap<(String, String), Vec<i64>>,
    by_raw_client: HashMap<String, Vec<i64>>,
}

impl CorpusIndex {
    /// Build the posting lists in one pass over the corpus.
    pub(super) fn build(symbols: &HashMap<i64, SymbolData>) -> Self {
        let mut index = CorpusIndex::default();
        for sym in symbols.values() {
            for callee in &sym.resolved_callees {
                index
                    .by_callee
                    .entry(callee.clone())
                    .or_default()
                    .push(sym.id);
            }
            for callee in sym.deep_weights.keys() {
                index
                    .by_deep_callee
                    .entry(callee.clone())
                    .or_default()
                    .push(sym.id);
            }
            let mut tokens: Vec<&String> = sym.tokens.iter().collect();
            tokens.sort_unstable();
            for (i, left) in tokens.iter().enumerate() {
                for right in &tokens[i + 1..] {
                    index
                        .by_token_pair
                        .entry(((*left).clone(), (*right).clone()))
                        .or_default()
                        .push(sym.id);
                }
            }
            for callee in &sym.all_callees {
                if is_raw_client(callee) {
                    index
                        .by_raw_client
                        .entry(normalize_client(callee))
                        .or_default()
                        .push(sym.id);
                }
            }
        }
        index
    }

    /// Symbol ids sharing at least `min_shared` keys with `target` for the
    /// requested signal. The caller's detector re-checks its exact predicate;
    /// this list is a conservative superset of every possible match.
    pub(super) fn candidates(
        &self,
        target: &SymbolData,
        kind: PostingKind,
        min_shared: usize,
    ) -> Vec<i64> {
        let mut counts: HashMap<i64, usize> = HashMap::new();
        match kind {
            PostingKind::Callee => {
                for key in &target.resolved_callees {
                    add_postings(self.by_callee.get(key), &mut counts);
                }
            }
            PostingKind::DeepCallee => {
                for key in target.deep_weights.keys() {
                    add_postings(self.by_deep_callee.get(key), &mut counts);
                }
            }
            PostingKind::TokenPair => {
                let mut tokens: Vec<&String> = target.tokens.iter().collect();
                tokens.sort_unstable();
                for (i, left) in tokens.iter().enumerate() {
                    for right in &tokens[i + 1..] {
                        add_postings(
                            self.by_token_pair.get(&((*left).clone(), (*right).clone())),
                            &mut counts,
                        );
                    }
                }
            }
            PostingKind::RawClient => {
                for key in target
                    .all_callees
                    .iter()
                    .filter(|c| is_raw_client(c))
                    .map(|c| normalize_client(c))
                {
                    add_postings(self.by_raw_client.get(&key), &mut counts);
                }
            }
        }
        counts
            .into_iter()
            .filter(|(_, shared)| *shared >= min_shared)
            .map(|(id, _)| id)
            .collect()
    }
}

/// The raw-client spelling used by both index keys and lookups: `reqwest::get`
/// and `reqwest.get` are the same client.
pub(super) fn normalize_client(client: &str) -> String {
    client.replace("::", ".")
}

fn add_postings(postings: Option<&Vec<i64>>, counts: &mut HashMap<i64, usize>) {
    if let Some(ids) = postings {
        for id in ids {
            *counts.entry(*id).or_default() += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn symbol_with(id: i64, callees: &[&str], tokens: &[&str]) -> SymbolData {
        SymbolData {
            id,
            name: format!("s{id}"),
            qualname: format!("m.s{id}"),
            file_id: id,
            file_path: format!("src/f{id}.rs"),
            line_start: 1,
            line_end: 2,
            resolved_callees: callees.iter().map(|s| s.to_string()).collect(),
            all_callees: callees.iter().map(|s| s.to_string()).collect(),
            callee_ids: HashSet::new(),
            deep_weights: HashMap::new(),
            inbound_callers: 0,
            tokens: tokens.iter().map(|s| s.to_string()).collect(),
            type_tokens: HashSet::new(),
        }
    }

    #[test]
    fn callee_posting_requires_two_shared_keys() {
        let mut symbols = HashMap::new();
        symbols.insert(1, symbol_with(1, &["a", "b", "c"], &[]));
        symbols.insert(2, symbol_with(2, &["a", "b", "x"], &[]));
        symbols.insert(3, symbol_with(3, &["a", "c"], &[]));
        symbols.insert(4, symbol_with(4, &["a", "d"], &[]));
        let index = CorpusIndex::build(&symbols);
        let target = &symbols[&1];
        let mut ids = index.candidates(target, PostingKind::Callee, 2);
        ids.retain(|id| *id != target.id);
        ids.sort_unstable();
        assert_eq!(ids, vec![2, 3], "only two-shared-callee symbols");
    }

    #[test]
    fn token_posting_requires_two_shared_tokens() {
        let mut symbols = HashMap::new();
        symbols.insert(1, symbol_with(1, &[], &["parse", "token", "alpha"]));
        symbols.insert(2, symbol_with(2, &[], &["parse", "token", "beta"]));
        symbols.insert(3, symbol_with(3, &[], &["parse", "gamma"]));
        let index = CorpusIndex::build(&symbols);
        let mut ids = index.candidates(&symbols[&1], PostingKind::TokenPair, 1);
        ids.retain(|id| *id != 1);
        assert_eq!(ids, vec![2]);
    }

    #[test]
    fn raw_client_posting_normalizes_path_separators() {
        let mut symbols = HashMap::new();
        symbols.insert(1, symbol_with(1, &["reqwest::get"], &[]));
        symbols.insert(2, symbol_with(2, &["reqwest.get"], &[]));
        symbols.insert(3, symbol_with(3, &["requests.get"], &[]));
        let index = CorpusIndex::build(&symbols);
        let mut ids = index.candidates(&symbols[&1], PostingKind::RawClient, 1);
        ids.retain(|id| *id != 1);
        assert_eq!(ids, vec![2]);
    }
}
