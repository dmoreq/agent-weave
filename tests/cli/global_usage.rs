//! Integration tests: global_usage section.

use super::common::*;

#[test]
fn global_usage_dual_writes_and_summarizes_cross_repo() {
    let repo1 = temp_repo("global-usage-repo1");
    let repo2 = temp_repo("global-usage-repo2");
    let global_dir = temp_repo("global-usage-store");

    write(&repo1, "a.py", "def func_a(): pass\n");
    write(&repo2, "b.py", "def func_b(): pass\n");

    // Run build and search in repo1 with ENGRAM_GLOBAL_DIR
    eg(&repo1)
        .env("ENGRAM_GLOBAL_DIR", &global_dir)
        .env_remove("ENGRAM_NO_JOURNAL")
        .args(["build"])
        .output()
        .unwrap();
    eg(&repo1)
        .env("ENGRAM_GLOBAL_DIR", &global_dir)
        .env_remove("ENGRAM_NO_JOURNAL")
        .args(["search", "func_a"])
        .output()
        .unwrap();

    // Run build in repo2 with ENGRAM_GLOBAL_DIR
    eg(&repo2)
        .env("ENGRAM_GLOBAL_DIR", &global_dir)
        .env_remove("ENGRAM_NO_JOURNAL")
        .args(["build"])
        .output()
        .unwrap();

    // 1. Verify local journals exist and are isolated
    assert!(repo1.join(".engram/usage.log").exists());
    assert!(repo2.join(".engram/usage.log").exists());
    let log1 = fs::read_to_string(repo1.join(".engram/usage.log")).unwrap();
    assert_eq!(log1.lines().count(), 2);
    let log2 = fs::read_to_string(repo2.join(".engram/usage.log")).unwrap();
    assert_eq!(log2.lines().count(), 1);

    // 2. Verify global journal in global_dir has all 3 entries with repo attribution
    let global_log_path = global_dir.join("usage.log");
    assert!(global_log_path.exists());
    let g_log = fs::read_to_string(&global_log_path).unwrap();
    assert_eq!(g_log.lines().count(), 3);
    assert!(g_log.contains("\"repo_name\":"));
    assert!(g_log.contains("\"worktree\":false"));

    // 3. Local usage in repo1 only shows repo1
    let out1 = eg(&repo1)
        .env("ENGRAM_GLOBAL_DIR", &global_dir)
        .args(["usage"])
        .output()
        .unwrap();
    assert_eq!(out1.status.code(), Some(0));
    let stdout1 = String::from_utf8_lossy(&out1.stdout);
    assert!(stdout1.contains("eg usage: 2 journaled calls"));

    // 4. Global usage aggregates across both repositories
    let out_g = eg(&repo1)
        .env("ENGRAM_GLOBAL_DIR", &global_dir)
        .args(["usage", "--global"])
        .output()
        .unwrap();
    assert_eq!(out_g.status.code(), Some(0));
    let stdout_g = String::from_utf8_lossy(&out_g.stdout);
    assert!(stdout_g.contains("eg global usage: 3 journaled calls across 2 repositories"));
    assert!(stdout_g.contains("Active Repositories:"));
    assert!(stdout_g.contains("Recent Errors: None (100% clean)"));

    // 5. Global usage works from an empty external directory without git
    let empty_dir = temp_repo("global-usage-empty-non-git");
    let out_ext = std::process::Command::new(env!("CARGO_BIN_EXE_eg"))
        .current_dir(&empty_dir)
        .env("ENGRAM_GLOBAL_DIR", &global_dir)
        .args(["usage", "--global"])
        .output()
        .unwrap();
    assert_eq!(out_ext.status.code(), Some(0));
    let stdout_ext = String::from_utf8_lossy(&out_ext.stdout);
    assert!(stdout_ext.contains("eg global usage: 3 journaled calls across 2 repositories"));
}

