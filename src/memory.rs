//! Project memory: pure presentation and scoring over rows `db.rs` returns.
//! No write primitives, no connection opening — everything here is a pure
//! function of its inputs, so CLI and MCP render byte-identical output by
//! construction (companion design: docs/architecture/memory-store.md;
//! resolver constants follow its Decision D4-A).

use crate::error::{EgError, Result};

// ---------------------------------------------------------------------------
// Tuning constants (one module, compile-in only — no config surface).
// ---------------------------------------------------------------------------

/// Honesty caveat, stdout first line of every mem command except `mem export`
/// (whose stdout must stay pure JSONL; the caveat goes to stderr there).
pub const MEMORY_CAVEAT: &str =
    "eg: memory — agent-authored notes, candidates only, never proof of current behavior";

/// Single source of the `mem write` secret-rejection usage text: key, body,
/// and tags/refs validation all emit this exact message so the bytes cannot
/// drift between the three validators (same discipline as CORRUPT_INDEX_MSG).
const SUSPECTED_SECRET_MSG: &str = "mem write: content contains suspected secret token";

/// Hard cap on resolver candidate rows pulled from the FTS match list.
pub const CANDIDATE_CAP: usize = 256;
/// `mem query` result cap: default 10, clamp 1..=100.
pub const DEFAULT_MEM_LIMIT: usize = 10;
pub const MAX_MEM_LIMIT: usize = 100;

pub const MAX_KEY_BYTES: usize = 200;
pub const MAX_BODY_BYTES: usize = 8192;
pub const MAX_TAGS: usize = 16;
pub const MAX_TAG_BYTES: usize = 32;
pub const MAX_REFS: usize = 16;
pub const MAX_REF_BYTES: usize = 256;
/// Import input cap: one Markdown/JSONL file larger than this is rejected
/// (exit 2) before parsing, so a hostile export cannot exhaust memory.
pub const MAX_IMPORT_FILE_BYTES: usize = 16 * 1024 * 1024;
/// Import note cap: a single import may carry at most this many notes.
pub const MAX_IMPORT_NOTES: usize = 5000;
/// Closed `kind` taxonomy (OQ3 keeps the design default until the owner says
/// otherwise); anything else is a usage error (exit 2). Stored rows spell the
/// kind through `as_str()` at the bind boundary and validation parses the raw
/// flag into a variant, so a misspelled kind is a compile error instead of a
/// silent wrong row (same discipline as `EdgeKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemKind {
    Decision,
    Gotcha,
    Fact,
    Task,
    Procedure,
}

/// The taxonomy as a list, kept under the `MEM_KINDS` name as the single
/// source the usage-error render and the parser both derive from.
pub const MEM_KINDS: [MemKind; 5] = [
    MemKind::Decision,
    MemKind::Gotcha,
    MemKind::Fact,
    MemKind::Task,
    MemKind::Procedure,
];

impl MemKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            MemKind::Decision => "decision",
            MemKind::Gotcha => "gotcha",
            MemKind::Fact => "fact",
            MemKind::Task => "task",
            MemKind::Procedure => "procedure",
        }
    }

    /// Parse one raw `--kind` flag value at the trust boundary.
    pub fn parse(raw: &str) -> Option<MemKind> {
        MEM_KINDS.iter().copied().find(|k| k.as_str() == raw)
    }
}

/// Closed scope taxonomy: `durable` never expires, `session` expires at
/// `now + ttl`. Parsed at the trust boundary like [`MemKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemScope {
    Durable,
    Session,
}

impl MemScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            MemScope::Durable => "durable",
            MemScope::Session => "session",
        }
    }

    /// Parse one raw `--scope` flag value at the trust boundary.
    pub fn parse(raw: &str) -> Option<MemScope> {
        [MemScope::Durable, MemScope::Session]
            .into_iter()
            .find(|s| s.as_str() == raw)
    }
}

