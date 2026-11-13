//! Integration tests: reuse section.

use super::common::*;

#[test]
fn reuse_empty_reports_zero_candidates() {
    let repo = temp_repo("reuse-empty");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    let out = eg(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("eg: reuse — 0 duplication candidate(s) found"),
        "{stdout}"
    );
}

#[test]
fn reuse_detects_candidate_duplicates() {
    let repo = temp_repo("reuse-dupes");
    write(
        &repo,
        "a.py",
        "def parse_token(token: str):\n    return token.strip()\n",
    );
    write(
        &repo,
        "b.py",
        "def parse_token(token: str):\n    return token.strip()\n",
    );
    build(&repo);
    let out = eg(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("duplication candidate(s) found"),
        "{stdout}"
    );
    assert!(stdout.contains("parse_token"), "{stdout}");
    // Verify telemetry recorded the duplication warning
    let usage = eg(&repo).args(["usage", "--quality"]).output().unwrap();
    let usage_stdout = String::from_utf8_lossy(&usage.stdout);
    assert!(
        usage_stdout.contains("Anti-Reinvention: 1 duplication warnings caught"),
        "{usage_stdout}"
    );
}

#[test]
fn search_outputs_interface_cards_and_callers_ranking() {
    let repo = temp_repo("search-cards");
    write(
        &repo,
        "util.py",
        "def popular_helper(x: int) -> int:\n    return x + 1\n\ndef lone_helper(x: int) -> int:\n    return x + 2\n",
    );
    write(
        &repo,
        "main.py",
        "from util import popular_helper\ndef run():\n    return popular_helper(1) + popular_helper(2)\n",
    );
    build(&repo);
    let out = eg(&repo).args(["search", "helper"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains(
            "util.py:1 function util.popular_helper def popular_helper(x: int) -> int [callers=1]"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "util.py:4 function util.lone_helper def lone_helper(x: int) -> int [callers=0]"
        ),
        "{stdout}"
    );
    let pop_idx = stdout.find("util.popular_helper").unwrap();
    let lone_idx = stdout.find("util.lone_helper").unwrap();
    assert!(
        pop_idx < lone_idx,
        "popular helper must rank ahead: {stdout}"
    );
}

#[test]
fn incremental_build_emits_reuse_hint_on_stderr() {
    let repo = temp_repo("build-reuse-hint");
    write(
        &repo,
        "core.py",
        "def transform_data(data: str):\n    return data.strip().lower()\n",
    );
    build(&repo);

    write(
        &repo,
        "extra.py",
        "def transform_data(data: str):\n    return data.strip().lower()\n",
    );
    let (code, _stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stderr.contains("eg: reuse-hint:"),
        "expected reuse hint on stderr: {stderr}"
    );
    assert!(stderr.contains("transform_data"), "{stderr}");
}

#[test]
fn reuse_scoped_path_filters_targets_and_no_duplicate_pairs() {
    let repo = temp_repo("reuse-scope-dedup");
    write(
        &repo,
        "a.py",
        "def parse_token(token: str):\n    return token.strip()\n",
    );
    write(
        &repo,
        "b.py",
        "def parse_token(token: str):\n    return token.strip()\n",
    );
    build(&repo);

    // Unscoped: exactly 1 candidate found (not duplicated as A->B and B->A)
    let out = eg(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("eg: reuse — 1 duplication candidate(s) found"),
        "must report exactly 1 candidate pair: {stdout}"
    );

    // Scoped to duplicate file b.py: reports candidate
    let out_b = eg(&repo).args(["reuse", "b.py"]).output().unwrap();
    let stdout_b = String::from_utf8_lossy(&out_b.stdout);
    assert_eq!(out_b.status.code(), Some(0), "{stdout_b}");
    assert!(
        stdout_b.contains("eg: reuse — 1 duplication candidate(s) found"),
        "must report b.py duplicate: {stdout_b}"
    );

    // Scoped to canonical file a.py: zero candidates (a.py is not duplicating anything)
    let out_a = eg(&repo).args(["reuse", "a.py"]).output().unwrap();
    let stdout_a = String::from_utf8_lossy(&out_a.stdout);
    assert_eq!(out_a.status.code(), Some(0), "{stdout_a}");
    assert!(
        stdout_a.contains("eg: reuse — 0 duplication candidate(s) found"),
        "canonical a.py must not be reported as a duplicate: {stdout_a}"
    );
}

#[test]
fn reuse_d1_callee_footprint_requires_resolved_callees() {
    let repo = temp_repo("reuse-d1");
    write(
        &repo,
        "core.py",
        "def helper1():\n    return 1\ndef helper2():\n    return 2\ndef helper3():\n    return 3\ndef canonical_pipeline():\n    return helper1() + helper2() + helper3()\n",
    );
    write(
        &repo,
        "main.py",
        "from core import canonical_pipeline\ndef run():\n    return canonical_pipeline() + canonical_pipeline()\n",
    );
    write(
        &repo,
        "dup.py",
        "from core import helper1, helper2, helper3\ndef duplicate_pipeline():\n    return helper1() + helper2() + helper3()\n",
    );
    build(&repo);

    let out = eg(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("duplicate_pipeline"),
        "must identify duplicate_pipeline: {stdout}"
    );
    assert!(
        stdout.contains("callee footprint"),
        "must be detected via D1 callee footprint: {stdout}"
    );
    assert!(
        stdout.contains("shared callees: [helper1, helper2, helper3]"),
        "must list shared resolved callees: {stdout}"
    );
}

fn mem_out_global(repo: &Path, global_dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = eg(repo)
        .env("ENGRAM_GLOBAL_DIR", global_dir)
        .args(["mem"])
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn global_memory_write_and_local_layered_query_with_global_tag() {
    let repo = temp_repo("mem-global-layered");
    let global_dir = temp_repo("global-eg-store");

    // 1. Write a global note
    let (code, stdout, stderr) = mem_out_global(
        &repo,
        &global_dir,
        &[
            "write",
            "developer_profile",
            "--body",
            "Preferred stack: Rust. Style: Minimal.",
            "--kind",
            "decision",
            "--global",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("ok: 'developer_profile' rev 1 written"));

    // Verify .engram/memory.db was NOT created in repo root, but memory.db WAS created in global_dir
    assert!(!repo.join(".engram/memory.db").exists());
    assert!(global_dir.join("memory.db").exists());

    // 2. Query from inside repo without flags: layered search finds global note and tags it [global]
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["query", "stack"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("developer_profile r1 decision durable "));
    assert!(stdout.contains(" [global]"));
    assert!(stdout.contains("Preferred stack: Rust. Style: Minimal."));

    // 3. Query --all also lists the global note with [global] tag
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["query", "--all"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("developer_profile r1 decision durable "));
    assert!(stdout.contains(" [global]"));

    // 4. Get from repo without flags falls back to global and tags with [global]
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["get", "developer_profile"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("developer_profile r1 decision durable "));
    assert!(stdout.contains(" [global]"));
    assert!(stdout.contains("revs: 1"));

    // 5. Write a local note with the SAME key
    let (code, stdout, stderr) = mem_out_global(
        &repo,
        &global_dir,
        &[
            "write",
            "developer_profile",
            "--body",
            "Repo override: Python stack.",
            "--kind",
            "decision",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(repo.join(".engram/memory.db").exists());

    // 6. Query from repo without flags: local shadows global!
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["query", "--all"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("Repo override: Python stack."));
    assert!(!stdout.contains("[global]"));
    assert!(!stdout.contains("Preferred stack: Rust."));

    // 7. Ranked query also shows local note shadowing global note
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["query", "developer"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("Repo override: Python stack."));
    assert!(!stdout.contains("[global]"));

    // 8. Get from repo returns the local note
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["get", "developer_profile"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("Repo override: Python stack."));
    assert!(!stdout.contains("[global]"));

    // 9. Query with --global explicitly targets global store, returning the global note
    let (code, stdout, stderr) =
        mem_out_global(&repo, &global_dir, &["query", "--all", "--global"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("Preferred stack: Rust. Style: Minimal."));
    assert!(stdout.contains(" [global]"));
    assert!(!stdout.contains("Repo override"));

    // 10. Delete --global deletes from global store
    let (code, stdout, stderr) = mem_out_global(
        &repo,
        &global_dir,
        &["delete", "developer_profile", "--global"],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("deleted 'developer_profile' revs=1"));

    // Global query now returns empty/no-results
    let (code, _, _) = mem_out_global(&repo, &global_dir, &["query", "--all", "--global"]);
    assert_eq!(code, 1);

    // Local note is untouched!
    let (code, stdout, _) = mem_out_global(&repo, &global_dir, &["get", "developer_profile"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("Repo override: Python stack."));
}

#[test]
fn global_memory_export_and_gc() {
    let repo = temp_repo("mem-global-export-gc");
    let global_dir = temp_repo("global-eg-store-export");

    let (code, stdout, stderr) = mem_out_global(
        &repo,
        &global_dir,
        &[
            "write",
            "global_item",
            "--body",
            "item body for export",
            "--kind",
            "fact",
            "--global",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");

    // Export with --global
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["export", "--global"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("\"key\":\"global_item\""), "{stdout}");
    assert!(
        stdout.contains("\"body\":\"item body for export\""),
        "{stdout}"
    );

    // GC with --global
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["gc", "--global"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("expired=0 removed, superseded=0 removed, durable=1 rows"),
        "{stdout}"
    );
}

#[test]
fn global_memory_read_never_creates_missing_directory() {
    let repo = temp_repo("mem-no-create-read");
    let non_existent_global = repo.join("definitely-does-not-exist-global-dir");
    assert!(!non_existent_global.exists());

    // A read-only query must exit 1 and never create the directory on disk
    let (code, _, _) = mem_out_global(&repo, &non_existent_global, &["query", "test"]);
    assert_eq!(code, 1);
    assert!(
        !non_existent_global.exists(),
        "query must not create missing global directory"
    );

    let (code, _, _) = mem_out_global(&repo, &non_existent_global, &["get", "test"]);
    assert_eq!(code, 1);
    assert!(
        !non_existent_global.exists(),
        "get must not create missing global directory"
    );
}

#[test]
fn global_memory_expired_local_does_not_shadow_global() {
    let repo = temp_repo("mem-expired-shadow");
    let global_dir = temp_repo("global-eg-expired-shadow");

    // Global note
    let (code, _, _) = mem_out_global(
        &repo,
        &global_dir,
        &[
            "write",
            "active_topic",
            "--body",
            "durable global knowledge",
            "--kind",
            "decision",
            "--global",
        ],
    );
    assert_eq!(code, 0);

    // Local note with short TTL (1 second)
    let (code, _, _) = mem_out_global(
        &repo,
        &global_dir,
        &[
            "write",
            "active_topic",
            "--body",
            "transient local note",
            "--kind",
            "decision",
            "--scope",
            "session",
            "--ttl",
            "1",
        ],
    );
    assert_eq!(code, 0);

    // Sleep until local note is expired
    std::thread::sleep(std::time::Duration::from_millis(1500));

    // Query must return the global note tagged [global], not be shadowed by dead local session note
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["query", "knowledge"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("active_topic r1 decision durable "));
    assert!(stdout.contains(" [global]"));
    assert!(stdout.contains("durable global knowledge"));

    // Get must also return the active global note
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["get", "active_topic"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("active_topic r1 decision durable "));
    assert!(stdout.contains(" [global]"));
    assert!(stdout.contains("durable global knowledge"));
}

#[test]
fn reuse_d3_raw_client_bypass_detects_matching_canonical_wrapper() {
    let repo = temp_repo("reuse-d3");
    write(
        &repo,
        "client.py",
        "import requests\ndef fetch_api(url: str):\n    return requests.get(url)\n",
    );
    write(
        &repo,
        "app.py",
        "from client import fetch_api\ndef run_a():\n    return fetch_api('http://a')\ndef run_b():\n    return fetch_api('http://b')\n",
    );
    write(
        &repo,
        "bypass.py",
        "import requests\ndef rogue_fetch(endpoint: str):\n    return requests.get(endpoint)\n",
    );
    write(
        &repo,
        "other.py",
        "import requests\ndef post_data(endpoint: str):\n    return requests.post(endpoint)\n",
    );
    build(&repo);

    let out = eg(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("rogue_fetch"),
        "must identify rogue_fetch: {stdout}"
    );
    assert!(
        stdout.contains("bypasses canonical client 'client.fetch_api'"),
        "must detect D3 raw client bypass: {stdout}"
    );
    assert!(
        stdout.contains("raw external client requests.get"),
        "must display raw external client requests.get: {stdout}"
    );
    assert!(
        !stdout.contains("post_data"),
        "post_data calling requests.post must not match requests.get wrapper: {stdout}"
    );
}

mod journal_ids;
