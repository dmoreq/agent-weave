//! Unit tests for the D1-D4 detectors and the inverted-index parity gate.

use super::*;

fn make_test_symbol(
    id: i64,
    qualname: &str,
    file_id: i64,
    callers: i64,
    callees: &[&str],
) -> SymbolData {
    SymbolData {
        id,
        name: qualname.rsplit('.').next().unwrap_or(qualname).to_string(),
        qualname: qualname.to_string(),
        file_id,
        file_path: format!("src/file_{file_id}.rs"),
        line_start: 10,
        line_end: 20,
        resolved_callees: HashSet::new(),
        all_callees: callees.iter().map(|&s| s.to_string()).collect(),
        callee_ids: HashSet::new(),
        deep_weights: HashMap::new(),
        inbound_callers: callers,
        tokens: HashSet::new(),
        type_tokens: HashSet::new(),
    }
}

#[test]
fn d3_matches_raw_client_across_rust_path_styles() {
    let mut symbols = HashMap::new();
    symbols.insert(
        1,
        make_test_symbol(1, "client.fetch", 1, 5, &["reqwest::get"]),
    );
    let target_rust = make_test_symbol(2, "app.rogue", 2, 0, &["reqwest.get"]);
    let corpus = Corpus {
        symbols: &symbols,
        index: None,
        deep_callee: false,
    };

    let candidate = check_d3(&target_rust, &corpus);
    assert!(
        candidate.is_some(),
        "must match reqwest.get with reqwest::get wrapper"
    );
    let cand = candidate.unwrap();
    assert_eq!(cand.canonical, "client.fetch");
    assert_eq!(cand.shared_items, vec!["raw_client=reqwest.get"]);

    let target_diff = make_test_symbol(3, "app.post", 2, 0, &["requests.post"]);
    assert!(check_d3(&target_diff, &corpus).is_none());
}

#[test]
fn d3_matches_exact_client_only() {
    let mut symbols = HashMap::new();
    symbols.insert(1, make_test_symbol(1, "api.get", 1, 3, &["requests.get"]));
    let target_post = make_test_symbol(2, "app.post", 2, 0, &["requests.post"]);
    let target_get = make_test_symbol(3, "app.fetch", 2, 0, &["requests.get"]);
    let corpus = Corpus {
        symbols: &symbols,
        index: None,
        deep_callee: false,
    };
    assert!(check_d3(&target_post, &corpus).is_none());

    let cand = check_d3(&target_get, &corpus).expect("should match requests.get");
    assert_eq!(cand.canonical, "api.get");
}

/// Build a symbol whose resolved callee set equals its all-callee set.
fn resolved_symbol(
    id: i64,
    qualname: &str,
    callers: i64,
    callees: &[&str],
    tokens: &[&str],
) -> SymbolData {
    let mut sym = make_test_symbol(id, qualname, id, callers, callees);
    sym.resolved_callees = sym.all_callees.clone();
    sym.tokens = tokens.iter().map(|t| t.to_string()).collect();
    sym
}

fn deep_symbol(
    id: i64,
    qualname: &str,
    direct: &[&str],
    second_hop: &[&str],
    file_id: i64,
    callers: i64,
) -> SymbolData {
    let mut sym = make_test_symbol(id, qualname, file_id, callers, direct);
    sym.resolved_callees = sym.all_callees.clone();
    for name in direct {
        sym.deep_weights.insert(name.to_string(), 1.0);
    }
    for name in second_hop {
        sym.deep_weights.entry(name.to_string()).or_insert(0.5);
    }
    sym
}

#[test]
fn deep_callee_precision_meets_the_gate() {
    // 20 two-hop wrappers (target -> mid -> canonical callees) and 20
    // disjoint controls. Precision must stay >= 85%.
    let mut detected = 0;
    let mut false_positives = 0;
    for case in 0..40 {
        let positive = case < 20;
        let (candidate_direct, target_second) = if positive {
            (&["a", "b", "c"][..], &["a", "b", "c"][..])
        } else {
            (&["x", "y", "z"][..], &["a", "b", "c"][..])
        };
        let mut symbols = HashMap::new();
        symbols.insert(1, deep_symbol(1, "t.target", &["mid"], target_second, 1, 0));
        symbols.insert(
            2,
            deep_symbol(2, "t.canonical", candidate_direct, &[], 2, 5),
        );
        let index = CorpusIndex::build(&symbols);
        let corpus = Corpus {
            symbols: &symbols,
            index: Some(&index),
            deep_callee: true,
        };
        let matched = find_best_d1(&symbols[&1], &corpus).is_some();
        if positive && matched {
            detected += 1;
        }
        if !positive && matched {
            false_positives += 1;
        }
    }
    let precision = detected as f64 / (detected + false_positives) as f64;
    assert!(detected >= 20, "positive detection {detected}/20");
    assert!(precision >= 0.85, "precision {precision:.2} below gate");
}

