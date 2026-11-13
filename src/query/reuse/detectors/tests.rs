//! Unit tests for the D1-D4 detectors and the inverted-index parity gate.

use super::*;
use std::cell::Cell;

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
        unresolved_callees: HashSet::new(),
        callee_ids: HashSet::new(),
        deep_weights: HashMap::new(),
        inbound_callers: callers,
        tokens: HashSet::new(),
        type_tokens: HashSet::new(),
        name_tokens: HashSet::new(),
    }
}

#[test]
fn d2_rejects_signature_only_twins() {
    let signature_tokens = ["candidate", "corpus", "option", "reuse", "symbol", "target"];
    let mut canonical = make_test_symbol(1, "mod.check_d3", 1, 5, &[]);
    canonical.tokens = signature_tokens.iter().map(|t| t.to_string()).collect();
    canonical.name_tokens = ["check"].iter().map(|t| t.to_string()).collect();
    let mut target = make_test_symbol(2, "mod.check_d4", 2, 0, &[]);
    target.tokens = signature_tokens.iter().map(|t| t.to_string()).collect();
    target.name_tokens = ["check"].iter().map(|t| t.to_string()).collect();

    let mut symbols = HashMap::new();
    symbols.insert(1, canonical);
    symbols.insert(2, target);
    let corpus = Corpus {
        symbols: &symbols,
        index: None,
        df_all_excl: None,
        deep_callee: false,
        guarded_out: Cell::new(0),
    };
    assert!(
        find_best_d2(&symbols[&2], &corpus).is_none(),
        "an identical signature alone must not make two functions twins"
    );
}

#[test]
fn d2_accepts_twins_that_share_name_tokens() {
    let shared_tokens = ["render", "memory", "list", "section", "value"];
    let mut canonical = make_test_symbol(1, "ui.render_memory_list", 1, 5, &[]);
    canonical.tokens = shared_tokens.iter().map(|t| t.to_string()).collect();
    canonical.name_tokens = ["render", "memory", "list"]
        .iter()
        .map(|t| t.to_string())
        .collect();
    canonical.type_tokens = ["section", "value"].iter().map(|t| t.to_string()).collect();
    let mut target = make_test_symbol(2, "ui.render_memory_hit", 2, 0, &[]);
    target.tokens = shared_tokens.iter().map(|t| t.to_string()).collect();
    target.name_tokens = ["render", "memory", "hit"]
        .iter()
        .map(|t| t.to_string())
        .collect();
    target.type_tokens = ["section", "value"].iter().map(|t| t.to_string()).collect();

    let mut symbols = HashMap::new();
    symbols.insert(1, canonical);
    symbols.insert(2, target);
    let corpus = Corpus {
        symbols: &symbols,
        index: None,
        df_all_excl: None,
        deep_callee: false,
        guarded_out: Cell::new(0),
    };
    let candidate = find_best_d2(&symbols[&2], &corpus).expect("shared name tokens match");
    assert_eq!(candidate.canonical, "ui.render_memory_list");
}

#[test]
fn d2_rejects_numbered_siblings() {
    let shared_tokens = ["finder", "best", "candidate", "corpus"];
    let mut canonical = make_test_symbol(1, "mod.find_best_d1", 1, 5, &[]);
    canonical.tokens = shared_tokens.iter().map(|t| t.to_string()).collect();
    canonical.name_tokens = ["find", "best", "d1"]
        .iter()
        .map(|t| t.to_string())
        .collect();
    canonical.type_tokens = ["candidate", "corpus"]
        .iter()
        .map(|t| t.to_string())
        .collect();
    let mut target = make_test_symbol(2, "mod.find_best_d2", 1, 0, &[]);
    target.tokens = shared_tokens.iter().map(|t| t.to_string()).collect();
    target.name_tokens = ["find", "best", "d2"]
        .iter()
        .map(|t| t.to_string())
        .collect();
    target.type_tokens = ["candidate", "corpus"]
        .iter()
        .map(|t| t.to_string())
        .collect();

    let mut symbols = HashMap::new();
    symbols.insert(1, canonical);
    symbols.insert(2, target);
    let corpus = Corpus {
        symbols: &symbols,
        index: None,
        df_all_excl: None,
        deep_callee: false,
        guarded_out: Cell::new(0),
    };
    assert!(
        find_best_d2(&symbols[&2], &corpus).is_none(),
        "numbered siblings in one file are a family, not twins"
    );
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
        df_all_excl: None,
        deep_callee: false,
        guarded_out: Cell::new(0),
    };

    let candidate = check_d3(&target_rust, &corpus);
    assert!(
        candidate.is_some(),
        "must match reqwest.get with reqwest::get wrapper"
    );
    let cand = candidate.unwrap();
    assert_eq!(cand.canonical, "client.fetch");
    assert_eq!(cand.raw_client.as_deref(), Some("reqwest.get"));
    assert!(cand.shared_items.is_empty());

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
        df_all_excl: None,
        deep_callee: false,
        guarded_out: Cell::new(0),
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

