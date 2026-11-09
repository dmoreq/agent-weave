//! Project memory over MCP (companion design, Phase 1): OQ1 puts mem_write +
//! mem_query in the default `eg mcp` tool set (ToolSpec::core); the other four
//! stay behind --all-tools. Every call dispatches through service::execute, so
//! the result text is the CLI's stdout byte for byte.

use std::fs;

use serde_json::json;

use super::common::{
    cli_mem, eg_cmd, fixture, init_and_list_tools, mem_fixture, notification, result_text, rpc,
    spawn_with, spawn_with_global,
};

#[test]
fn mem_core_tools_default_surface_and_byte_parity() {
    let repo = mem_fixture("core");
    // CLI reference output first (the server would journal its own calls).
    let (code, cli_write, cli_stderr) = cli_mem(
        &repo,
        &[
            "write",
            "parity-note",
            "--body",
            "the resolver keeps confidence tiers over resolved edges",
            "--kind",
            "decision",
            "--tags",
            "resolver confidence",
            "--refs",
            "src/query/callgraph.rs",
        ],
    );
    assert_eq!(code, 0, "{cli_write}{cli_stderr}");

    // spawn() defaults to --all-tools; the default surface needs spawn_with(false).
    let mut srv = spawn_with(&repo, false);
    let init = rpc(
        &mut srv,
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "engram");
    notification(&mut srv, "notifications/initialized");
    let list = rpc(&mut srv, 2, "tools/list", json!({}));
    let names: Vec<String> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"mem_write".to_string()), "{names:?}");
    assert!(names.contains(&"mem_query".to_string()), "{names:?}");
    for tool in ["mem_get", "mem_delete", "mem_gc", "mem_export"] {
        assert!(
            !names.contains(&tool.to_string()),
            "{tool} stays behind --all-tools: {names:?}"
        );
    }

    // A second note via MCP (duplicate content under a new key shape).
    let call = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({
            "name": "mem_write",
            "arguments": {
                "key": "parity-note-2",
                "body": "memory notes are candidates, never proof",
                "kind": "fact"
            }
        }),
    );
    assert_eq!(call["result"]["isError"], false, "{}", result_text(&call));

    // mem_query over MCP is byte-identical to the CLI stdout.
    let (code, cli_query, _stderr) = cli_mem(&repo, &["query", "confidence", "tiers"]);
    assert_eq!(code, 0);
    let query_call = rpc(
        &mut srv,
        4,
        "tools/call",
        json!({"name": "mem_query", "arguments": {"terms": "confidence tiers"}}),
    );
    assert_eq!(query_call["result"]["isError"], false);
    assert_eq!(result_text(&query_call), cli_query, "mem_query byte parity");

    // Bare mem_query with {} defaults to listing current notes (parity with query --all)
    let bare_query_call = rpc(
        &mut srv,
        5,
        "tools/call",
        json!({"name": "mem_query", "arguments": {}}),
    );
    assert_eq!(bare_query_call["result"]["isError"], false);
    assert!(result_text(&bare_query_call).contains("parity-note"));

    // Journal: MCP surface tag lands on mem tool rows.
    drop(srv);
    let log = fs::read_to_string(repo.join(".engram/usage.log")).unwrap();
    assert!(
        log.lines()
            .any(|l| l.contains("\"tool\":\"mem_write\"") && l.contains("\"surface\":\"mcp\"")),
        "{log}"
    );
}

