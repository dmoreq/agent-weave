//! Tokenization for memory FTS text, query terms, and ref/tag overlap
//! sets (the pure text side of the memory pipeline).

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

/// Split one alphanumeric run at camelCase boundaries: `calleeEmptyReason`
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

/// The design's single tokenizer for the memory store: split on
/// non-alphanumeric runs, split camelCase boundaries, lowercase. Query terms,
/// stored FTS text, tags, and ref-overlap token sets all flow through this
/// one function. Deliberately separate from the symbol-search tokenizer
/// (`extract::split_ident`): the two index different stores with different
/// digit/camelCase boundaries ("v2Fix" -> ["v","2","fix"] here vs "v2 fix"
/// there), so a camelCase fix must not silently change the other index.
pub fn mem_tokens(text: &str) -> Vec<String> {
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

/// Tokenize one ref (extension stripped first) — the ref-overlap signal.
pub(crate) fn mem_ref_tokens(ref_text: &str) -> Vec<String> {
    mem_tokens(strip_ref_extension(ref_text))
}

/// FTS text for one note row: tokens of key + tags + body, space-joined.
/// refs are excluded (the ref signal is a separate ranking term, no double
/// count). Shared by the insert path (db.rs) and the purity test so the
/// indexed bytes cannot drift.
pub fn mem_fts_text(key: &str, tags: &str, body: &str) -> String {
    let mut tokens = mem_tokens(key);
    tokens.extend(mem_tokens(tags));
    tokens.extend(mem_tokens(body));
    tokens.join(" ")
}

/// FTS MATCH query for the resolver: tokenized terms, space-joined. Every
/// token is `[a-z0-9]+` after `mem_tokens`, so no FTS syntax quoting is
/// needed. Empty input yields an empty string (no candidates).
pub fn mem_query_terms(terms: &str) -> String {
    mem_tokens(terms).join(" ")
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

// ---------------------------------------------------------------------------
// Write validation + normalization (pure; both surfaces share it).
// ---------------------------------------------------------------------------