fn probe_id(name: &str) -> i64 {
    match name {
        "a" => 1001,
        "b" => 1002,
        "c" => 1003,
        "x" => 1004,
        "y" => 1005,
        "z" => 1006,
        "mid" => 1007,
        other => 2000 + other.len() as i64,
    }
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
        sym.callee_ids.insert(probe_id(name));
        sym.deep_weights.insert(probe_id(name), 1.0);
    }
    for name in second_hop {
        sym.deep_weights.entry(probe_id(name)).or_insert(0.5);
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
            df_all_excl: None,
            deep_callee: true,
            guarded_out: Cell::new(0),
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
    let name_ids: HashMap<String, i64> = symbols
        .values()
        .map(|sym| (sym.name.clone(), sym.id))
        .collect();
    for symbol in symbols.values_mut() {
        for name in &symbol.resolved_callees {
            if let Some(id) = name_ids.get(name.as_str()) {
                symbol.callee_ids.insert(*id);
            }
        }
    }
    let index = CorpusIndex::build(&symbols);
    let corpus = Corpus {
        symbols: &symbols,
        index: Some(&index),
        df_all_excl: None,
        deep_callee: false,
        guarded_out: Cell::new(0),
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
    // D1 intersects project symbol IDs, so every named callee needs a real
    // corpus entry and the callers need matching callee_ids.
    let mut callee_ids = HashMap::new();
    for (offset, name) in [
        "helper_one",
        "helper_two",
        "helper_three",
        "different",
        "only",
    ]
    .into_iter()
    .enumerate()
    {
        let id = 100 + offset as i64;
        symbols.insert(
            id,
            make_test_symbol(id, &format!("x.{name}"), 90 + offset as i64, 0, &[]),
        );
        callee_ids.insert(name, id);
    }
    for symbol in symbols.values_mut() {
        for name in &symbol.resolved_callees {
            if let Some(id) = callee_ids.get(name.as_str()) {
                symbol.callee_ids.insert(*id);
            }
        }
    }
    let index = CorpusIndex::build(&symbols);
    let flat = Corpus {
        symbols: &symbols,
        index: None,
        df_all_excl: None,
        deep_callee: false,
        guarded_out: Cell::new(0),
    };
    let indexed = Corpus {
        symbols: &symbols,
        index: Some(&index),
        df_all_excl: None,
        deep_callee: false,
        guarded_out: Cell::new(0),
    };
    for target in symbols.values() {
        let flat_d1 = find_best_d1(target, &flat).map(|c| c.format_text(false));
        let indexed_d1 = find_best_d1(target, &indexed).map(|c| c.format_text(false));
        assert_eq!(flat_d1, indexed_d1, "D1 parity for {}", target.qualname);
        let flat_d2 = find_best_d2(target, &flat).map(|c| c.format_text(false));
        let indexed_d2 = find_best_d2(target, &indexed).map(|c| c.format_text(false));
        assert_eq!(flat_d2, indexed_d2, "D2 parity for {}", target.qualname);
    }
}

#[test]
fn d4_emits_no_measured_overlap_and_keeps_its_rank() {
    // D4: a 3-line single-caller pass-through wrapper around a canonical symbol.
    let mut canonical = make_test_symbol(1, "mod.canonical", 1, 5, &[]);
    canonical.line_start = 1;
    canonical.line_end = 3;
    let mut wrapper = make_test_symbol(2, "mod.wrapper", 2, 1, &["canonical"]);
    wrapper.line_start = 1;
    wrapper.line_end = 3;
    wrapper.callee_ids.insert(1);

    let mut symbols = HashMap::new();
    symbols.insert(1, canonical);
    symbols.insert(2, wrapper);
    let corpus = Corpus {
        symbols: &symbols,
        index: None,
        df_all_excl: None,
        deep_callee: false,
        guarded_out: Cell::new(0),
    };
    let cand = check_d4(&symbols[&2], &corpus).expect("D4 wrapper");
    assert!(
        cand.overlap_pct.is_none(),
        "D4 carries an evidence label, not a fabricated percentage"
    );
    // The historical rank is preserved so the default --limit still surfaces
    // wrappers; only the displayed metric was removed.
    assert_eq!(cand.sort_rank(), 100);
}

#[test]
fn d3_emits_no_measured_overlap() {
    let mut canonical = make_test_symbol(1, "mod.fetch_json", 1, 5, &["reqwest.Client"]);
    canonical.line_start = 1;
    canonical.line_end = 3;
    let mut target = make_test_symbol(2, "mod.get_data", 2, 1, &["reqwest.Client"]);
    target.line_start = 5;
    target.line_end = 8;
    target.callee_ids.insert(1);
    target.name_tokens = ["get", "data"].iter().map(|t| t.to_string()).collect();

    let mut symbols = HashMap::new();
    symbols.insert(1, canonical);
    symbols.insert(2, target);
    let corpus = Corpus {
        symbols: &symbols,
        index: None,
        df_all_excl: None,
        deep_callee: false,
        guarded_out: Cell::new(0),
    };
    if let Some(cand) = check_d3(&symbols[&2], &corpus) {
        assert!(cand.overlap_pct.is_none(), "D3 carries no percentage");
        assert_eq!(cand.sort_rank(), 90);
    }
}
