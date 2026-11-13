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
    /// Resolved production callee IDs (D1's two-callee gate).
    Callee,
    /// Weighted 1-hop/2-hop callee IDs (D1 `--deep-callee`).
    DeepCallee,
    /// Interface signature/name token pairs (D2's two-token gate). Indexing
    /// pairs instead of single tokens keeps the exact `>= 2 shared tokens`
    /// predicate while avoiding a scan of every symbol that carries one
    /// frequent token such as `filler`.
    TokenPair,
}

/// One posting list per signal key: `key -> symbol ids`.
#[derive(Debug, Default)]
pub(super) struct CorpusIndex {
    by_callee: HashMap<i64, Vec<i64>>,
    by_deep_callee: HashMap<i64, Vec<i64>>,
    /// Representative deep key per posting-group, plus key -> group lookup.
    /// Many two-hop leaves share an identical caller set; grouping collapses
    /// their union cost while preserving the exact shared-key counts.
    deep_group_rep: Vec<i64>,
    deep_key_group: HashMap<i64, usize>,
    by_token_pair: HashMap<(String, String), Vec<i64>>,
    by_raw_client: HashMap<String, Vec<i64>>,
}

impl CorpusIndex {
    /// Build the posting lists in one pass over the corpus.
    pub(super) fn build(symbols: &HashMap<i64, SymbolData>) -> Self {
        let mut index = CorpusIndex::default();
        for sym in symbols.values() {
            for callee in &sym.callee_ids {
                // Post only IDs that can satisfy D1's C_in predicate. A
                // stoplisted ubiquitous callee would otherwise create an
                // N-length posting shared by every caller, making each target
                // scan the whole corpus before the detector filters it out.
                if symbols.get(callee).is_some_and(|callee_sym| {
                    !super::guards::cross_language(sym, callee_sym)
                        && sym.resolved_callees.contains(&callee_sym.name)
                }) {
                    index.by_callee.entry(*callee).or_default().push(sym.id);
                }
            }
            for callee in sym.deep_weights.keys() {
                index
                    .by_deep_callee
                    .entry(*callee)
                    .or_default()
                    .push(sym.id);
            }
            let mut tokens: Vec<&String> = sym.name_tokens.iter().collect();
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
            // D3 only accepts a wrapper with at least two callers; indexing
            // low-caller symbols would make every raw-client user a candidate.
            if sym.inbound_callers >= 2 {
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
        }
        // Group deep keys whose posting lists are identical. A target with
        // 2,000 shared leaves then walks one 400-symbol list, not 2,000.
        let mut deep_keys: Vec<i64> = index.by_deep_callee.keys().copied().collect();
        deep_keys.sort_unstable();
        let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
        for key in deep_keys {
            let hash = hash_ids(&index.by_deep_callee[&key]);
            let existing = buckets.get(&hash).and_then(|groups| {
                groups.iter().copied().find(|&group| {
                    index.by_deep_callee[&index.deep_group_rep[group]] == index.by_deep_callee[&key]
                })
            });
            let group = match existing {
                Some(group) => group,
                None => {
                    let group = index.deep_group_rep.len();
                    index.deep_group_rep.push(key);
                    buckets.entry(hash).or_default().push(group);
                    group
                }
            };
            index.deep_key_group.insert(key, group);
        }
        // D3 picks the most canonical wrapper. Sort each raw-client posting
        // best-first so a target can stop at the first eligible candidate
        // instead of scanning every wrapper sharing the client.
        for postings in index.by_raw_client.values_mut() {
            postings.sort_by(|a, b| {
                let left = &symbols[a];
                let right = &symbols[b];
                right
                    .inbound_callers
                    .cmp(&left.inbound_callers)
                    .then_with(|| left.file_path.cmp(&right.file_path))
                    .then_with(|| left.line_start.cmp(&right.line_start))
                    .then_with(|| left.qualname.cmp(&right.qualname))
            });
            postings.dedup();
        }
        index
    }

    /// Raw-client posting list, best canonical first.
    pub(super) fn raw_candidates(&self, client: &str) -> Option<&[i64]> {
        self.by_raw_client.get(client).map(Vec::as_slice)
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
                for key in &target.callee_ids {
                    add_postings(self.by_callee.get(key), &mut counts);
                }
            }
            PostingKind::DeepCallee => {
                let mut groups: HashMap<usize, usize> = HashMap::new();
                for key in target.deep_weights.keys() {
                    if let Some(group) = self.deep_key_group.get(key) {
                        *groups.entry(*group).or_default() += 1;
                    }
                }
                for (group, multiplicity) in groups {
                    let Some(rep) = self.deep_group_rep.get(group) else {
                        continue;
                    };
                    if let Some(ids) = self.by_deep_callee.get(rep) {
                        for id in ids {
                            *counts.entry(*id).or_default() += multiplicity;
                        }
                    }
                }
            }
            PostingKind::TokenPair => {
                let mut tokens: Vec<&String> = target.name_tokens.iter().collect();
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

fn hash_ids(ids: &[i64]) -> u64 {
    ids.iter().fold(0xcbf2_9ce4_8422_2325, |hash, id| {
        (hash ^ (*id as u64)).wrapping_mul(0x0000_0100_0000_01b3)
    })
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

    fn callee_key(name: &str) -> i64 {
        name.bytes()
            .fold(0i64, |acc, byte| acc * 31 + i64::from(byte))
    }

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
            unresolved_callees: HashSet::new(),
            callee_ids: callees.iter().map(|name| callee_key(name)).collect(),
            deep_weights: HashMap::new(),
            inbound_callers: 0,
            tokens: tokens.iter().map(|s| s.to_string()).collect(),
            type_tokens: HashSet::new(),
            name_tokens: tokens.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn callee_symbol(name: &str) -> SymbolData {
        let mut symbol = symbol_with(callee_key(name), &[], &[]);
        symbol.name = name.to_string();
        symbol.qualname = format!("m.{name}");
        symbol
    }

    #[test]
    fn callee_posting_requires_two_shared_keys() {
        let mut symbols = HashMap::new();
        symbols.insert(1, symbol_with(1, &["a", "b", "c"], &[]));
        symbols.insert(2, symbol_with(2, &["a", "b", "x"], &[]));
        symbols.insert(3, symbol_with(3, &["a", "c"], &[]));
        symbols.insert(4, symbol_with(4, &["a", "d"], &[]));
        for name in ["a", "b", "c", "d", "x"] {
            symbols.insert(callee_key(name), callee_symbol(name));
        }
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
        for id in 1..=3 {
            let callee = match id {
                1 => "reqwest::get",
                2 => "reqwest.get",
                _ => "requests.get",
            };
            let mut symbol = symbol_with(id, &[callee], &[]);
            symbol.inbound_callers = 2;
            symbols.insert(id, symbol);
        }
        let index = CorpusIndex::build(&symbols);
        let mut ids = index
            .raw_candidates("reqwest.get")
            .unwrap_or_default()
            .to_vec();
        ids.retain(|id| *id != 1);
        assert_eq!(ids, vec![2]);
    }
}
