//! Project memory: pure presentation and scoring over rows `db.rs` returns.
//! No write primitives, no connection opening — everything here is a pure
//! function of its inputs, so CLI and MCP render byte-identical output by
//! construction (companion design: docs/designs/2026-11-05_agent-weave-
//! memory-retrieval.md; resolver constants follow its Decision D4-A).

use crate::error::{AwError, Result};

// ---------------------------------------------------------------------------
// Tuning constants (one module, compile-in only — no config surface).
// ---------------------------------------------------------------------------

/// Honesty caveat, stdout first line of every mem command except `mem export`
/// (whose stdout must stay pure JSONL; the caveat goes to stderr there).
pub const MEMORY_CAVEAT: &str =
    "aw: memory — agent-authored notes, candidates only, never proof of current behavior";

/// Hard cap on resolver candidate rows pulled from the FTS match list.
pub const CANDIDATE_CAP: usize = 256;
/// Fusion weights (D4-A): lexical, ref/token-overlap, recency. Fixed until the
/// Phase 3 frozen oracle produces evidence for a retune.
pub const W_LEX: f64 = 0.5;
pub const W_REF: f64 = 0.3;
pub const W_REC: f64 = 0.2;
/// `mem query` result cap: default 10, clamp 1..=100.
pub const DEFAULT_MEM_LIMIT: usize = 10;
pub const MAX_MEM_LIMIT: usize = 100;

pub const MAX_KEY_BYTES: usize = 200;
pub const MAX_BODY_BYTES: usize = 8192;
pub const MAX_TAGS: usize = 16;
pub const MAX_TAG_BYTES: usize = 32;
pub const MAX_REFS: usize = 16;
pub const MAX_REF_BYTES: usize = 256;
/// Closed `kind` taxonomy (OQ3 keeps the design default until the owner says
/// otherwise); anything else is a usage error (exit 2).
pub const MEM_KINDS: [&str; 4] = ["decision", "gotcha", "fact", "task"];
pub const SCOPE_DURABLE: &str = "durable";
pub const SCOPE_SESSION: &str = "session";
/// OQ2 (owner, 2026-11-05): a session note without an explicit `--ttl` lives
/// for 7 days. Explicit `--ttl` always wins.
pub const DEFAULT_SESSION_TTL_SECS: i64 = 604_800;
/// Upper bound for an explicit `--ttl` (10 years). Keeps the `u64 -> i64`
/// conversion and `now + ttl` free of overflow and keeps a session note inside
/// a range the storage clock can honor.
pub const MAX_SESSION_TTL_SECS: u64 = 10 * 365 * 86_400;
/// gc retention clock for superseded revisions (Phase 2): revs superseded
/// more than this many days ago are purged, clocked by `superseded_at`.
pub const RETENTION_DAYS: i64 = 30;

// ---------------------------------------------------------------------------
// One tokenizer, two consumers (Storage design note): the FTS text stored at
// insert time and the query side (MATCH terms + ref-overlap) both pass
// through `mem_tokens`, so query and stored text can never disagree.
// ---------------------------------------------------------------------------

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
pub fn mem_ref_tokens(ref_text: &str) -> Vec<String> {
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

/// Normalized note input after validation: tags lowercased, whitespace-collapsed;
/// refs whitespace-collapsed (case preserved — qualnames are case-sensitive).
/// `ttl_secs` is the relative lifetime: 0 for durable notes (never expires),
/// the explicit `--ttl` or the 7-day OQ2 default for session notes. The write
/// path stamps the absolute `expires_at = now + ttl_secs` (pure module: no clock).
#[derive(Debug)]
pub struct NormalizedNote {
    pub key: String,
    pub kind: String,
    pub tags: String,
    pub refs: String,
    pub scope: String,
    pub body: String,
    pub ttl_secs: i64,
}

/// Raw `mem write` input before normalization: one bundle both surfaces
/// build (CLI flags and MCP arguments), so a new field is one struct change
/// instead of a longer parameter list. Field semantics match the flags:
/// `tags`/`refs` are optional raw sets, `ttl` is the explicit `--ttl`.
pub struct NoteInput<'a> {
    pub key: &'a str,
    pub kind: &'a str,
    pub tags: Option<&'a str>,
    pub refs: Option<&'a str>,
    pub scope: &'a str,
    pub body: &'a str,
    pub ttl: Option<u64>,
}

