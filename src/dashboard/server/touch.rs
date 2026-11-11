//! Touch handler: one store-addressed mutation with a revision precondition.

use std::net::TcpStream;
use std::path::Path;

use super::http::{send_response, touch_request_allowed, HttpRequest};
use super::*;

pub(super) fn handle_memory_touch(
    stream: &mut TcpStream,
    root: Option<&Path>,
    log_path: &Path,
    req: &HttpRequest,
) -> std::io::Result<()> {
    if let Err((status, message)) = touch_request_allowed(req) {
        return touch_reply(
            stream,
            status,
            &serde_json::json!({ "ok": false, "error": message }),
        );
    }
    if req.body.len() > 4096 {
        return touch_reply(
            stream,
            400,
            &serde_json::json!({ "ok": false, "error": "body exceeds 4096 bytes" }),
        );
    }
    let val: serde_json::Value =
        serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
    let key = val
        .get("key")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let store_id = val
        .get("store_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let Some(expected_rev) = val.get("rev").and_then(serde_json::Value::as_i64) else {
        return touch_reply(
            stream,
            400,
            &serde_json::json!({ "ok": false, "error": "missing rev: reload the dashboard" }),
        );
    };
    if key.is_empty() || store_id.is_empty() {
        return touch_reply(
            stream,
            400,
            &serde_json::json!({ "ok": false, "error": "missing key or store_id" }),
        );
    }

    let entries = data::read_usage_entries(log_path);
    let Some((store_name, _repo_root, db_path)) = data::resolve_store(root, &entries, store_id)
    else {
        let available: Vec<String> = data::store_ids(root, &entries);
        return touch_reply(
            stream,
            404,
            &serde_json::json!({
                "ok": false,
                "error": format!("unknown store '{store_id}'"),
                "key": key,
                "requested_store": store_id,
                "available_stores": available,
            }),
        );
    };

    let mut conn = match crate::db::open_memory_mutation_at(&db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("eg dashboard: touch store open failed: {store_name} {e}");
            return touch_reply(
                stream,
                503,
                &serde_json::json!({ "ok": false, "error": format!("store open failed: {e}"), "store": store_name, "store_id": store_id }),
            );
        }
    };

    let current_rev: Option<i64> = conn
        .query_row(
            "SELECT rev FROM memories WHERE key = ?1 AND superseded_by IS NULL LIMIT 1",
            rusqlite::params![key],
            |r| r.get(0),
        )
        .ok();
    let Some(current_rev) = current_rev else {
        return touch_reply(
            stream,
            404,
            &serde_json::json!({ "ok": false, "error": format!("unknown key '{key}'"), "store": store_name, "store_id": store_id }),
        );
    };
    if expected_rev != current_rev {
        return touch_reply(
            stream,
            409,
            &serde_json::json!({
                "ok": false,
                "error": format!("revision changed (you saw r{expected_rev}, current r{current_rev}) — reload"),
                "store": store_name,
                "store_id": store_id,
                "rev": current_rev,
            }),
        );
    }

    let now = crate::db::unix_now();
    match crate::db::memory_touch_if_rev(&mut conn, key, expected_rev, now) {
        Ok(Some(rev)) => {
            eprintln!(
                "eg dashboard: touch ok key={key} store={store_name} rev={rev} store_id={store_id}"
            );
            touch_reply(
                stream,
                200,
                &serde_json::json!({ "ok": true, "key": key, "store": store_name, "store_id": store_id, "rev": rev, "anchored_at": now }),
            )
        }
        Ok(None) => touch_reply(
            stream,
            409,
            &serde_json::json!({ "ok": false, "error": "revision moved during verify — reload", "store": store_name, "store_id": store_id }),
        ),
        Err(e) => {
            eprintln!("eg dashboard: touch write failed key={key} store={store_name}: {e}");
            touch_reply(
                stream,
                503,
                &serde_json::json!({ "ok": false, "error": format!("store write failed: {e}"), "store": store_name, "store_id": store_id }),
            )
        }
    }
}

fn touch_reply(
    stream: &mut TcpStream,
    status: u16,
    body: &serde_json::Value,
) -> std::io::Result<()> {
    let encoded = serde_json::to_vec(body).unwrap_or_default();
    send_response(
        stream,
        status,
        "application/json; charset=utf-8",
        &encoded,
        false,
    )
}
