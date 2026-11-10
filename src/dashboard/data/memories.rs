//! Memory-store aggregation for the dashboard memories panel.
//! Correlates local and global memory notes with recall telemetry from
//! usage entries and verifies ref targets against the code index.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::db;
use crate::error::Result;

pub fn collect_memories(root: Option<&Path>, entries: &[Value]) -> Result<Vec<Value>> {
    let stores = discover_memory_stores(root, entries);
    let recalls_map = aggregate_recalls(entries);
    let mut notes_json = Vec::new();

    for (store_name, (repo_root, db_path)) in stores {
        collect_store_notes(
            &store_name,
            &repo_root,
            &db_path,
            entries,
            &recalls_map,
            &mut notes_json,
        );
    }

    Ok(notes_json)
}

fn discover_memory_stores(
    root: Option<&Path>,
    entries: &[Value],
) -> HashMap<String, (PathBuf, PathBuf)> {
    let mut stores: HashMap<String, (PathBuf, PathBuf)> = HashMap::new();

    if let Ok(g_dir) = db::global_eg_dir() {
        let g_db = g_dir.join("memory.db");
        if g_db.is_file() {
            stores.insert("global".to_string(), (g_dir, g_db));
        }
    }

    let mut locals: HashMap<String, PathBuf> = HashMap::new();
    super::discover_repos(root, entries, "memory.db", &mut locals);
    let store_pair = |r: PathBuf| {
        let l_db = r.join(".engram").join("memory.db");
        (r, l_db)
    };
    for (name, r) in locals {
        stores.entry(name).or_insert_with(|| store_pair(r));
    }

    stores
}

fn aggregate_recalls(entries: &[Value]) -> HashMap<String, (usize, f64)> {
    let mut recalls_map: HashMap<String, (usize, f64)> = HashMap::new();
    for e in entries {
        let tool = e.get("tool").and_then(Value::as_str).unwrap_or("");
        if tool != "mem_get" && tool != "mem_query" {
            continue;
        }
        let ts = e.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
        let args = e.get("args").cloned().unwrap_or(Value::Null);
        let key = args.get("key").and_then(Value::as_str).unwrap_or("");
        if !key.is_empty() {
            let rec = recalls_map.entry(key.to_string()).or_insert((0, 0.0));
            rec.0 += 1;
            if ts > rec.1 {
                rec.1 = ts;
            }
        } else if tool == "mem_query" {
            let terms = args.get("terms").and_then(Value::as_str).unwrap_or("");
            if terms.trim().is_empty() {
                let rec = recalls_map.entry(String::new()).or_insert((0, 0.0));
                rec.0 += 1;
                if ts > rec.1 {
                    rec.1 = ts;
                }
            }
        }
    }
    recalls_map
}

fn kind_prevention_card(kind: &str, key: &str) -> (&'static str, &'static str, String) {
    match crate::memory::MemKind::parse(kind) {
        Some(crate::memory::MemKind::Gotcha) => (
            "REGRESSION GUARD",
            "Regression & Bug Guard",
            format!("Prevents repeating root-cause trap in '{key}'. Blocks known fragility before changes are committed."),
        ),
        Some(crate::memory::MemKind::Decision) => (
            "ARCHITECTURE INVARIANT",
            "Architecture & Policy Guard",
            format!("Enforces repository architectural constraint '{key}'. Prevents forbidden abstractions or boundary violations."),
        ),
        Some(crate::memory::MemKind::Fact) => (
            "EXPLORATION SHORTCUT",
            "Verified Fact Shortcut",
            format!("Avoids costly multi-file grep scans by caching verified system truth for '{key}'."),
        ),
        Some(crate::memory::MemKind::Procedure) => (
            "OPERATIONAL PLAYBOOK",
            "Reusable Procedure",
            format!("Provides a verified runbook for '{key}' that the agent can rehearse at task start."),
        ),
        Some(crate::memory::MemKind::Task) | None => (
            "HANDOFF CONTEXT",
            "Cross-Session Handoff Guard",
            format!("Preserves active state across conversation turns for '{key}'."),
        ),
    }
}

fn estimate_fallback_recall(key: &str, body: &str, entries: &[Value]) -> (usize, f64) {
    let key_lower = key.to_lowercase();
    let body_lower = body.to_lowercase();
    let mut count = 0;
    let mut last_ts: f64 = 0.0;
    for e in entries {
        let tool = e.get("tool").and_then(Value::as_str).unwrap_or("");
        if tool != "mem_query" && tool != "mem_get" {
            continue;
        }
        let args = e.get("args").cloned().unwrap_or(Value::Null);
        let terms = args
            .get("terms")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        if !terms.is_empty() && (key_lower.contains(&terms) || body_lower.contains(&terms)) {
            count += 1;
            let ts = e.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
            if ts > last_ts {
                last_ts = ts;
            }
        }
    }
    (count, last_ts)
}