/// Validate and normalize a `mem write` input. Scope rules (Phase 2):
/// `durable` never expires and rejects `--ttl`; `session` expires at
/// `now + ttl`, defaulting to the 7-day OQ2 constant without one.
pub fn normalize_note(input: NoteInput<'_>) -> Result<NormalizedNote> {
    let NoteInput {
        key,
        kind,
        tags,
        refs,
        scope,
        body,
        ttl,
    } = input;
    let key = validate_key(key)?;
    if !MEM_KINDS.contains(&kind) {
        return Err(AwError::Usage(format!(
            "mem write: kind must be one of {} (got '{kind}')",
            MEM_KINDS.join("|")
        )));
    }
    validate_body(body)?;
    Ok(NormalizedNote {
        key: key.to_string(),
        kind: kind.to_string(),
        tags: normalize_tags(tags.unwrap_or(""))?,
        refs: normalize_refs(refs.unwrap_or(""))?,
        scope: scope.to_string(),
        body: body.to_string(),
        ttl_secs: normalize_ttl(scope, ttl)?,
    })
}

fn validate_key(key: &str) -> Result<&str> {
    let key = key.trim();
    if key.is_empty() {
        return Err(AwError::Usage("mem write: key must not be empty".into()));
    }
    if key.len() > MAX_KEY_BYTES {
        return Err(AwError::Usage(format!(
            "mem write: key exceeds {MAX_KEY_BYTES} bytes"
        )));
    }
    if key.chars().any(|c| c.is_control()) {
        return Err(AwError::Usage(
            "mem write: key must not contain control characters".into(),
        ));
    }
    Ok(key)
}

fn validate_body(body: &str) -> Result<()> {
    let body_len = body.len();
    if body.trim().is_empty() {
        return Err(AwError::Usage("mem write: body must not be empty".into()));
    }
    if body_len > MAX_BODY_BYTES {
        return Err(AwError::Usage(format!(
            "mem write: body exceeds {MAX_BODY_BYTES} bytes (got {body_len})"
        )));
    }
    Ok(())
}

fn normalize_token_set(raw: &str, max: usize, max_len: usize, label: &str) -> Result<String> {
    let tokens: Vec<String> = raw.split_whitespace().map(str::to_string).collect();
    if tokens.len() > max {
        return Err(AwError::Usage(format!(
            "mem write: more than {max} {label}"
        )));
    }
    if let Some(t) = tokens.iter().find(|t| t.len() > max_len) {
        return Err(AwError::Usage(format!(
            "mem write: {label} token exceeds {max_len} bytes: '{t}'"
        )));
    }
    Ok(tokens.join(" "))
}

/// Canonical set form: tags are lowercased and sorted so the stored bytes —
/// and therefore the content hash — are order-insensitive.
fn normalize_tags(raw: &str) -> Result<String> {
    let tokens = normalize_token_set(raw, MAX_TAGS, MAX_TAG_BYTES, "tags")?;
    let mut tags: Vec<String> = tokens
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    tags.sort();
    Ok(tags.join(" "))
}

/// Refs use the same canonical set rule (a set of qualnames/paths).
fn normalize_refs(raw: &str) -> Result<String> {
    let tokens = normalize_token_set(raw, MAX_REFS, MAX_REF_BYTES, "refs")?;
    let mut refs: Vec<String> = tokens.split_whitespace().map(str::to_string).collect();
    refs.sort();
    Ok(refs.join(" "))
}

