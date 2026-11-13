//! Reuse MCP parity and judge/allowlist surface tests.

use std::fs;

use serde_json::json;

use super::common::*;

#[test]
fn mcp_allowlist_write_requires_confirm() {
    let repo = fixture("mcp-reuse-allowlist-confirm");
    let mut srv = spawn(&repo);
    let rejected = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({
            "name": "mem_write",
            "arguments": {
                "key": "reuse-allow/a->b",
                "body": "intentional",
                "kind": "decision"
            }
        }),
    );
    assert_eq!(rejected["result"]["isError"], true, "{rejected}");
    let accepted = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({
            "name": "mem_write",
            "arguments": {
                "key": "reuse-allow/a->b",
                "body": "intentional",
                "kind": "decision",
                "confirm": true
            }
        }),
    );
    assert_eq!(accepted["result"]["isError"], false, "{accepted}");
    let _ = fs::remove_dir_all(&repo);
}

#[cfg(unix)]
#[test]
fn mcp_reuse_judge_note_is_merged() {
    use std::os::unix::fs::PermissionsExt;

    let repo = super::common::mem_fixture("mcp-reuse-judge");
    fs::write(
        repo.join("gray_a.py"),
        "def normalize(value):\n    return value\n\ndef parse_token_value_data_extra(value):\n    return normalize(value)\n",
    )
    .unwrap();
    fs::write(
        repo.join("gray_b.py"),
        "def normalize(value):\n    return value\n\ndef parse_token_value(value):\n    return normalize(value)\n",
    )
    .unwrap();
    let build = eg_cmd(&repo).arg("build").output().unwrap();
    assert_eq!(build.status.code(), Some(0));
    let bin = repo.join("judge.sh");
    fs::write(
        &bin,
        "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"decision\":\"distinct\",\"probability\":0.2}'\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&bin).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&bin, perms).unwrap();
    let model = repo.join("model");
    fs::create_dir_all(&model).unwrap();
    fs::write(model.join("lazy.json"), "{\"head_layers\":1}\n").unwrap();

    let mut srv = spawn_with_env(
        &repo,
        true,
        &[
            ("EG_JUDGE_BIN", bin.as_path()),
            ("EG_JUDGE_MODEL", model.as_path()),
        ],
    );
    let judged = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "reuse", "arguments": {"paths": ["gray_b.py"], "judge": true}}),
    );
    assert_eq!(judged["result"]["isError"], false, "{judged}");
    let text = result_text(&judged);
    assert!(text.contains("eg: judge: reviewed 1, rejected 1"), "{text}");
    assert!(text.contains("0 duplication candidate(s) found"), "{text}");
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn mcp_reuse_phase4_coverage_parity() {
    let repo = fixture("mcp-reuse-phase4");
    let build = eg_cmd(&repo).arg("build").output().unwrap();
    assert_eq!(build.status.code(), Some(0));

    let cli = eg_cmd(&repo)
        .args(["reuse", "--coverage", "class"])
        .output()
        .unwrap();
    assert_eq!(cli.status.code(), Some(0));

    let mut srv = spawn(&repo);
    let mcp = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "reuse", "arguments": {"coverage": ["class"]}}),
    );
    assert_eq!(mcp["result"]["isError"], false, "{mcp}");
    assert_eq!(
        strip_footer(&result_text(&mcp)),
        String::from_utf8_lossy(&cli.stdout).into_owned()
    );
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn mcp_reuse_phase2_proposal_parity() {
    let repo = fixture("mcp-reuse-phase2");
    let build = eg_cmd(&repo).arg("build").output().unwrap();
    assert_eq!(build.status.code(), Some(0));

    let cli = eg_cmd(&repo)
        .args(["reuse", "--propose", "def normalize(text)"])
        .output()
        .unwrap();
    assert_eq!(cli.status.code(), Some(0));

    let mut srv = spawn(&repo);
    let mcp = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "reuse", "arguments": {"propose": "def normalize(text)"}}),
    );
    assert_eq!(mcp["result"]["isError"], false, "{mcp}");
    assert_eq!(
        strip_footer(&result_text(&mcp)),
        String::from_utf8_lossy(&cli.stdout).into_owned()
    );
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn mcp_reuse_phase1_parity() {
    let repo = fixture("mcp-reuse-phase1");
    let build = eg_cmd(&repo).arg("build").output().unwrap();
    assert_eq!(build.status.code(), Some(0));

    let cli_ok = eg_cmd(&repo)
        .args(["reuse", "utils.py", "--explain"])
        .output()
        .unwrap();
    assert_eq!(cli_ok.status.code(), Some(0));

    let mut srv = spawn(&repo);
    let ok = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "reuse", "arguments": {"paths": ["utils.py"], "explain": true}}),
    );
    assert_eq!(ok["result"]["isError"], false, "{ok}");
    assert_eq!(
        strip_footer(&result_text(&ok)),
        String::from_utf8_lossy(&cli_ok.stdout).into_owned()
    );

    let cli_err = eg_cmd(&repo)
        .args(["reuse", "missing.py"])
        .output()
        .unwrap();
    assert_eq!(cli_err.status.code(), Some(1));
    let err = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "reuse", "arguments": {"paths": ["missing.py"]}}),
    );
    assert_eq!(err["result"]["isError"], true, "{err}");
    assert_eq!(
        strip_footer(&result_text(&err)),
        String::from_utf8_lossy(&cli_err.stderr).into_owned()
    );
    let _ = fs::remove_dir_all(&repo);
}