#[test]
#[ignore = "release gate: cargo test --release --lib detector_scan_gate -- --ignored --nocapture"]
fn detector_scan_gate_on_5000_symbols() {
    use std::time::{Duration, Instant};
    let mut symbols: HashMap<i64, SymbolData> = HashMap::new();
    // 1,500 helpers, 1,000 pipelines (500 duplicate pairs), 2,500 filler.
    for group in 0..500_i64 {
        for helper in 0..3_i64 {
            let id = group * 10 + helper;
            symbols.insert(
                id,
                resolved_symbol(
                    id,
                    &format!("h{group}_{helper}"),
                    2,
                    &[],
                    &[&format!("h{group}"), &helper.to_string()],
                ),
            );
        }
        for variant in 0..2_i64 {
            let id = group * 10 + 3 + variant;
            symbols.insert(
                id,
                resolved_symbol(
                    id,
                    &format!("pipeline_{group}_{variant}"),
                    variant,
                    &[
                        &format!("h{group}_0"),
                        &format!("h{group}_1"),
                        &format!("h{group}_2"),
                    ],
                    &["pipeline"],
                ),
            );
        }
    }
    for i in 0..2_500_i64 {
        let id = 10_000 + i;
        symbols.insert(
            id,
            resolved_symbol(id, &format!("filler_{i}"), 0, &[], &["filler"]),
        );
    }
    assert_eq!(symbols.len(), 5_000);
    let index = CorpusIndex::build(&symbols);
    let corpus = Corpus {
        symbols: &symbols,
        index: Some(&index),
        deep_callee: false,
    };
    let targets: Vec<&SymbolData> = symbols.values().collect();
    let start = Instant::now();
    for target in &targets {
        for detect in DETECTORS {
            if detect(target, &corpus).is_some() {
                break;
            }
        }
    }
    let elapsed = start.elapsed();
    println!("detector scan on 5,000 symbols: {elapsed:?}");
    assert!(
        elapsed < Duration::from_millis(15),
        "C1 detector-phase gate: {elapsed:?} >= 15ms"
    );
}

#[test]
fn inverted_index_preserves_detector_results() {
    let mut symbols = HashMap::new();
    for (id, name, callees, tokens) in [
        (
            1,
            "a.pipeline",
            &["helper_one", "helper_two", "helper_three"][..],
            &["parse", "token"][..],
        ),
        (
            2,
            "b.pipeline",
            &["helper_one", "helper_two", "helper_three"][..],
            &["parse", "token"][..],
        ),
        (
            3,
            "c.other",
            &["helper_one", "helper_two", "different"][..],
            &["parse", "other"][..],
        ),
        (4, "d.single", &["helper_one", "only"][..], &["single"][..]),
    ] {
        symbols.insert(id, resolved_symbol(id, name, 0, callees, tokens));
    }
    let index = CorpusIndex::build(&symbols);
    let flat = Corpus {
        symbols: &symbols,
        index: None,
        deep_callee: false,
    };
    let indexed = Corpus {
        symbols: &symbols,
        index: Some(&index),
        deep_callee: false,
    };
    for target in symbols.values() {
        let flat_d1 = find_best_d1(target, &flat).map(|c| c.format_text());
        let indexed_d1 = find_best_d1(target, &indexed).map(|c| c.format_text());
        assert_eq!(flat_d1, indexed_d1, "D1 parity for {}", target.qualname);
        let flat_d2 = find_best_d2(target, &flat).map(|c| c.format_text());
        let indexed_d2 = find_best_d2(target, &indexed).map(|c| c.format_text());
        assert_eq!(flat_d2, indexed_d2, "D2 parity for {}", target.qualname);
    }
}