fn normalize_ttl(scope: &str, ttl: Option<u64>) -> Result<i64> {
    match scope {
        SCOPE_DURABLE => {
            if ttl.is_some() {
                return Err(AwError::Usage(
                    "mem write: --ttl is only valid with --scope session".to_string(),
                ));
            }
            Ok(0)
        }
        SCOPE_SESSION => match ttl {
            None => Ok(DEFAULT_SESSION_TTL_SECS),
            Some(t) if t == 0 || t > MAX_SESSION_TTL_SECS => Err(AwError::Usage(format!(
                "mem write: --ttl must be in 1..={MAX_SESSION_TTL_SECS} seconds"
            ))),
            Some(t) => Ok(t as i64),
        },
        other => Err(AwError::Usage(format!(
            "mem write: scope must be '{SCOPE_DURABLE}' or '{SCOPE_SESSION}' (got '{other}')"
        ))),
    }
}

/// Content hash over the normalized (kind, body, tags, refs, scope) tuple with
/// a unit separator so field boundaries cannot collide. Dedup works on the
/// normalized form: two writes differing only in tag order or whitespace are
/// the same content.
pub fn mem_content_hash(kind: &str, body: &str, tags: &str, refs: &str, scope: &str) -> u64 {
    crate::hash::fnv1a64(
        format!("{kind}\u{1f}{body}\u{1f}{tags}\u{1f}{refs}\u{1f}{scope}").as_bytes(),
    )
}

// ---------------------------------------------------------------------------
// Resolver fusion (D4-A): bounded signals, min-max normalization, fixed
// weights, deterministic tie-breaks.
// ---------------------------------------------------------------------------

/// One candidate row with its raw signals. `lex_raw` is the FTS5 bm25 rank
/// (smaller is better); `ref_raw` is the token-overlap fraction in [0,1];
/// `age_days` feeds the bounded recency decay.
pub struct Candidate {
    pub row: crate::db::MemRow,
    pub lex_raw: f64,
    pub ref_raw: f64,
    pub age_days: f64,
    /// Fused score, filled by [`fuse`].
    pub score: f64,
}

/// Min-max normalize with the design's all-equal convention: an all-equal
/// signal maps every row to 1.0 (zeromem convention; order-invariant).
fn minmax(values: &[f64], invert: bool) -> Vec<f64> {
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if !min.is_finite() || !max.is_finite() || (max - min).abs() < f64::EPSILON {
        return vec![1.0; values.len()];
    }
    values
        .iter()
        .map(|v| {
            let n = (v - min) / (max - min);
            if invert {
                1.0 - n
            } else {
                n
            }
        })
        .collect()
}

/// Fuse the candidates: invert bm25 (smaller is better), min-max both
/// bounded signals, apply the recency decay, weight, then sort by
/// (score desc, created_at desc, key asc, rev desc). Deterministic: same
/// rows + same query + same binary -> same order.
pub fn fuse(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    let lex = minmax(
        &candidates.iter().map(|c| c.lex_raw).collect::<Vec<_>>(),
        true,
    );
    let reference = minmax(
        &candidates.iter().map(|c| c.ref_raw).collect::<Vec<_>>(),
        false,
    );
    for (i, c) in candidates.iter_mut().enumerate() {
        let rec = 1.0 / (1.0 + c.age_days);
        c.score = W_LEX * lex[i] + W_REF * reference[i] + W_REC * rec;
    }
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.row.created_at.cmp(&a.row.created_at))
            .then(a.row.key.cmp(&b.row.key))
            .then(b.row.rev.cmp(&a.row.rev))
    });
    candidates
}

// ---------------------------------------------------------------------------
// Rendering (shared bytes for CLI and MCP).
// ---------------------------------------------------------------------------

/// One note row header line: `key rev kind scope created_at`.
pub fn render_row_header(row: &crate::db::MemRow) -> String {
    format!(
        "{} r{} {} {} {}",
        row.key, row.rev, row.kind, row.scope, row.created_at
    )
}

