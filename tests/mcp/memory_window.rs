//! MCP parity for memory windows, explain, local scope, legacy, and reindex.

use serde_json::json;

use super::common::{cli_mem, mem_fixture, notification, result_text, rpc, spawn_with, Server};

fn call(srv: &mut Server, id: u64, name: &str, args: serde_json::Value) -> serde_json::Value {
    rpc(
        srv,
        id,
        "tools/call",
        json!({"name": name, "arguments": args}),
    )
}

#[test]
fn unicode_query_and_explain_parity() {
    let repo = mem_fixture("window-unicode");
    let (code, _, err) = cli_mem(
        &repo,
        &[
            "write",
            "viet",
            "--body",
            "Bộ nhớ dự án lưu quyết định kiến trúc",
            "--kind",
            "fact",
        ],
    );
    assert_eq!(code, 0, "{err}");

    let mut srv = spawn_with(&repo, false);
    notification(&mut srv, "notifications/initialized");

    let (code, cli, _) = cli_mem(&repo, &["query", "bo nho"]);
    assert_eq!(code, 0);
    let mcp = call(&mut srv, 1, "mem_query", json!({"terms": "bo nho"}));
    assert_eq!(mcp["result"]["isError"], false);
    assert_eq!(result_text(&mcp), cli, "unicode query parity");

    let (code, cli, _) = cli_mem(&repo, &["query", "bo nho", "--explain"]);
    assert_eq!(code, 0);
    let mcp = call(
        &mut srv,
        2,
        "mem_query",
        json!({"terms": "bo nho", "explain": true}),
    );
    assert_eq!(mcp["result"]["isError"], false);
    assert_eq!(result_text(&mcp), cli, "explain parity");
}

#[test]
fn window_bounds_parity_and_empty_window() {
    let repo = mem_fixture("window-bounds");
    let (code, _, err) = cli_mem(
        &repo,
        &["write", "note", "--body", "window body", "--kind", "fact"],
    );
    assert_eq!(code, 0, "{err}");

    let mut srv = spawn_with(&repo, false);
    notification(&mut srv, "notifications/initialized");

    let (code, cli, _) = cli_mem(&repo, &["query", "window", "--since", "@0"]);
    assert_eq!(code, 0);
    let mcp = call(
        &mut srv,
        1,
        "mem_query",
        json!({"terms": "window", "since": "@0"}),
    );
    assert_eq!(result_text(&mcp), cli);

    let (code, _, _) = cli_mem(&repo, &["query", "window", "--until", "@0"]);
    assert_eq!(code, 1);
    let mcp = call(
        &mut srv,
        2,
        "mem_query",
        json!({"terms": "window", "until": "@0"}),
    );
    assert_eq!(mcp["result"]["isError"], true, "{mcp}");
}

#[test]
fn typed_rejections_for_new_params() {
    let repo = mem_fixture("window-typed");
    let (code, _, err) = cli_mem(
        &repo,
        &["write", "note", "--body", "typed body", "--kind", "fact"],
    );
    assert_eq!(code, 0, "{err}");

    let mut srv = spawn_with(&repo, false);
    notification(&mut srv, "notifications/initialized");

    let numeric = call(
        &mut srv,
        1,
        "mem_query",
        json!({"terms": "typed", "since": 1794441600}),
    );
    assert_eq!(numeric["result"]["isError"], true);
    assert!(
        result_text(&numeric).contains("must be a string"),
        "{numeric}"
    );

    let bad_bool = call(
        &mut srv,
        2,
        "mem_query",
        json!({"terms": "typed", "local": "true"}),
    );
    assert_eq!(bad_bool["result"]["isError"], true);
    assert!(
        result_text(&bad_bool).contains("must be a boolean"),
        "{bad_bool}"
    );

    let conflict = call(
        &mut srv,
        3,
        "mem_query",
        json!({"terms": "typed", "local": true, "global": true}),
    );
    assert_eq!(conflict["result"]["isError"], true);
    assert!(
        result_text(&conflict).contains("--local and --global are mutually exclusive"),
        "{conflict}"
    );
}

#[test]
fn export_window_and_reindex_are_exposed() {
    let repo = mem_fixture("window-export");
    let (code, _, err) = cli_mem(
        &repo,
        &["write", "note", "--body", "export body", "--kind", "fact"],
    );
    assert_eq!(code, 0, "{err}");

    let mut srv = spawn_with(&repo, true);
    notification(&mut srv, "notifications/initialized");

    let (code, cli, _) = cli_mem(&repo, &["export", "--since", "@0"]);
    assert_eq!(code, 0);
    let mcp = call(&mut srv, 1, "mem_export", json!({"since": "@0"}));
    assert_eq!(result_text(&mcp), cli, "export window parity");

    let reindex = call(&mut srv, 2, "mem_reindex", json!({}));
    assert_eq!(reindex["result"]["isError"], false, "{reindex}");
    assert!(
        result_text(&reindex).starts_with("reindexed "),
        "{}",
        result_text(&reindex)
    );
}

#[test]
fn local_and_legacy_parity() {
    let repo = mem_fixture("window-local-legacy");
    let (code, _, err) = cli_mem(
        &repo,
        &[
            "write",
            "viet",
            "--body",
            "Bộ nhớ dự án lưu quyết định",
            "--kind",
            "fact",
        ],
    );
    assert_eq!(code, 0, "{err}");

    let mut srv = spawn_with(&repo, false);
    notification(&mut srv, "notifications/initialized");

    let (code, cli, _) = cli_mem(&repo, &["query", "bộ nhớ", "--local"]);
    assert_eq!(code, 0);
    let mcp = call(
        &mut srv,
        1,
        "mem_query",
        json!({"terms": "bộ nhớ", "local": true}),
    );
    assert_eq!(result_text(&mcp), cli, "local parity");

    let (code, cli, _) = cli_mem(&repo, &["query", "bộ nhớ", "--legacy"]);
    assert_eq!(code, 0);
    let mcp = call(
        &mut srv,
        2,
        "mem_query",
        json!({"terms": "bộ nhớ", "legacy": true}),
    );
    assert_eq!(result_text(&mcp), cli, "legacy parity");
}