#[test]
fn mem_all_tools_exposes_full_family_with_parity() {
    let repo = mem_fixture("all");
    let (code, _stdout, stderr) = cli_mem(
        &repo,
        &[
            "write",
            "export-me",
            "--body",
            "export parity body",
            "--kind",
            "fact",
        ],
    );
    assert_eq!(code, 0, "{stderr}");

    let mut srv = spawn_with(&repo, true);
    notification(&mut srv, "notifications/initialized");
    let list = rpc(&mut srv, 1, "tools/list", json!({}));
    let names: Vec<String> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    for tool in [
        "mem_write",
        "mem_query",
        "mem_get",
        "mem_delete",
        "mem_gc",
        "mem_export",
    ] {
        assert!(
            names.contains(&tool.to_string()),
            "{tool} missing: {names:?}"
        );
    }

    // mem_get parity.
    let (code, cli_get, _stderr) = cli_mem(&repo, &["get", "export-me"]);
    assert_eq!(code, 0);
    let get = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "mem_get", "arguments": {"key": "export-me"}}),
    );
    assert_eq!(result_text(&get), cli_get, "mem_get byte parity");

    // mem_gc parity (Phase 1 report-only).
    let (code, cli_gc, _stderr) = cli_mem(&repo, &["gc"]);
    assert_eq!(code, 0);
    let gc = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({"name": "mem_gc", "arguments": {}}),
    );
    assert_eq!(result_text(&gc), cli_gc, "mem_gc byte parity");

    // mem_export parity: pure JSONL text, caveat absent from the text.
    let (code, cli_export, _stderr) = cli_mem(&repo, &["export"]);
    assert_eq!(code, 0);
    let export = rpc(
        &mut srv,
        4,
        "tools/call",
        json!({"name": "mem_export", "arguments": {}}),
    );
    assert_eq!(result_text(&export), cli_export, "mem_export byte parity");
    assert!(
        !result_text(&export).contains("eg: memory"),
        "{:?}",
        result_text(&export)
    );

    // Phase 2 lifecycle over MCP: session scope + ttl + cross-key supersedes.
    let session = rpc(
        &mut srv,
        6,
        "tools/call",
        json!({"name": "mem_write", "arguments": {
            "key": "mcp-session-note", "body": "transient mcp scratch", "kind": "fact",
            "scope": "session", "ttl": 3600
        }}),
    );
    assert_eq!(
        session["result"]["isError"],
        false,
        "{}",
        result_text(&session)
    );
    let supersedes = rpc(
        &mut srv,
        7,
        "tools/call",
        json!({"name": "mem_write", "arguments": {
            "key": "mcp-replacement", "body": "replaces the export note", "kind": "fact",
            "supersedes": "export-me"
        }}),
    );
    assert_eq!(
        supersedes["result"]["isError"],
        false,
        "{}",
        result_text(&supersedes)
    );
    let (code, cli_all, _stderr) = cli_mem(&repo, &["query", "--all"]);
    assert_eq!(code, 0);
    assert!(
        cli_all.contains("mcp-session-note") && !cli_all.contains("export-me"),
        "{cli_all}"
    );
    let all = rpc(
        &mut srv,
        8,
        "tools/call",
        json!({"name": "mem_query", "arguments": {"all": true}}),
    );
    assert_eq!(result_text(&all), cli_all, "mem_query --all byte parity");
    let with_superseded = rpc(
        &mut srv,
        9,
        "tools/call",
        json!({"name": "mem_query", "arguments": {"all": true, "include_superseded": true}}),
    );
    let (code, cli_sup, _stderr) = cli_mem(&repo, &["query", "--all", "--include-superseded"]);
    assert_eq!(code, 0);
    assert!(
        result_text(&with_superseded).contains("[superseded]"),
        "{}",
        result_text(&with_superseded)
    );
    assert_eq!(
        result_text(&with_superseded),
        cli_sup,
        "include-superseded byte parity"
    );

    // mem_gc parity: expire the session note via the store, then compare.
    {
        let conn = rusqlite::Connection::open(repo.join(".engram/memory.db")).unwrap();
        conn.execute(
            "UPDATE memories SET expires_at = 1 WHERE key = 'mcp-session-note'",
            [],
        )
        .unwrap();
    }
    let (code, cli_gc_after, _stderr) = cli_mem(&repo, &["gc"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(cli_gc_after.contains("expired=1 removed"), "{cli_gc_after}");

    // mem_delete parity: MCP delete text equals the CLI delete text for the
    // same input (recreated between the two calls — delete is destructive).
    let write = rpc(
        &mut srv,
        10,
        "tools/call",
        json!({"name": "mem_write", "arguments": {"key": "doomed", "body": "delete parity body", "kind": "fact"}}),
    );
    assert_eq!(write["result"]["isError"], false);
    let delete = rpc(
        &mut srv,
        11,
        "tools/call",
        json!({"name": "mem_delete", "arguments": {"key": "doomed"}}),
    );
    assert_eq!(delete["result"]["isError"], false);
    // Recreate via CLI so the CLI delete sees the same input shape.
    let (code, _stdout, stderr) = cli_mem(
        &repo,
        &[
            "write",
            "doomed",
            "--body",
            "delete parity body",
            "--kind",
            "fact",
        ],
    );
    assert_eq!(code, 0, "{stderr}");
    let (code, cli_delete, _stderr) = cli_mem(&repo, &["delete", "doomed"]);
    assert_eq!(code, 0);
    assert_eq!(result_text(&delete), cli_delete, "mem_delete byte parity");

    // Error path: unknown key reaches the caller with the CLI's stderr text.
    let (code, _stdout, cli_err) = cli_mem(&repo, &["get", "missing"]);
    assert_eq!(code, 1);
    let missing = rpc(
        &mut srv,
        5,
        "tools/call",
        json!({"name": "mem_get", "arguments": {"key": "missing"}}),
    );
    assert_eq!(missing["result"]["isError"], true);
    assert!(
        result_text(&missing).contains("unknown memory key 'missing'"),
        "{}",
        result_text(&missing)
    );
    assert!(
        result_text(&missing).contains(cli_err.trim()),
        "{}",
        result_text(&missing)
    );
}

#[test]
fn global_memory_mcp_protocol_parity() {
    let repo = mem_fixture("mcp-global-memory");
    let global_dir = mem_fixture("mcp-global-eg");
    let mut srv = spawn_with_global(&repo, &global_dir);
    init_and_list_tools(&mut srv);

    // 1. mem_write with global: true
    let write_res = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({
            "name": "mem_write",
            "arguments": {
                "key": "user_pref",
                "body": "Language: Vietnamese. Style: Concise.",
                "kind": "decision",
                "global": true
            }
        }),
    );
    assert_eq!(write_res["result"]["isError"], false, "{write_res}");
    let text = result_text(&write_res);
    assert!(text.contains("ok: 'user_pref' rev 1 written"), "{text}");

    // Global file exists, local file does not
    assert!(global_dir.join("memory.db").exists());
    assert!(!repo.join(".engram/memory.db").exists());

    // 2. mem_query without global flag (layered query) returns the global note tagged [global]
    let query_res = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({
            "name": "mem_query",
            "arguments": {
                "terms": "Vietnamese"
            }
        }),
    );
    assert_eq!(query_res["result"]["isError"], false, "{query_res}");
    let text = result_text(&query_res);
    assert!(text.contains("user_pref r1 decision durable "));
    assert!(text.contains(" [global]"), "{text}");
    assert!(text.contains("Language: Vietnamese"), "{text}");

    // 3. mem_get with global: true
    let get_res = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({
            "name": "mem_get",
            "arguments": {
                "key": "user_pref",
                "global": true
            }
        }),
    );
    assert_eq!(get_res["result"]["isError"], false, "{get_res}");
    let text = result_text(&get_res);
    assert!(text.contains("user_pref r1 decision durable "));
    assert!(text.contains(" [global]"), "{text}");
    assert!(text.contains("revs: 1"), "{text}");

    // 4. mem_export with global: true
    let export_res = rpc(
        &mut srv,
        4,
        "tools/call",
        json!({
            "name": "mem_export",
            "arguments": {
                "global": true
            }
        }),
    );
    assert_eq!(export_res["result"]["isError"], false, "{export_res}");
    let text = result_text(&export_res);
    assert!(text.contains("\"key\":\"user_pref\""), "{text}");

    // 5. mem_gc with global: true
    let gc_res = rpc(
        &mut srv,
        5,
        "tools/call",
        json!({
            "name": "mem_gc",
            "arguments": {
                "global": true
            }
        }),
    );
    assert_eq!(gc_res["result"]["isError"], false, "{gc_res}");
    let text = result_text(&gc_res);
    assert!(
        text.contains("expired=0 removed, superseded=0 removed, durable=1 rows"),
        "{text}"
    );

    // 6. mem_delete with global: true
    let del_res = rpc(
        &mut srv,
        6,
        "tools/call",
        json!({
            "name": "mem_delete",
            "arguments": {
                "key": "user_pref",
                "global": true
            }
        }),
    );
    assert_eq!(del_res["result"]["isError"], false, "{del_res}");
    let text = result_text(&del_res);
    assert!(text.contains("deleted 'user_pref' revs=1"), "{text}");

    let _ = fs::remove_dir_all(&repo);
    let _ = fs::remove_dir_all(&global_dir);
}

#[test]
fn global_usage_mcp_returns_cross_repo_summary() {
    let repo = fixture("mcp-global-usage");
    let global_dir = mem_fixture("mcp-global-usage-eg");

    let build = eg_cmd(&repo)
        .env("ENGRAM_GLOBAL_DIR", &global_dir)
        .arg("build")
        .output()
        .unwrap();
    assert_eq!(build.status.code(), Some(0));

    let mut srv = spawn_with_global(&repo, &global_dir);
    init_and_list_tools(&mut srv);

    // Call search tool to generate telemetry
    let _ = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "search", "arguments": {"terms": "normalize"}}),
    );

    // Call usage tool with global: true via MCP
    let usage = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "usage", "arguments": {"global": true}}),
    );
    assert_eq!(usage["result"]["isError"], false, "{usage}");
    let text = result_text(&usage);
    assert!(text.contains("eg global usage:"), "header: {text}");
    assert!(text.contains("Speed: p50 ="), "speed: {text}");
    assert!(
        text.contains("Active Repositories:"),
        "active repos: {text}"
    );

    let _ = fs::remove_dir_all(&repo);
    let _ = fs::remove_dir_all(&global_dir);
}