#[test]
fn global_usage_drops_dead_and_identityless_lines() {
    let live_repo = temp_repo("global-usage-live");
    let global_dir = temp_repo("global-usage-dead-store");
    write(&live_repo, "a.py", "def func_a(): pass\n");

    // One live entry through a real eg run.
    eg(&live_repo)
        .env("ENGRAM_GLOBAL_DIR", &global_dir)
        .env_remove("ENGRAM_NO_JOURNAL")
        .args(["build"])
        .output()
        .unwrap();

    // Append a deleted repository and an identityless legacy line to the
    // global journal.
    let global_log = global_dir.join("usage.log");
    let mut content = fs::read_to_string(&global_log).unwrap();
    content.push_str(
        r#"{"tool":"callers","exit_code":0,"repo":"/nonexistent-eg-global/deleted","repo_name":"agent-skills"}
{"tool":"callers","exit_code":0,"repo":".tmp/verify/repo1","repo_name":"repo1"}
"#,
    );
    fs::write(&global_log, content).unwrap();

    let out = eg(&live_repo)
        .env("ENGRAM_GLOBAL_DIR", &global_dir)
        .args(["usage", "--global"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Everything without a live repository is dropped from the totals: only
    // the live build survives, and no unattributed bucket is printed.
    assert!(
        stdout.contains("eg global usage: 1 journaled calls across 1 repositories"),
        "{stdout}"
    );
    assert!(!stdout.contains("unattributed"), "{stdout}");
    assert!(stdout.contains("Active Repositories:"), "{stdout}");
    assert!(stdout.contains("global-usage-live"), "{stdout}");
    assert!(!stdout.contains("agent-skills"), "{stdout}");
    assert!(!stdout.contains("repo1"), "{stdout}");
}

#[test]
fn mem_query_finds_note_via_refs_when_terms_absent_from_body() {
    let repo = temp_repo("mem-refs-query");
    let (code, stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "resolver-gotcha",
            "--body",
            "ambiguous edges keep target null",
            "--kind",
            "gotcha",
            "--refs",
            "src/extract/resolve.rs",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");

    // Query for 'resolve' which is ONLY present in --refs (not key, tags, or body)
    let (code, stdout, stderr) = mem_out(&repo, &["query", "resolve"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("resolver-gotcha"),
        "must find note via refs: {stdout}"
    );
}

#[test]
fn mem_stale_ref_flagged_when_anchor_modified_or_missing() {
    let repo = temp_repo("mem-stale-ref");
    write(&repo, "app.py", "def run():\n    return 42\n");
    build(&repo);

    let (code, stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "app-note",
            "--body",
            "settled architecture for run function",
            "--kind",
            "decision",
            "--refs",
            "app.py",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");

    // Right after build: app.py mtime <= note created_at, ref is valid
    let (code, stdout, stderr) = mem_out(&repo, &["query", "architecture"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("app-note"));
    assert!(
        !stdout.contains("[stale-ref]"),
        "must not be stale initially: {stdout}"
    );

    // Now sleep 1.1s and modify app.py, re-index so files.mtime > created_at
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write(&repo, "app.py", "def run():\n    return 43\n");
    build(&repo);

    // Stale check in mem query
    let (code, stdout, stderr) = mem_out(&repo, &["query", "architecture"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("app-note"));
    assert!(
        stdout.contains("[stale-ref]"),
        "must flag [stale-ref] after file modification: {stdout}"
    );

    // Stale check in mem get
    let (code, stdout, stderr) = mem_out(&repo, &["get", "app-note"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("[stale-ref]"),
        "mem get must also flag [stale-ref]: {stdout}"
    );
}

#[test]
fn reuse_detects_intra_file_duplication() {
    let repo = temp_repo("reuse-intra-file");
    write(
        &repo,
        "helpers.py",
        "def parse_token(token: str):\n    return token.strip()\n\ndef parse_token_v2(token: str):\n    return token.strip()\n",
    );
    build(&repo);

    let out = eg(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("duplication candidate(s) found"),
        "must detect intra-file duplicate: {stdout}"
    );
    assert!(stdout.contains("parse_token"), "{stdout}");
}
