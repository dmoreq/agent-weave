//! Memory-store aggregation for the dashboard memories panel.
//! Correlates local and global memory notes with recall telemetry from
//! usage entries and verifies ref targets against the code index.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::db;
use crate::error::Result;

pub fn collect_memories(
    root: Option<&Path>,
    global_only: bool,
    entries: &[Value],
) -> Result<Vec<Value>> {
    let mut stores: HashMap<String, (PathBuf, PathBuf)> = HashMap::new();

    if let Ok(g_dir) = db::global_aw_dir() {
        let g_db = g_dir.join("memory.db");
        if g_db.is_file() {
            stores.insert("global".to_string(), (g_dir, g_db));
        }
    }

    if !global_only {
        let mut locals: HashMap<String, PathBuf> = HashMap::new();
        super::discover_repos(root, entries, "memory.db", &mut locals);
        // An explicit root replaces a same-named store (today's short-circuit
        // overwrite); discovered repos only fill gaps, matching the scan's
        // contains_key guard against the already-inserted global store.
        let store_pair = |r: PathBuf| {
            let l_db = r.join(".aw").join("memory.db");
            (r, l_db)
        };
        if root.is_some() {
            stores.extend(locals.into_iter().map(|(n, r)| (n, store_pair(r))));
        } else {
            for (name, r) in locals {
                stores.entry(name).or_insert_with(|| store_pair(r));
            }
        }
    }

    // Correlate recall counts from usage entries
    let mut recalls_map: HashMap<String, (usize, f64)> = HashMap::new();
    for e in entries {
        let tool = e.get("tool").and_then(Value::as_str).unwrap_or("");
        if tool == "mem_get" || tool == "mem_query" {
            let ts = e.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
            let args = e.get("args").cloned().unwrap_or(Value::Null);
            let key = args.get("key").and_then(Value::as_str).unwrap_or("");
            if !key.is_empty() {
                let rec = recalls_map.entry(key.to_string()).or_insert((0, 0.0));
                rec.0 += 1;
                if ts > rec.1 {
                    rec.1 = ts;
                }
            }
        }
    }

    let mut notes_json = Vec::new();

    for (store_name, (repo_root, db_path)) in stores {
        let conn =
            match Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            {
                Ok(c) => c,
                Err(_) => continue,
            };

        let index_conn = Connection::open_with_flags(
            repo_root.join(".aw").join("index.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .ok();

        let mut stmt = match conn.prepare(
            "SELECT id, key, rev, kind, scope, session, body, tags, refs, content_hash, source, created_at, expires_at, superseded_by, superseded_at FROM memories ORDER BY key ASC, rev DESC"
        ) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let rows = stmt.query_map([], |r| {
            let id: i64 = r.get(0)?;
            let key: String = r.get(1)?;
            let rev: i64 = r.get(2)?;
            let kind: String = r.get(3)?;
            let scope: String = r.get(4)?;
            let session: String = r.get(5)?;
            let body: String = r.get(6)?;
            let tags_raw: String = r.get(7)?;
            let refs_raw: String = r.get(8)?;
            let content_hash: i64 = r.get(9)?;
            let source: String = r.get(10)?;
            let created_at: i64 = r.get(11)?;
            let expires_at: i64 = r.get(12)?;
            let superseded_by: Option<i64> = r.get(13)?;
            let superseded_at: i64 = r.get(14)?;
            Ok((
                id,
                key,
                rev,
                kind,
                scope,
                session,
                body,
                tags_raw,
                refs_raw,
                content_hash,
                source,
                created_at,
                expires_at,
                superseded_by,
                superseded_at,
            ))
        });

        if let Ok(mapped_rows) = rows {
            for row in mapped_rows.flatten() {
                let (
                    id,
                    key,
                    rev,
                    kind,
                    scope,
                    session,
                    body,
                    tags_raw,
                    refs_raw,
                    content_hash,
                    source,
                    created_at,
                    expires_at,
                    superseded_by,
                    superseded_at,
                ) = row;
                let is_current = superseded_by.is_none();
                let tags: Vec<String> =
                    tags_raw.split_whitespace().map(|s| s.to_string()).collect();
                let refs_list: Vec<String> =
                    refs_raw.split_whitespace().map(|s| s.to_string()).collect();

                let (prev_cat, prev_title, prev_detail) = match crate::memory::MemKind::parse(&kind)
                {
                    Some(crate::memory::MemKind::Gotcha) => (
                        "REGRESSION GUARD",
                        "Regression & Bug Guard",
                        format!("Prevents repeating root-cause trap in '{}'. Blocks known fragility before changes are committed.", key),
                    ),
                    Some(crate::memory::MemKind::Decision) => (
                        "ARCHITECTURE INVARIANT",
                        "Architecture & Policy Guard",
                        format!("Enforces repository architectural constraint '{}'. Prevents forbidden abstractions or boundary violations.", key),
                    ),
                    Some(crate::memory::MemKind::Fact) => (
                        "EXPLORATION SHORTCUT",
                        "Verified Fact Shortcut",
                        format!("Avoids costly multi-file grep scans by caching verified system truth for '{}'.", key),
                    ),
                    Some(crate::memory::MemKind::Task) | None => (
                        "HANDOFF CONTEXT",
                        "Cross-Session Handoff Guard",
                        format!("Preserves active state across conversation turns for '{}'.", key),
                    ),
                };

                let mut recall_count = recalls_map.get(&key).map(|r| r.0).unwrap_or(0);
                let mut last_recalled_ts = recalls_map.get(&key).map(|r| r.1).unwrap_or(0.0);

                if recall_count == 0 {
                    let key_lower = key.to_lowercase();
                    let body_lower = body.to_lowercase();
                    for e in entries {
                        let tool = e.get("tool").and_then(Value::as_str).unwrap_or("");
                        if tool == "mem_query" || tool == "mem_get" {
                            let args = e.get("args").cloned().unwrap_or(Value::Null);
                            let terms = args
                                .get("terms")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_lowercase();
                            if !terms.is_empty()
                                && (key_lower.contains(&terms) || body_lower.contains(&terms))
                            {
                                recall_count += 1;
                                let ts = e.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
                                if ts > last_recalled_ts {
                                    last_recalled_ts = ts;
                                }
                            }
                        }
                    }
                }

                // Verify refs against index.db
                let mut resolved_refs = Vec::new();
                for r in &refs_list {
                    let mut ref_val = json!({
                        "ref": r,
                        "verified": false,
                        "type": "path",
                        "location": ""
                    });

                    if let Some(ref iconn) = index_conn {
                        if let Ok(mut f_stmt) = iconn.prepare(
                            "SELECT path, size FROM files WHERE path = ?1 OR path LIKE ?2 LIMIT 1",
                        ) {
                            if let Ok(f_row) =
                                f_stmt.query_row(rusqlite::params![r, format!("%{}", r)], |row| {
                                    let path: String = row.get(0)?;
                                    let size: i64 = row.get(1)?;
                                    Ok((path, size))
                                })
                            {
                                ref_val = json!({
                                    "ref": r,
                                    "verified": true,
                                    "type": "file",
                                    "location": format!("{} ({} bytes)", f_row.0, f_row.1)
                                });
                            }
                        }
                        if !ref_val
                            .get("verified")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                        {
                            if let Ok(mut s_stmt) = iconn.prepare("SELECT file_path, line_start, line_end FROM symbols WHERE name = ?1 OR qualname = ?1 LIMIT 1") {
                                if let Ok(s_row) = s_stmt.query_row(rusqlite::params![r], |row| {
                                    let fp: String = row.get(0)?;
                                    let ls: i64 = row.get(1)?;
                                    let le: i64 = row.get(2)?;
                                    Ok((fp, ls, le))
                                }) {
                                    ref_val = json!({
                                        "ref": r,
                                        "verified": true,
                                        "type": "symbol",
                                        "location": format!("{}:{}-{}", s_row.0, s_row.1, s_row.2)
                                    });
                                }
                            }
                        }
                    }
                    resolved_refs.push(ref_val);
                }

                notes_json.push(json!({
                    "id": id,
                    "store": store_name,
                    "key": key,
                    "rev": rev,
                    "kind": kind,
                    "scope": scope,
                    "session": session,
                    "body": body,
                    "tags": tags,
                    "refs": refs_list,
                    "resolved_refs": resolved_refs,
                    "content_hash": format!("{content_hash:x}"),
                    "source": source,
                    "created_at": created_at,
                    "expires_at": expires_at,
                    "is_current": is_current,
                    "superseded_by": superseded_by,
                    "superseded_at": superseded_at,
                    "prev_category": prev_cat,
                    "prev_title": prev_title,
                    "prev_detail": prev_detail,
                    "recall_count": recall_count,
                    "last_recalled_ts": last_recalled_ts,
                }));
            }
        }
    }

    Ok(notes_json)
}
