//! Integration tests: global_usage section.

use super::common::*;

#[test]
fn global_usage_dual_writes_and_summarizes_cross_repo() {
    let repo1 = temp_repo("global-usage-repo1");
    let repo2 = temp_repo("global-usage-repo2");
    let global_dir = temp_repo("global-usage-store");

    write(&repo1, "a.py", "def func_a(): pass\n");
    write(&repo2, "b.py", "def func_b(): pass\n");

    // Run build and search in repo1 with AW_GLOBAL_DIR
    aw(&repo1)
        .env("AW_GLOBAL_DIR", &global_dir)
        .env_remove("AW_NO_JOURNAL")
        .args(["build"])
        .output()
        .unwrap();
    aw(&repo1)
        .env("AW_GLOBAL_DIR", &global_dir)
        .env_remove("AW_NO_JOURNAL")
        .args(["search", "func_a"])
        .output()
        .unwrap();

    // Run build in repo2 with AW_GLOBAL_DIR
    aw(&repo2)
        .env("AW_GLOBAL_DIR", &global_dir)
        .env_remove("AW_NO_JOURNAL")
        .args(["build"])
        .output()
        .unwrap();

    // 1. Verify local journals exist and are isolated
    assert!(repo1.join(".aw/usage.log").exists());
    assert!(repo2.join(".aw/usage.log").exists());
    let log1 = fs::read_to_string(repo1.join(".aw/usage.log")).unwrap();
    assert_eq!(log1.lines().count(), 2);
    let log2 = fs::read_to_string(repo2.join(".aw/usage.log")).unwrap();
    assert_eq!(log2.lines().count(), 1);

    // 2. Verify global journal in global_dir has all 3 entries with repo attribution
    let global_log_path = global_dir.join("usage.log");
    assert!(global_log_path.exists());
    let g_log = fs::read_to_string(&global_log_path).unwrap();
    assert_eq!(g_log.lines().count(), 3);
    assert!(g_log.contains("\"repo_name\":"));
    assert!(g_log.contains("\"worktree\":false"));

    // 3. Local usage in repo1 only shows repo1
    let out1 = aw(&repo1)
        .env("AW_GLOBAL_DIR", &global_dir)
        .args(["usage"])
        .output()
        .unwrap();
    assert_eq!(out1.status.code(), Some(0));
    let stdout1 = String::from_utf8_lossy(&out1.stdout);
    assert!(stdout1.contains("aw usage: 2 journaled calls"));

    // 4. Global usage aggregates across both repositories
    let out_g = aw(&repo1)
        .env("AW_GLOBAL_DIR", &global_dir)
        .args(["usage", "--global"])
        .output()
        .unwrap();
    assert_eq!(out_g.status.code(), Some(0));
    let stdout_g = String::from_utf8_lossy(&out_g.stdout);
    assert!(stdout_g.contains("aw global usage: 3 journaled calls across 2 repositories"));
    assert!(stdout_g.contains("Active Repositories:"));
    assert!(stdout_g.contains("Recent Errors: None (100% clean)"));

    // 5. Global usage works from an empty external directory without git
    let empty_dir = temp_repo("global-usage-empty-non-git");
    let out_ext = std::process::Command::new(env!("CARGO_BIN_EXE_aw"))
        .current_dir(&empty_dir)
        .env("AW_GLOBAL_DIR", &global_dir)
        .args(["usage", "--global"])
        .output()
        .unwrap();
    assert_eq!(out_ext.status.code(), Some(0));
    let stdout_ext = String::from_utf8_lossy(&out_ext.stdout);
    assert!(stdout_ext.contains("aw global usage: 3 journaled calls across 2 repositories"));
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

    let out = aw(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("duplication candidate(s) found"),
        "must detect intra-file duplicate: {stdout}"
    );
    assert!(stdout.contains("parse_token"), "{stdout}");
}
