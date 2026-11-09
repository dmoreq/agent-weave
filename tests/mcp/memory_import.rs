//! Portable-memory exchange over MCP: fenced Markdown export and
//! reject-on-conflict import, byte-identical to the CLI. Import paths are
//! repo-relative and confined to the project store.

use std::fs;

use serde_json::json;

use super::common::{
    cli_mem, eg_cmd, mem_fixture, notification, result_text, rpc, spawn_with, strip_footer,
};

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
