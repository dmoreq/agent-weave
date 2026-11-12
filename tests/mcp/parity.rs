//! Tool listing and byte-parity conformance between the MCP server and the CLI.

use std::fs;

use serde_json::json;

use super::common::{
    eg_cmd, fixture, init_and_list_tools, result_text, rpc, spawn, spawn_with, strip_footer,
};

#[test]
fn g10_initialize_tools_list_and_parity() {
    let repo = fixture("parity");
    // Build the index before capturing CLI references.
    let build = eg_cmd(&repo).arg("build").output().unwrap();
    assert_eq!(build.status.code(), Some(0));
    // CLI reference outputs, captured before the server opens the journal.
    let cli_callers = eg_cmd(&repo)
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    assert_eq!(cli_callers.status.code(), Some(0));
    let cli_symbols = eg_cmd(&repo)
        .args(["symbols", "utils.py"])
        .output()
        .unwrap();
    let cli_symbols_outline = eg_cmd(&repo)
        .args(["symbols", "utils.py", "--outline"])
        .output()
        .unwrap();
    let cli_search = eg_cmd(&repo)
        .args(["search", "normalize"])
        .output()
        .unwrap();
    let mut srv = spawn(&repo);
    let names = init_and_list_tools(&mut srv);
    assert_eq!(
        names,
        [
            "build",
            "callers",
            "callees",
            "path",
            "context",
            "imports",
            "impact",
            "symbols",
            "reuse",
            "search",
            "usage",
            "mem_write",
            "mem_query",
            "mem_touch",
            "mem_get",
            "mem_delete",
            "mem_gc",
            "mem_export",
            "mem_import",
            "mem_reindex"
        ]
        .map(str::to_string)
    );
    assert!(
        !names.iter().any(|name| name == "doctor"),
        "doctor is not exposed (Amendment 8)"
    );

    // Byte parity with the CLI stdout.
    let callers = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({"name": "callers", "arguments": {"name": "normalize"}}),
    );
    assert_eq!(callers["result"]["isError"], false);
    assert_eq!(
        strip_footer(&result_text(&callers)),
        String::from_utf8_lossy(&cli_callers.stdout).into_owned()
    );
    assert!(result_text(&callers).contains("⚡ eg:"));

    let symbols = rpc(
        &mut srv,
        4,
        "tools/call",
        json!({"name": "symbols", "arguments": {"file": "utils.py"}}),
    );
    assert_eq!(
        strip_footer(&result_text(&symbols)),
        String::from_utf8_lossy(&cli_symbols.stdout).into_owned()
    );
    assert!(result_text(&symbols).contains("⚡ eg:"));

    let symbols_outline = rpc(
        &mut srv,
        8,
        "tools/call",
        json!({"name": "symbols", "arguments": {"file": "utils.py", "outline": true}}),
    );
    assert_eq!(
        strip_footer(&result_text(&symbols_outline)),
        String::from_utf8_lossy(&cli_symbols_outline.stdout).into_owned()
    );
    assert!(result_text(&symbols_outline).contains("⚡ eg:"));

    let search = rpc(
        &mut srv,
        9,
        "tools/call",
        json!({"name": "search", "arguments": {"terms": "normalize"}}),
    );
    assert_eq!(
        strip_footer(&result_text(&search)),
        String::from_utf8_lossy(&cli_search.stdout).into_owned()
    );
    assert!(result_text(&search).contains("⚡ eg:"));

    // R6e: parity covers every pointer tool that does not depend on
    // wall-clock timings or journal state.
    for (tool, args, cli_args) in [
        (
            "callers",
            json!({"name": "normalize", "depth": 2}),
            vec!["callers", "normalize", "--depth", "2"],
        ),
        (
            "callees",
            json!({"name": "caller"}),
            vec!["callees", "caller"],
        ),
        (
            "impact",
            json!({"file": "utils.py"}),
            vec!["impact", "utils.py"],
        ),
        (
            "imports",
            json!({"file": "app.py"}),
            vec!["imports", "app.py"],
        ),
        ("reuse", json!({}), vec!["reuse"]),
    ] {
        let cli = eg_cmd(&repo).args(&cli_args).output().unwrap();
        assert_eq!(cli.status.code(), Some(0), "{tool} CLI failed");
        let via_mcp = rpc(
            &mut srv,
            13,
            "tools/call",
            json!({"name": tool, "arguments": args}),
        );
        assert_eq!(
            strip_footer(&result_text(&via_mcp)),
            String::from_utf8_lossy(&cli.stdout).into_owned(),
            "{tool} byte parity"
        );
        assert!(result_text(&via_mcp).contains("⚡ eg:"));
    }

    // P8 no-result parity: imports on an unknown file (CLI stderr, MCP text).
    let cli_imports_missing = eg_cmd(&repo).args(["imports", "nope.py"]).output().unwrap();
    assert_eq!(cli_imports_missing.status.code(), Some(1));
    let imports_missing = rpc(
        &mut srv,
        15,
        "tools/call",
        json!({"name": "imports", "arguments": {"file": "nope.py"}}),
    );
    assert_eq!(imports_missing["result"]["isError"], true);
    assert_eq!(
        strip_footer(&result_text(&imports_missing)).trim_end(),
        String::from_utf8_lossy(&cli_imports_missing.stderr).trim_end()
    );
    assert!(result_text(&imports_missing).contains("⚡ eg:"));

    // G10 parity extends to context (Amendment 17).
    let cli_context = eg_cmd(&repo)
        .args(["context", "normalize"])
        .output()
        .unwrap();
    assert_eq!(cli_context.status.code(), Some(0));
    let context_call = rpc(
        &mut srv,
        15,
        "tools/call",
        json!({"name": "context", "arguments": {"name": "normalize"}}),
    );
    assert_eq!(
        strip_footer(&result_text(&context_call)),
        String::from_utf8_lossy(&cli_context.stdout).into_owned()
    );
    assert!(result_text(&context_call).contains("⚡ eg:"));

    // G10 parity extends to path (Amendment 13): hop pointers + chain line
    // byte-equal between the surfaces.
    let cli_path = eg_cmd(&repo)
        .args(["path", "caller", "normalize"])
        .output()
        .unwrap();
    assert_eq!(cli_path.status.code(), Some(0));
    let path_call = rpc(
        &mut srv,
        12,
        "tools/call",
        json!({"name": "path", "arguments": {"from": "caller", "to": "normalize"}}),
    );
    assert_eq!(
        strip_footer(&result_text(&path_call)),
        String::from_utf8_lossy(&cli_path.stdout).into_owned()
    );
    assert!(result_text(&path_call).contains("⚡ eg:"));
    assert!(
        result_text(&path_call).starts_with("eg: path — resolved static edges only;"),
        "path caveat first: {}",
        result_text(&path_call)
    );

    let unknown = rpc(
        &mut srv,
        7,
        "tools/call",
        json!({"name": "doctor", "arguments": {}}),
    );
    assert_eq!(
        unknown["error"]["code"], -32602,
        "unregistered tool rejected"
    );

    let log = fs::read_to_string(repo.join(".engram/usage.log")).unwrap();
    let mcp_lines = log
        .lines()
        .filter(|l| l.contains("\"surface\":\"mcp\""))
        .count();
    assert_eq!(mcp_lines, 12, "{log}");

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn g10_ambiguous_carries_candidates_and_iserror() {
    let repo = fixture("ambiguous");
    fs::write(
        repo.join("a.py"),
        "class A:\n    def process(self):\n        return 1\n",
    )
    .unwrap();
    fs::write(
        repo.join("b.py"),
        "class B:\n    def process(self):\n        return 2\n",
    )
    .unwrap();
    eg_cmd(&repo).arg("build").output().unwrap();

    let mut srv = spawn(&repo);
    let resp = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "callers", "arguments": {"name": "process"}}),
    );
    assert_eq!(
        resp["result"]["isError"], true,
        "ambiguous name is an error result"
    );
    let text = result_text(&resp);
    assert!(text.contains("a.py:2 a.A.process"), "{text}");
    assert!(text.contains("b.py:2 b.B.process"), "{text}");
    assert!(text.contains("ambiguous name 'process'"), "{text}");
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn reuse_arguments_via_mcp() {
    let repo = fixture("mcp-reuse");
    let build = eg_cmd(&repo).arg("build").output().unwrap();
    assert_eq!(build.status.code(), Some(0));
    let mut srv = spawn(&repo);

    let bad_paths = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "reuse", "arguments": {"paths": "not_an_array"}}),
    );
    assert_eq!(bad_paths["result"]["isError"], true, "{bad_paths}");

    let ok = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "reuse", "arguments": {"paths": ["utils.py"]}}),
    );
    assert_eq!(ok["result"]["isError"], false, "{ok}");

    let deep = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({"name": "reuse", "arguments": {"paths": ["utils.py"], "deep_callee": true}}),
    );
    assert_eq!(deep["result"]["isError"], false, "{deep}");
    let _ = fs::remove_dir_all(&repo);
}

/// Default MCP profile: the 13 core navigation tools plus the memory
/// write/query loop (OQ1). The full inventory/analysis surface stays
/// available behind `--all-tools`.
#[test]
fn default_mcp_exposes_core_navigation_tools() {
    let repo = fixture("core");
    let mut srv = spawn_with(&repo, false);
    let names = init_and_list_tools(&mut srv);
    assert_eq!(
        names,
        [
            "build",
            "callers",
            "callees",
            "path",
            "context",
            "imports",
            "impact",
            "symbols",
            "reuse",
            "search",
            "usage",
            "mem_write",
            "mem_query",
        ]
        .map(str::to_string)
    );

    let denied = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({"name": "mem_get", "arguments": {"key": "k"}}),
    );
    assert_eq!(denied["error"]["code"], -32602);
    assert!(denied["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown tool 'mem_get'"));
}
