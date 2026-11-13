//! Tokenization for memory FTS text, query terms, and ref/tag overlap
//! sets (the pure text side of the memory pipeline).
//!
//! Two contracts coexist during the dual-index migration:
//! - `*_legacy` keeps the exact 1.0.4 ASCII behavior for `memories_fts`, so an
//!   older binary and a not-yet-migrated store keep working.
//! - the Unicode functions preserve Vietnamese words for `memories_fts_v2`.
//!
//! `mem_tokens` and the public text/query helpers use the Unicode contract;
//! the legacy table is written through the explicit `*_legacy` functions.

/// Strip one trailing file extension from a ref before tokenizing: ref
/// `src/foo.rs` yields tokens `src foo` — the bare token `rs` must not match
/// every query. The extension must be 1-8 alphanumeric characters and its
/// dot must sit in the last path segment; otherwise the ref is kept whole.
fn strip_ref_extension(ref_text: &str) -> &str {
    let Some((head, ext)) = ref_text.rsplit_once('.') else {
        return ref_text;
    };
    if head.is_empty() {
        return ref_text;
    }
    // The dot must come after the last path separator (last segment only).
    if let Some(slash) = ref_text.rfind(['/', '\\']) {
        if ref_text.rfind('.').unwrap_or(0) < slash {
            return ref_text;
        }
    }
    let valid = !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric());
    if valid {
        head
    } else {
        ref_text
    }
}

/// Split one ASCII alphanumeric run at camelCase boundaries: `calleeEmptyReason`
/// -> `callee empty reason`; `HTTPServer` -> `http server`; digits break
/// letter runs (`v2Fix` -> `v 2 fix`).
fn split_run(run: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let chars: Vec<char> = run.chars().collect();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let prev = if i == 0 { None } else { chars.get(i - 1) };
        let next = chars.get(i + 1);
        let boundary = match (prev, c, next) {
            // lower/digit -> upper boundary
            (Some(p), u, _)
                if u.is_ascii_uppercase() && (p.is_ascii_lowercase() || p.is_ascii_digit()) =>
            {
                true
            }
            // upper-run -> Upper+lower boundary: HTTP|Server splits before S
            (Some(p), u, Some(n))
                if u.is_ascii_uppercase() && p.is_ascii_uppercase() && n.is_ascii_lowercase() =>
            {
                true
            }
            // letter -> digit boundary
            (Some(p), d, _) if d.is_ascii_digit() && p.is_ascii_alphabetic() => true,
            _ => false,
        };
        if boundary && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur).to_ascii_lowercase());
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        parts.push(cur.to_ascii_lowercase());
    }
    parts
}

/// Unicode variant of [`split_run`]. Case boundaries use Unicode predicates;
/// each part is lowercased and stripped of any non-alphanumeric character
/// introduced by case expansion (for example `İ` -> `i` + combining dot).
fn split_run_unicode(run: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let chars: Vec<char> = run.chars().collect();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let prev = if i == 0 { None } else { chars.get(i - 1) };
        let next = chars.get(i + 1);
        let boundary = match (prev, c, next) {
            (Some(p), u, _) if u.is_uppercase() && (p.is_lowercase() || p.is_numeric()) => true,
            (Some(p), u, Some(n)) if u.is_uppercase() && p.is_uppercase() && n.is_lowercase() => {
                true
            }
            (Some(p), d, _) if d.is_numeric() && p.is_alphabetic() => true,
            _ => false,
        };
        if boundary && !cur.is_empty() {
            push_clean(&mut parts, &std::mem::take(&mut cur));
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        push_clean(&mut parts, &cur);
    }
    parts
}

fn push_clean(parts: &mut Vec<String>, raw: &str) {
    let cleaned: String = raw
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    if !cleaned.is_empty() {
        parts.push(cleaned);
    }
}

/// The 1.0.4 tokenizer: ASCII alphanumerics only, ASCII camelCase split,
/// ASCII lowercase. Frozen so the legacy index cannot drift.
pub fn mem_tokens_legacy(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut run = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            run.push(c);
        } else if !run.is_empty() {
            out.extend(split_run(&run));
            run.clear();
        }
    }
    if !run.is_empty() {
        out.extend(split_run(&run));
    }
    out
}

/// The Unicode tokenizer: keeps Vietnamese letters, maps `đ`/`Đ` to `d`
/// (SQLite's `remove_diacritics 2` does not fold the stroke), splits ASCII
/// camelCase and digit runs, and lowercases.
pub fn mem_tokens_unicode(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut run = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            run.push(match c {
                'đ' | 'Đ' => 'd',
                other => other,
            });
        } else if !run.is_empty() {
            out.extend(split_run_unicode(&run));
            run.clear();
        }
    }
    if !run.is_empty() {
        out.extend(split_run_unicode(&run));
    }
    out
}

