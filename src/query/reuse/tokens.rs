//! Identifier tokenization and the stoplist that filters prelude/utility
//! callees and high-frequency shared names.

use super::corpus::SymbolData;
use super::*;
use std::collections::HashMap;

use crate::extract::split_ident;

/// Standard prelude/utility constructors and methods excluded from callee footprint.
const STOPLIST: &[&str] = &[
    // Rust prelude & stdlib
    "Ok",
    "Err",
    "Some",
    "None",
    "Vec::new",
    "String::new",
    "Default::default",
    "unwrap",
    "clone",
    "to_string",
    "format",
    "push",
    "len",
    "is_empty",
    "as_str",
    "as_ref",
    "into",
    "from",
    "map",
    "unwrap_or",
    "unwrap_or_default",
    "and_then",
    "contains",
    "get",
    "insert",
    "open",
    "new",
    "collect",
    "iter",
    "into_iter",
    "eprintln",
    "println",
    // TypeScript / JavaScript
    "Promise.all",
    "Promise.resolve",
    "console",
    "log",
    "error",
    "warn",
    "string",
    "number",
    "boolean",
    "any",
    "void",
    "Promise",
    "Array",
    "Object",
    // Python
    "def",
    "self",
    "cls",
    "int",
    "str",
    "float",
    "bool",
    "list",
    "dict",
    "set",
    "tuple",
    "print",
    "range",
    "isinstance",
];

pub(super) fn tokenize_name_sig(name: &str, sig: &str) -> (HashSet<String>, HashSet<String>) {
    let mut all_tokens = HashSet::new();
    let mut type_tokens = HashSet::new();
    // One normalized insert shared by every scan: lowercase, keep the
    // three-plus-char non-stoplist tokens, and also file them into the
    // type-only set when they come from a typed position.
    let mut push_token = |tok: &str, typed: bool| {
        let lower = tok.to_ascii_lowercase();
        if keep_token(&lower) {
            if typed {
                type_tokens.insert(lower.clone());
            }
            all_tokens.insert(lower);
        }
    };

    for tok in split_ident(name).split_whitespace() {
        push_token(tok, false);
    }

    for part in sig.split(['(', ')', ',', '{', ';']) {
        // Parameter (`:`) and return (`->`) annotations share one loop.
        for (_, typed) in [part.split_once(':'), part.split_once("->")]
            .into_iter()
            .flatten()
        {
            for tok in split_ident(typed).split_whitespace() {
                push_token(tok, true);
            }
        }
    }

    for tok in split_ident(sig).split_whitespace() {
        push_token(tok, false);
    }

    (all_tokens, type_tokens)
}

/// Shared tokenizer guard: keep lowercase tokens of at least three characters
/// that the stoplist does not cover.
fn keep_token(lower: &str) -> bool {
    lower.len() >= 3 && !STOPLIST.contains(&lower)
}

pub(super) fn build_stoplist(symbols: &HashMap<i64, SymbolData>) -> HashSet<String> {
    let mut stopset: HashSet<String> = STOPLIST.iter().map(|&s| s.to_string()).collect();
    let mut freq: HashMap<String, usize> = HashMap::new();
    let total = symbols.len();

    for sym in symbols.values() {
        for callee in &sym.all_callees {
            *freq.entry(callee.clone()).or_default() += 1;
        }
    }

    let threshold = (total as f64 * 0.05).max(4.0) as usize;
    for (callee, count) in freq {
        if count >= threshold && !is_raw_client(&callee) {
            stopset.insert(callee);
        }
    }
    stopset
}
