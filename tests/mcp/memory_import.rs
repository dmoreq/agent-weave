//! Portable-memory exchange over MCP: fenced Markdown export and
//! reject-on-conflict import, byte-identical to the CLI. Import paths are
//! repo-relative and confined to the project store.

use std::fs;

use serde_json::json;

use super::common::{
    cli_mem, eg_cmd, mem_fixture, notification, result_text, rpc, spawn_with, strip_footer,
};

#[test]
fn mem_import_rejects_traversal_and_allowlist_keys() {
    let repo = mem_fixture("import-guard");
    let outside = repo.parent().unwrap().join("outside-import.jsonl");
    fs::write(
        &outside,
        r#"{"key":"outside","kind":"fact","body":"outside"}"#,
    )
    .unwrap();
    fs::write(
        repo.join("allow.jsonl"),
        r#"{"key":"reuse-allow/a->b","kind":"decision","body":"intentional"}"#,
    )
    .unwrap();

    let mut srv = spawn_with(&repo, true);
    notification(&mut srv, "notifications/initialized");
    let traversal = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "mem_import", "arguments": {"path": "../outside-import.jsonl"}}),
    );
    assert_eq!(traversal["result"]["isError"], true, "{traversal}");
    let allowlist = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "mem_import", "arguments": {"path": "allow.jsonl"}}),
    );
    assert_eq!(allowlist["result"]["isError"], true, "{allowlist}");
    let stdin_path = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({"name": "mem_import", "arguments": {"path": "-"}}),
    );
    assert_eq!(stdin_path["result"]["isError"], true, "{stdin_path}");
    let _ = fs::remove_file(outside);
    let _ = fs::remove_dir_all(&repo);
}

#[cfg(unix)]
#[test]
fn mem_import_rejects_non_regular_files() {
    use std::process::Command;
    let repo = mem_fixture("import-fifo");
    let fifo = repo.join("notes.fifo");
    let status = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(status.success());
    let mut srv = spawn_with(&repo, true);
    notification(&mut srv, "notifications/initialized");
    let resp = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "mem_import", "arguments": {"path": "notes.fifo"}}),
    );
    assert_eq!(resp["result"]["isError"], true, "{resp}");
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn mem_import_and_markdown_export_parity() {
    let repo = mem_fixture("import");
    let (code, _stdout, stderr) = cli_mem(
        &repo,
        &[
            "write",
            "md-note",
            "--body",
            "run the release checklist before tagging",
            "--kind",
            "procedure",
        ],
    );
    assert_eq!(code, 0, "{stderr}");
    let cli_md = eg_cmd(&repo)
        .args(["mem", "export", "--format", "markdown"])
        .output()
        .unwrap();
    assert_eq!(cli_md.status.code(), Some(0));
    let markdown = String::from_utf8_lossy(&cli_md.stdout).into_owned();
    fs::write(repo.join("memory.md"), &markdown).unwrap();

    let mut srv = spawn_with(&repo, true);
    notification(&mut srv, "notifications/initialized");
    let list = rpc(&mut srv, 1, "tools/list", json!({}));
    let names: Vec<String> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"mem_import".to_string()), "{names:?}");

    let export = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "mem_export", "arguments": {"format": "markdown"}}),
    );
    assert_eq!(export["result"]["isError"], false, "{export}");
    assert_eq!(
        strip_footer(&result_text(&export)),
        markdown,
        "markdown export byte parity"
    );

    let cli_path = repo.join("memory.md");
    let (code, cli_import, _stderr) = cli_mem(&repo, &["import", cli_path.to_str().unwrap()]);
    assert_eq!(code, 0, "{cli_import}");
    let import = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({"name": "mem_import", "arguments": {"path": "memory.md"}}),
    );
    assert_eq!(import["result"]["isError"], false, "{import}");
    assert_eq!(
        strip_footer(&result_text(&import)),
        cli_import,
        "mem_import byte parity"
    );

    let _ = fs::remove_dir_all(&repo);
}