/// Parse an optional `--kind` filter at the trust boundary: a misspelling is
/// a usage error naming the allowed set (the read-path counterpart of the
/// `mem write` validation text, prefixed with the issuing command).
pub fn parse_kind_filter(command: &str, raw: Option<&str>) -> Result<Option<MemKind>> {
    raw.map(|k| {
        MemKind::parse(k).ok_or_else(|| {
            EgError::Usage(format!(
                "{command}: kind must be one of {} (got '{k}')",
                MEM_KINDS.map(MemKind::as_str).join("|")
            ))
        })
    })
    .transpose()
}

/// Parse an optional `--scope` filter at the trust boundary; see
/// [`parse_kind_filter`] for the error contract.
pub fn parse_scope_filter(command: &str, raw: Option<&str>) -> Result<Option<MemScope>> {
    raw.map(|s| {
        MemScope::parse(s).ok_or_else(|| {
            EgError::Usage(format!(
                "{command}: scope must be one of durable|session (got '{s}')"
            ))
        })
    })
    .transpose()
}
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
// Two tokenizer contracts coexist during the dual-index migration: the frozen
// legacy ASCII contract for `memories_fts` and the Unicode contract for
// `memories_fts_v2`. The secret detector and time grammar are standalone.
// ---------------------------------------------------------------------------

pub mod secret;
pub mod time;
pub use time::{parse_time_bound, validate_window, TimeWindow};
mod graph;
mod guard;
mod note;
mod rank;
mod tokens;

pub(crate) use graph::*;
pub(crate) use guard::*;
pub(crate) use note::*;
pub(crate) use rank::*;
pub(crate) use tokens::*;
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
            graph_raw: 0.0,
            age_days: age,
            score: 0.0,
            is_global: false,
            from_fts: false,
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
            mem_content_hash(a.kind.as_str(), &a.body, &a.tags, &a.refs, a.scope.as_str()),
            mem_content_hash(b.kind.as_str(), &b.body, &b.tags, &b.refs, b.scope.as_str())
        );
        let c = normalize_note(NoteInput {
            tags: Some("a b"),
            refs: Some("src/main.rs"),
            ..note("k", "fact", "body ", "durable")
        })
        .unwrap();
        assert_ne!(
            mem_content_hash(a.kind.as_str(), &a.body, &a.tags, &a.refs, a.scope.as_str()),
            mem_content_hash(c.kind.as_str(), &c.body, &c.tags, &c.refs, c.scope.as_str()),
            "body bytes are part of the identity"
        );
    }

    #[test]
    fn secret_rejection_covers_key_body_tags_refs() {
        let openai = format!("sk-{}", "a".repeat(24));
        let github = format!("ghp_{}", "a".repeat(36));
        let aws = format!("AKIA{}", "A".repeat(16));
        let jwt = format!(
            "eyJ{}.eyJ{}.{}",
            "a".repeat(10),
            "b".repeat(10),
            "c".repeat(10)
        );
        let pem = format!("-----BEGIN {} PRIVATE KEY-----", "RSA");

        assert!(normalize_note(NoteInput {
            key: &openai,
            ..note("k", "fact", "b", "durable")
        })
        .is_err());
        assert!(normalize_note(NoteInput {
            body: &github,
            ..note("k", "fact", "b", "durable")
        })
        .is_err());
        assert!(normalize_note(NoteInput {
            tags: Some(&aws),
            ..note("k", "fact", "b", "durable")
        })
        .is_err());
        assert!(normalize_note(NoteInput {
            refs: Some(&jwt),
            ..note("k", "fact", "b", "durable")
        })
        .is_err());
        assert!(normalize_note(NoteInput {
            body: &pem,
            ..note("k", "fact", "b", "durable")
        })
        .is_err());

        // Short prefixes and ordinary words that contain `sk-` are allowed.
        for text in [
            "sk-12345",
            "ghp_abcdef",
            "eyJhbGciOi",
            "task-1 pending",
            "risk-averse",
        ] {
            assert!(
                normalize_note(NoteInput {
                    body: text,
                    ..note("k", "fact", "b", "durable")
                })
                .is_ok(),
                "{text}"
            );
        }
    }
}
