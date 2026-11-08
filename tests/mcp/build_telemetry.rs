//! Build, telemetry, and response-decoration behavior surfaced through MCP.

use std::fs;

use serde_json::json;

use super::common::{aw_cmd, fixture, init_and_list_tools, result_text, rpc, spawn};

#[test]
fn g10_build_via_mcp_journals_and_returns_stats_line() {
    let repo = fixture("build");
    let mut srv = spawn(&repo);
    let resp = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "build", "arguments": {}}),
    );
    assert_eq!(resp["result"]["isError"], false);
    let text = result_text(&resp);
    assert!(text.starts_with("aw: built index: files=2"), "{text}");
    assert!(text.contains("parse_errors=0"), "{text}");
    let log = fs::read_to_string(repo.join(".aw/usage.log")).unwrap();
    assert!(
        log.contains("\"tool\":\"build\"") && log.contains("\"surface\":\"mcp\""),
        "{log}"
    );
    let _ = fs::remove_dir_all(&repo);
}

#[cfg(unix)]
#[test]
fn mcp_build_rejects_symlinked_aw_directory() {
    use std::os::unix::fs::symlink;
    let repo = fixture("mcp-symlink-aw");
    let ext = std::env::temp_dir().join(format!("aw-ext-mcp-aw-{}", std::process::id()));
    let _ = fs::remove_dir_all(&ext);
    fs::create_dir_all(&ext).unwrap();
    let sentinel = ext.join("sentinel.txt");
    fs::write(&sentinel, "keep").unwrap();
    let _ = fs::remove_dir_all(repo.join(".aw"));
    symlink(&ext, repo.join(".aw")).unwrap();

    let mut srv = spawn(&repo);
    let build = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "build", "arguments": {}}),
    );
    assert_eq!(build["result"]["isError"], true, "{build}");
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");
    assert!(!ext.join("index.db").exists());
    let _ = fs::remove_dir_all(&ext);
}

#[test]
fn mcp_response_includes_kpi_footer() {
    let repo = fixture("kpi-footer");
    let build = aw_cmd(&repo).arg("build").output().unwrap();
    assert_eq!(build.status.code(), Some(0));

    let mut srv = spawn(&repo);
    init_and_list_tools(&mut srv);

    // 1. context
    let ctx = rpc(
        &mut srv,
        10,
        "tools/call",
        json!({"name": "context", "arguments": {"name": "normalize"}}),
    );
    assert_eq!(ctx["result"]["isError"], false);
    let text = result_text(&ctx);
    assert!(text.contains("\n───\n⚡ aw:"), "context footer: {text}");
    assert!(
        text.contains("tokens saved vs file reads"),
        "context tokens: {text}"
    );

    // 2. search
    let search = rpc(
        &mut srv,
        11,
        "tools/call",
        json!({"name": "search", "arguments": {"terms": "normalize"}}),
    );
    assert_eq!(search["result"]["isError"], false);
    let text = result_text(&search);
    assert!(text.contains("\n───\n⚡ aw:"), "search footer: {text}");
    assert!(
        text.contains("tokens saved vs file reads"),
        "search tokens: {text}"
    );

    // 3. reuse
    let reuse = rpc(
        &mut srv,
        12,
        "tools/call",
        json!({"name": "reuse", "arguments": {}}),
    );
    assert_eq!(reuse["result"]["isError"], false);
    let text = result_text(&reuse);
    assert!(text.contains("\n───\n⚡ aw:"), "reuse footer: {text}");
    assert!(
        text.contains("tokens saved vs file reads"),
        "reuse tokens: {text}"
    );

    // 4. callees on a leaf function (0 items -> clean footer without tokens saved)
    let leaf = rpc(
        &mut srv,
        13,
        "tools/call",
        json!({"name": "callees", "arguments": {"name": "normalize"}}),
    );
    assert_eq!(leaf["result"]["isError"], true);
    let leaf_text = result_text(&leaf);
    assert!(
        leaf_text.contains("\n───\n⚡ aw:"),
        "leaf footer: {leaf_text}"
    );
    assert!(
        leaf_text.contains("| 0 items"),
        "leaf zero items: {leaf_text}"
    );
    assert!(
        !leaf_text.contains("tokens saved"),
        "leaf no savings: {leaf_text}"
    );

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn mcp_usage_tool_returns_telemetry() {
    let repo = fixture("mcp-usage");
    let build = aw_cmd(&repo).arg("build").output().unwrap();
    assert_eq!(build.status.code(), Some(0));

    let mut srv = spawn(&repo);
    init_and_list_tools(&mut srv);

    // Call a couple of tools first to populate the journal
    let _ = rpc(
        &mut srv,
        10,
        "tools/call",
        json!({"name": "callers", "arguments": {"name": "normalize"}}),
    );
    let _ = rpc(
        &mut srv,
        11,
        "tools/call",
        json!({"name": "search", "arguments": {"terms": "normalize"}}),
    );

    // Now call usage tool via MCP
    let usage = rpc(
        &mut srv,
        12,
        "tools/call",
        json!({"name": "usage", "arguments": {}}),
    );
    assert_eq!(usage["result"]["isError"], false, "{usage}");
    let text = result_text(&usage);
    assert!(text.contains("aw usage:"), "header: {text}");
    assert!(text.contains("Speed: p50 ="), "speed: {text}");
    assert!(text.contains("Reliability:"), "reliability: {text}");
    assert!(text.contains("Estimated Token Savings:"), "savings: {text}");
    assert!(
        text.contains("Anti-Reinvention:"),
        "anti-reinvention: {text}"
    );
    assert!(text.contains("callers:"), "callers: {text}");
    // usage is an on-demand telemetry card; it must not append a redundant footer
    assert!(
        !text.contains("⚡ aw:"),
        "usage has no redundant footer: {text}"
    );

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn mcp_build_emits_incremental_reuse_hints() {
    let repo = fixture("build-hint");
    fs::write(
        repo.join("core.py"),
        "def transform_data(data: str):\n    return data.strip().lower()\n",
    )
    .unwrap();
    let build = aw_cmd(&repo).arg("build").output().unwrap();
    assert_eq!(build.status.code(), Some(0));

    fs::write(
        repo.join("extra.py"),
        "def transform_data(data: str):\n    return data.strip().lower()\n",
    )
    .unwrap();

    let mut srv = spawn(&repo);
    let resp = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "build", "arguments": {}}),
    );
    assert_eq!(resp["result"]["isError"], false);
    let text = result_text(&resp);
    assert!(
        text.contains("aw: reuse-hint:"),
        "MCP build output must include reuse hints on stderr: {text}"
    );
    assert!(text.contains("transform_data"), "{text}");
}