fn resolve_single_ref(r: &str, index_conn: Option<&Connection>, created_at: i64) -> Value {
    let ref_val = json!({
        "ref": r,
        "verified": false,
        "status": "broken",
        "type": "path",
        "location": ""
    });
    let Some(iconn) = index_conn else {
        return ref_val;
    };

    if let Ok(mut f_stmt) =
        iconn.prepare("SELECT path, size, mtime FROM files WHERE path = ?1 OR path LIKE ?2 LIMIT 1")
    {
        if let Ok((path, size, mtime_nanos)) =
            f_stmt.query_row(rusqlite::params![r, format!("%/{r}")], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
        {
            let is_modified = (mtime_nanos / 1_000_000_000) > created_at;
            return json!({
                "ref": r,
                "verified": true,
                "status": if is_modified { "modified" } else { "valid" },
                "type": "file",
                "location": format!("{path} ({size} bytes)")
            });
        }
    }

    if let Ok(mut s_stmt) = iconn.prepare(
        "SELECT f.path, s.line_start, s.line_end, f.mtime FROM symbols s JOIN files f ON s.file_id = f.id WHERE s.name = ?1 OR s.qualname = ?1 LIMIT 1",
    ) {
        if let Ok((fp, ls, le, mtime_nanos)) = s_stmt.query_row(rusqlite::params![r], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?))
        }) {
            let is_modified = (mtime_nanos / 1_000_000_000) > created_at;
            return json!({
                "ref": r,
                "verified": true,
                "status": if is_modified { "modified" } else { "valid" },
                "type": "symbol",
                "location": format!("{fp}:{ls}-{le}")
            });
        }
    }

    ref_val
}

fn resolve_note_refs(
    refs_list: &[String],
    index_conn: Option<&Connection>,
    created_at: i64,
) -> Vec<Value> {
    refs_list
        .iter()
        .map(|r| resolve_single_ref(r, index_conn, created_at))
        .collect()
}

struct MemoryRow {
    id: i64,
    key: String,
    rev: i64,
    kind: String,
    scope: String,
    session: String,
    body: String,
    tags_raw: String,
    refs_raw: String,
    content_hash: i64,
    source: String,
    created_at: i64,
    expires_at: i64,
    superseded_by: Option<i64>,
    superseded_at: i64,
}

impl MemoryRow {
    fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            key: r.get(1)?,
            rev: r.get(2)?,
            kind: r.get(3)?,
            scope: r.get(4)?,
            session: r.get(5)?,
            body: r.get(6)?,
            tags_raw: r.get(7)?,
            refs_raw: r.get(8)?,
            content_hash: r.get(9)?,
            source: r.get(10)?,
            created_at: r.get(11)?,
            expires_at: r.get(12)?,
            superseded_by: r.get(13)?,
            superseded_at: r.get(14)?,
        })
    }
}

fn format_note_json(
    store_name: &str,
    row: MemoryRow,
    index_conn: Option<&Connection>,
    recalls_map: &HashMap<String, (usize, f64)>,
    entries: &[Value],
) -> Value {
    let tags: Vec<String> = row
        .tags_raw
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    let refs_list: Vec<String> = row
        .refs_raw
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    let (prev_cat, prev_title, prev_detail) = kind_prevention_card(&row.kind, &row.key);

    let (bare_recalls, bare_ts) = recalls_map.get("").copied().unwrap_or((0, 0.0));
    let mut recall_count = recalls_map.get(&row.key).map(|r| r.0).unwrap_or(0) + bare_recalls;
    let mut last_recalled_ts = recalls_map
        .get(&row.key)
        .map(|r| r.1)
        .unwrap_or(0.0)
        .max(bare_ts);
    if recall_count == 0 {
        let (fallback_count, fallback_ts) = estimate_fallback_recall(&row.key, &row.body, entries);
        recall_count = fallback_count;
        last_recalled_ts = last_recalled_ts.max(fallback_ts);
    }

    let resolved_refs = resolve_note_refs(&refs_list, index_conn, row.created_at);
    let ref_status = if resolved_refs
        .iter()
        .any(|r| r.get("status").and_then(Value::as_str) == Some("broken"))
    {
        "broken"
    } else if resolved_refs
        .iter()
        .any(|r| r.get("status").and_then(Value::as_str) == Some("modified"))
    {
        "modified"
    } else if !resolved_refs.is_empty() {
        "valid"
    } else {
        "none"
    };

    json!({
        "id": row.id,
        "store": store_name,
        "key": row.key,
        "rev": row.rev,
        "kind": row.kind,
        "scope": row.scope,
        "session": row.session,
        "body": row.body,
        "tags": tags,
        "refs": refs_list,
        "resolved_refs": resolved_refs,
        "ref_status": ref_status,
        "content_hash": format!("{:x}", row.content_hash),
        "source": row.source,
        "created_at": row.created_at,
        "expires_at": row.expires_at,
        "is_current": row.superseded_by.is_none(),
        "superseded_by": row.superseded_by,
        "superseded_at": row.superseded_at,
        "prev_category": prev_cat,
        "prev_title": prev_title,
        "prev_detail": prev_detail,
        "recall_count": recall_count,
        "last_recalled_ts": last_recalled_ts,
    })
}

fn collect_store_notes(
    store_name: &str,
    repo_root: &Path,
    db_path: &Path,
    entries: &[Value],
    recalls_map: &HashMap<String, (usize, f64)>,
    out: &mut Vec<Value>,
) {
    let conn =
        match Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(c) => c,
            Err(_) => return,
        };
    let index_conn = Connection::open_with_flags(
        repo_root.join(".engram").join("index.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok();

    let sql = "SELECT id, key, rev, kind, scope, session, body, tags, refs, content_hash, source, created_at, expires_at, superseded_by, superseded_at FROM memories ORDER BY key ASC, rev DESC";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return,
    };

    let rows = stmt.query_map([], MemoryRow::from_row);

    let Ok(mapped_rows) = rows else {
        return;
    };
    for row in mapped_rows.flatten() {
        out.push(format_note_json(
            store_name,
            row,
            index_conn.as_ref(),
            recalls_map,
            entries,
        ));
    }
}
