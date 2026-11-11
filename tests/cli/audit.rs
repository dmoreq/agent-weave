//! `eg audit` CLI contract: read-only, redacted, JSON-stable, and explicit
//! about missing data.

use super::common::*;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn line(repo: &Path, ts: i64, tool: &str, exit: i64, results: i64) -> String {
    serde_json::json!({
        "ts": ts,
        "tool": tool,
        "exit_code": exit,
        "result_count": results,
        "shown": results,
        "repo": repo.to_string_lossy(),
        "repo_name": repo.file_name().unwrap().to_string_lossy(),
        "agent": "test",
    })
    .to_string()
}

fn seed(repo: &Path, lines: &[String]) {
    let path = repo.join(".engram/usage.log");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("{}\n", lines.join("\n"))).unwrap();
}

fn audit(repo: &Path, args: &[&str]) -> (i32, String, String) {
    let out = eg(repo).args(["audit"]).args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn ready_repo(name: &str) -> TempRepo {
    let repo = fixture(name);
    let (code, _out, err) = build(&repo);
    assert_eq!(code, 0, "{err}");
    let _ = fs::remove_file(repo.join(".engram/usage.log"));
    repo
}

#[test]
fn audit_json_is_redacted_and_does_not_journal() {
    let repo = ready_repo("audit-json");
    let ts = now() - 60;
    let mut lines: Vec<String> = (0..20).map(|_| line(&repo, ts, "callers", 0, 3)).collect();
    lines.extend((0..10).map(|_| line(&repo, ts, "search", 0, 2)));
    seed(&repo, &lines);
    let before = fs::read(repo.join(".engram/usage.log")).unwrap();

    let (code, stdout, stderr) = audit(&repo, &["--json"]);
    assert_eq!(code, 0, "{stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["standards_version"], 1);
    assert!(doc["status"].is_string());
    assert!(doc["data_status"].is_string());
    assert_eq!(doc["checks"].as_array().unwrap().len(), 15);
    let path = repo.to_string_lossy();
    assert!(!stdout.contains(path.as_ref()), "path leaked: {stdout}");
    assert!(doc["repos"][0]["alias"]
        .as_str()
        .unwrap()
        .starts_with("repo-"));
    assert_eq!(fs::read(repo.join(".engram/usage.log")).unwrap(), before);
}

#[test]
fn audit_no_data_emits_json_and_exits_one() {
    let repo = ready_repo("audit-empty");
    let (code, stdout, _stderr) = audit(&repo, &["--json"]);
    assert_eq!(code, 1);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["data_status"], "insufficient");
    assert_eq!(doc["status"], "pass");
}

#[test]
fn audit_rejects_bad_flags_and_scope_conflicts() {
    let repo = ready_repo("audit-flags");
    let (code, _out, err) = audit(&repo, &["--days", "0"]);
    assert_eq!(code, 2, "{err}");
    let (code, _out, err) = audit(&repo, &["--baseline-days", "2"]);
    assert_eq!(code, 2, "{err}");
    let (code, _out, err) = audit(&repo, &["--json", "--reveal-paths"]);
    assert_eq!(code, 2, "{err}");
    let (code, _out, err) = audit(&repo, &["--global"]);
    assert_eq!(code, 2, "{err}");
}

#[test]
fn audit_reports_a_corrupt_index_as_critical() {
    let repo = ready_repo("audit-corrupt");
    seed(&repo, &[line(&repo, now() - 60, "callers", 0, 1)]);
    fs::write(repo.join(".engram/index.db"), b"not a sqlite database").unwrap();
    let (code, stdout, _stderr) = audit(&repo, &["--json"]);
    assert_eq!(code, 0);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["status"], "critical");
    let checks = doc["checks"].as_array().unwrap();
    let index = checks
        .iter()
        .find(|c| c["id"] == "index.integrity")
        .unwrap();
    assert_eq!(index["status"], "critical");
}

#[test]
fn audit_does_not_prune_dead_repository_lines() {
    let repo = ready_repo("audit-noprune");
    let ts = now() - 60;
    let mut lines = vec![line(&repo, ts, "callers", 0, 1)];
    lines.push(
        serde_json::json!({
            "ts": ts, "tool": "callers", "exit_code": 0, "result_count": 1,
            "repo": "/definitely-gone-eg-audit", "repo_name": "gone"
        })
        .to_string(),
    );
    seed(&repo, &lines);
    let before = fs::read(repo.join(".engram/usage.log")).unwrap();
    let (code, _stdout, _stderr) = audit(&repo, &["--json"]);
    assert_eq!(code, 0);
    assert_eq!(fs::read(repo.join(".engram/usage.log")).unwrap(), before);
}

#[test]
fn audit_global_reads_the_global_log_only() {
    let repo = ready_repo("audit-global");
    let other = ready_repo("audit-global-other");
    let global = repo.join(".test_global");
    fs::create_dir_all(&global).unwrap();
    let ts = now() - 60;
    let lines = [
        line(&repo, ts, "callers", 0, 2),
        line(&other, ts, "search", 0, 2),
    ];
    fs::write(global.join("usage.log"), format!("{}\n", lines.join("\n"))).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_eg"))
        .args(["audit", "--global", "--json"])
        .env("ENGRAM_GLOBAL_DIR", repo.join(".test_global"))
        .current_dir(&repo)
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(code, 0, "{stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["mode"], "global");
    assert_eq!(doc["coverage"]["window_entries"], 2);
    assert_eq!(doc["repos"].as_array().unwrap().len(), 2);
}