/// Render a row with a trailing header tag (the `--include-superseded`
/// listing marks replaced revisions with ` [superseded]`).
pub fn render_row_tagged(row: &crate::db::MemRow, tag: &str) -> String {
    let mut out = render_row_header(row);
    out.push_str(tag);
    for line in row.body.lines() {
        out.push_str(&format!("\n  {line}"));
    }
    out
}

/// `showing N of M` cap line (stdout, last line of a capped query).
pub fn render_showing(shown: usize, total: usize) -> String {
    crate::query::cap_line(shown, total, "").unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default `mem write` input (tags/refs/ttl unset); tests override the
    /// differing fields with struct-update syntax.
    fn note<'a>(key: &'a str, kind: &'a str, body: &'a str, scope: &'a str) -> NoteInput<'a> {
        NoteInput {
            key,
            kind,
            tags: None,
            refs: None,
            scope,
            body,
            ttl: None,
        }
    }

    #[test]
    fn tokenizer_splits_camel_snake_and_nonalnum() {
        assert_eq!(
            mem_tokens("calleeEmptyReason"),
            vec!["callee", "empty", "reason"]
        );
        assert_eq!(
            mem_tokens("callee_empty_reason"),
            vec!["callee", "empty", "reason"]
        );
        assert_eq!(mem_tokens("HTTPServer"), vec!["http", "server"]);
        assert_eq!(mem_tokens("v2Fix"), vec!["v", "2", "fix"]);
        assert_eq!(mem_tokens("src/foo.rs"), vec!["src", "foo", "rs"]);
        assert_eq!(mem_tokens(""), Vec::<String>::new());
    }

    #[test]
    fn ref_extension_stripped_only_in_last_segment() {
        assert_eq!(mem_ref_tokens("src/foo.rs"), vec!["src", "foo"]);
        assert_eq!(mem_ref_tokens("src.foo/bar.rs"), vec!["src", "foo", "bar"]);
        assert_eq!(mem_ref_tokens("a::b::c"), vec!["a", "b", "c"]);
        // A long trailing chunk is not an extension — kept whole.
        assert_eq!(
            mem_ref_tokens("module.nameverylongtoken"),
            vec!["module", "nameverylongtoken"]
        );
    }

    #[test]
    fn normalize_rejects_bad_kind_and_unknown_scope() {
        let err = normalize_note(note("k", "mood", "b", "durable")).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        let err = normalize_note(note("k", "fact", "b", "orbit")).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(normalize_note(note("k", "fact", "b", "durable")).is_ok());
        assert!(normalize_note(note("k", "fact", "b", "session")).is_ok());
    }

    #[test]
    fn session_ttl_defaults_to_seven_days_and_durable_rejects_ttl() {
        // OQ2: session without --ttl expires in 7 days; explicit ttl wins.
        let normalized = normalize_note(note("k", "fact", "b", "session")).unwrap();
        assert_eq!(normalized.ttl_secs, DEFAULT_SESSION_TTL_SECS);
        let normalized = normalize_note(NoteInput {
            ttl: Some(60),
            ..note("k", "fact", "b", "session")
        })
        .unwrap();
        assert_eq!(normalized.ttl_secs, 60);
        // Out-of-range values cannot wrap the u64 -> i64 cast or `now + ttl`.
        for bad in [0, u64::MAX, MAX_SESSION_TTL_SECS + 1] {
            let err = normalize_note(NoteInput {
                ttl: Some(bad),
                ..note("k", "fact", "b", "session")
            })
            .unwrap_err();
            assert_eq!(err.exit_code(), 2, "ttl {bad}");
        }
        let max = normalize_note(NoteInput {
            ttl: Some(MAX_SESSION_TTL_SECS),
            ..note("k", "fact", "b", "session")
        })
        .unwrap();
        assert_eq!(max.ttl_secs, MAX_SESSION_TTL_SECS as i64);
        // Durable never expires and rejects --ttl (exit 2).
        let normalized = normalize_note(note("k", "fact", "b", "durable")).unwrap();
        assert_eq!(normalized.ttl_secs, 0);
        let err = normalize_note(NoteInput {
            ttl: Some(60),
            ..note("k", "fact", "b", "durable")
        })
        .unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("only valid with --scope session"));
    }

    #[test]
    fn normalize_caps_and_lowercases_tags() {
        let normalized = normalize_note(NoteInput {
            tags: Some("Auth Login AUTH"),
            ..note("k", "fact", "b", "durable")
        })
        .unwrap();
        assert_eq!(
            normalized.tags, "auth auth login",
            "tags are a sorted lowercase set"
        );
        let err = normalize_note(NoteInput {
            tags: Some(&vec!["t"; MAX_TAGS + 1].join(" ")),
            ..note("k", "fact", "b", "durable")
        })
        .unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn body_cap_is_enforced() {
        let big = "x".repeat(MAX_BODY_BYTES + 1);
        let err = normalize_note(NoteInput {
            body: &big,
            ..note("k", "fact", "b", "durable")
        })
        .unwrap_err();
        assert_eq!(err.exit_code(), 2);
        let ok = "x".repeat(MAX_BODY_BYTES);
        assert!(normalize_note(NoteInput {
            body: &ok,
            ..note("k", "fact", "b", "durable")
        })
        .is_ok());
    }

    #[test]
    fn fusion_is_deterministic_and_all_equal_maps_to_one() {
        let mk = |id: i64, lex: f64, r#ref: f64, age: f64, created: i64| Candidate {
            row: crate::db::MemRow {
                id,
                key: format!("k{id}"),
                rev: 1,
                kind: "fact".into(),
                scope: "durable".into(),
                session: String::new(),
                body: "b".into(),
                tags: String::new(),
                refs: String::new(),
                source: "cli".into(),
                created_at: created,
                expires_at: 0,
                superseded_by: None,
            },
            lex_raw: lex,
            ref_raw: r#ref,
            age_days: age,
            score: 0.0,
        };
        // All-equal lex and ref -> both 1.0; recency alone separates.
        let fused = fuse(vec![mk(1, 2.0, 0.5, 0.0, 100), mk(2, 2.0, 0.5, 10.0, 100)]);
        assert!(fused[0].score > fused[1].score);
        // Inverted bm25: smaller raw rank wins.
        let fused = fuse(vec![mk(1, 1.0, 0.0, 0.0, 100), mk(2, 5.0, 0.0, 0.0, 100)]);
        assert!(fused[0].row.id == 1, "smaller bm25 must rank first");
        // Deterministic order for equal scores: created_at desc, key asc.
        let fused = fuse(vec![mk(2, 1.0, 0.0, 0.0, 100), mk(1, 1.0, 0.0, 0.0, 100)]);
        assert_eq!(fused[0].row.key, "k1");
    }

    #[test]
    fn content_hash_ignores_tag_order_and_whitespace() {
        // Dedup hashes the NORMALIZED note (writes always normalize first),
        // so tag order and whitespace runs are not part of the identity.
        let a = normalize_note(NoteInput {
            tags: Some("a b"),
            refs: Some("src/main.rs"),
            ..note("k", "fact", "body", "durable")
        })
        .unwrap();
        let b = normalize_note(NoteInput {
            tags: Some("b   a"),
            refs: Some("src/main.rs"),
            ..note("k", "fact", "body", "durable")
        })
        .unwrap();
        assert_eq!(
            mem_content_hash(&a.kind, &a.body, &a.tags, &a.refs, &a.scope),
            mem_content_hash(&b.kind, &b.body, &b.tags, &b.refs, &b.scope)
        );
        let c = normalize_note(NoteInput {
            tags: Some("a b"),
            refs: Some("src/main.rs"),
            ..note("k", "fact", "body ", "durable")
        })
        .unwrap();
        assert_ne!(
            mem_content_hash(&a.kind, &a.body, &a.tags, &a.refs, &a.scope),
            mem_content_hash(&c.kind, &c.body, &c.tags, &c.refs, &c.scope),
            "body bytes are part of the identity"
        );
    }
}