/// Active tokenizer: the Unicode contract for `memories_fts_v2`. Legacy
/// callers use `mem_tokens_legacy` explicitly.
pub fn mem_tokens(text: &str) -> Vec<String> {
    mem_tokens_unicode(text)
}

/// Tokenize one ref (extension stripped first) — the ref-overlap signal.
pub(crate) fn mem_ref_tokens(ref_text: &str) -> Vec<String> {
    mem_tokens(strip_ref_extension(ref_text))
}

/// FTS text for one note under the legacy tokenizer.
pub fn mem_fts_text_legacy(key: &str, tags: &str, body: &str) -> String {
    let mut tokens = mem_tokens_legacy(key);
    tokens.extend(mem_tokens_legacy(tags));
    tokens.extend(mem_tokens_legacy(body));
    tokens.join(" ")
}

/// FTS text for one note under the Unicode tokenizer.
pub fn mem_fts_text_unicode(key: &str, tags: &str, body: &str) -> String {
    let mut tokens = mem_tokens_unicode(key);
    tokens.extend(mem_tokens_unicode(tags));
    tokens.extend(mem_tokens_unicode(body));
    tokens.join(" ")
}

/// Active FTS text for the Unicode index.
pub fn mem_fts_text(key: &str, tags: &str, body: &str) -> String {
    mem_fts_text_unicode(key, tags, body)
}

/// FTS MATCH query under the legacy tokenizer.
pub fn mem_query_terms_legacy(terms: &str) -> String {
    mem_tokens_legacy(terms).join(" ")
}

/// FTS MATCH query under the Unicode tokenizer.
pub fn mem_query_terms_unicode(terms: &str) -> String {
    mem_tokens_unicode(terms).join(" ")
}

/// Active query terms for the Unicode index.
pub fn mem_query_terms(terms: &str) -> String {
    mem_query_terms_unicode(terms)
}

/// Ref/tag token set under the legacy tokenizer.
pub fn mem_ref_tag_tokens_legacy(refs: &str, tags: &str) -> std::collections::BTreeSet<String> {
    let mut set = std::collections::BTreeSet::new();
    for r in refs.split_whitespace() {
        set.extend(mem_tokens_legacy(strip_ref_extension(r)));
    }
    for t in tags.split_whitespace() {
        set.extend(mem_tokens_legacy(t));
    }
    set
}

/// Ref/tag token set of a row for the ref-overlap signal: refs first
/// (extension-stripped), then tags. Deduplicated, order-insensitive.
pub fn mem_ref_tag_tokens(refs: &str, tags: &str) -> std::collections::BTreeSet<String> {
    let mut set = std::collections::BTreeSet::new();
    for r in refs.split_whitespace() {
        set.extend(mem_ref_tokens(r));
    }
    for t in tags.split_whitespace() {
        set.extend(mem_tokens(t));
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unicode_tokens_preserve_words() {
        assert_eq!(
            mem_tokens_unicode("Bộ nhớ dự án"),
            strings(&["bộ", "nhớ", "dự", "án"])
        );
        assert_eq!(mem_tokens_unicode("đường"), strings(&["dường"]));
    }

    #[test]
    fn legacy_vietnamese_skeleton() {
        assert_eq!(mem_tokens_legacy("Bộ nhớ"), strings(&["b", "nh"]));
    }

    #[test]
    fn legacy_ascii_equivalent() {
        for text in [
            "calleeEmptyReason",
            "HTTPServer v2Fix",
            "src/memory/rank.rs",
            "snake_case resolver tiers",
        ] {
            assert_eq!(mem_tokens_legacy(text), mem_tokens_unicode(text), "{text}");
        }
    }

    #[test]
    fn unicode_camel_and_digit_split() {
        assert_eq!(
            mem_tokens_unicode("calleeEmptyReason"),
            strings(&["callee", "empty", "reason"])
        );
        assert_eq!(
            mem_tokens_unicode("HTTPServer"),
            strings(&["http", "server"])
        );
        assert_eq!(mem_tokens_unicode("v2Fix"), strings(&["v", "2", "fix"]));
    }

    #[test]
    fn fts_metacharacters_are_dropped() {
        for text in ["quote\" NEAR(a b) *", "a AND b OR c NOT d", "ключ: bộ"] {
            for token in mem_tokens_unicode(text) {
                assert!(
                    token.chars().all(|c| c.is_alphanumeric()),
                    "token leaked a metacharacter: {token}"
                );
            }
        }
    }

    #[test]
    fn active_contract_is_unicode() {
        assert_eq!(mem_tokens("Bộ nhớ"), mem_tokens_unicode("Bộ nhớ"));
        assert_eq!(
            mem_fts_text("k", "t", "Bộ nhớ"),
            mem_fts_text_unicode("k", "t", "Bộ nhớ")
        );
    }
}

// ---------------------------------------------------------------------------
// Write validation + normalization (pure; both surfaces share it).
// ---------------------------------------------------------------------------
