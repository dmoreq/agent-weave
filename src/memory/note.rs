//! Note normalization and validation: key/body/tags/refs/ttl bounds,
//! secret screening, and the content hash.

use super::*;

/// Normalized note input after validation: tags lowercased, whitespace-collapsed;
/// refs whitespace-collapsed (case preserved — qualnames are case-sensitive).
/// `kind` and `scope` are the parsed taxonomy variants; storage renders their
/// spelling through `as_str()` at the bind boundary. `ttl_secs` is the relative
/// lifetime: 0 for durable notes (never expires), the explicit `--ttl` or the
/// 7-day OQ2 default for session notes. The write path stamps the absolute
/// `expires_at = now + ttl_secs` (pure module: no clock).
#[derive(Debug)]
pub struct NormalizedNote {
    pub key: String,
    pub kind: MemKind,
    pub tags: String,
    pub refs: String,
    pub scope: MemScope,
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
    let kind = MemKind::parse(kind).ok_or_else(|| {
        EgError::Usage(format!(
            "mem write: kind must be one of {} (got '{kind}')",
            MEM_KINDS.map(MemKind::as_str).join("|")
        ))
    })?;
    validate_body(body)?;
    if contains_suspected_secret(tags.unwrap_or(""))
        || contains_suspected_secret(refs.unwrap_or(""))
    {
        return Err(EgError::Usage(SUSPECTED_SECRET_MSG.into()));
    }
    // Keep the original validation order: tags, refs, then scope/ttl last, so
    // a multi-field error reports the same field as before the enum split.
    let tags = normalize_tags(tags.unwrap_or(""))?;
    let refs = normalize_refs(refs.unwrap_or(""))?;
    let (scope, ttl_secs) = normalize_ttl(scope, ttl)?;
    Ok(NormalizedNote {
        key: key.to_string(),
        kind,
        tags,
        refs,
        scope,
        body: body.to_string(),
        ttl_secs,
    })
}

fn contains_suspected_secret(text: &str) -> bool {
    const SUSPECTED_SECRETS: [&str; 5] = ["sk-", "ghp_", "AKIA", "eyJ", "-----BEGIN "];
    SUSPECTED_SECRETS
        .iter()
        .any(|&prefix| text.contains(prefix))
}

fn validate_key(key: &str) -> Result<&str> {
    let key = key.trim();
    if key.is_empty() {
        return Err(EgError::Usage("mem write: key must not be empty".into()));
    }
    if key.len() > MAX_KEY_BYTES {
        return Err(EgError::Usage(format!(
            "mem write: key exceeds {MAX_KEY_BYTES} bytes"
        )));
    }
    if key.chars().any(|c| c.is_control()) {
        return Err(EgError::Usage(
            "mem write: key must not contain control characters".into(),
        ));
    }
    if contains_suspected_secret(key) {
        return Err(EgError::Usage(SUSPECTED_SECRET_MSG.into()));
    }
    Ok(key)
}

fn validate_body(body: &str) -> Result<()> {
    let body_len = body.len();
    if body.trim().is_empty() {
        return Err(EgError::Usage("mem write: body must not be empty".into()));
    }
    if body_len > MAX_BODY_BYTES {
        return Err(EgError::Usage(format!(
            "mem write: body exceeds {MAX_BODY_BYTES} bytes (got {body_len})"
        )));
    }
    if contains_suspected_secret(body) {
        return Err(EgError::Usage(SUSPECTED_SECRET_MSG.into()));
    }
    Ok(())
}

fn normalize_token_set(raw: &str, max: usize, max_len: usize, label: &str) -> Result<String> {
    let tokens: Vec<String> = raw.split_whitespace().map(str::to_string).collect();
    if tokens.len() > max {
        return Err(EgError::Usage(format!(
            "mem write: more than {max} {label}"
        )));
    }
    if let Some(t) = tokens.iter().find(|t| t.len() > max_len) {
        return Err(EgError::Usage(format!(
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

/// Validate the raw scope flag and resolve the relative lifetime together:
/// `durable` never expires and rejects `--ttl`; `session` expires at
/// `now + ttl`, defaulting to the 7-day OQ2 constant without one. Returns the
/// parsed variant so the note carries the enum, not the string.
fn normalize_ttl(scope: &str, ttl: Option<u64>) -> Result<(MemScope, i64)> {
    let scope = MemScope::parse(scope).ok_or_else(|| {
        EgError::Usage(format!(
            "mem write: scope must be '{}' or '{}' (got '{scope}')",
            MemScope::Durable.as_str(),
            MemScope::Session.as_str()
        ))
    })?;
    let ttl_secs = match scope {
        MemScope::Durable => {
            if ttl.is_some() {
                return Err(EgError::Usage(
                    "mem write: --ttl is only valid with --scope session".to_string(),
                ));
            }
            0
        }
        MemScope::Session => match ttl {
            None => DEFAULT_SESSION_TTL_SECS,
            Some(t) if t == 0 || t > MAX_SESSION_TTL_SECS => {
                return Err(EgError::Usage(format!(
                    "mem write: --ttl must be in 1..={MAX_SESSION_TTL_SECS} seconds"
                )));
            }
            Some(t) => t as i64,
        },
    };
    Ok((scope, ttl_secs))
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
