//! Integration tests: memory_basics section.

use super::common::*;

// ---------------------------------------------------------------------------
// Project memory (companion design: 2026-11-05_agent-weave-memory-retrieval).
// Phase 1 acceptance: byte-exact shapes, exit contracts, idempotent replay,
// revision supersession, version/corruption gates, doctor disposition,
// concurrent writers, export round-trip, journal privacy, recall smoke.
// ---------------------------------------------------------------------------

#[test]
fn mem_write_query_get_shapes_and_idempotent_replay() {
    let repo = temp_repo("mem-shapes");
    let (code, stdout, stderr) = mem_write(
        &repo,
        "resolver-tiers",
        "the resolver keeps confidence tiers; AMBIGUOUS edges keep dst_id NULL",
        "decision",
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.starts_with(
            "aw: memory — agent-authored notes, candidates only, never proof of current behavior\n"
        ),
        "caveat must be the stdout first line: {stdout:?}"
    );
    assert_eq!(
        stdout.lines().nth(1),
        Some("ok: 'resolver-tiers' rev 1 written"),
        "{stdout:?}"
    );
    // Idempotent replay: identical content is a successful no-op naming the rev.
    let (code, stdout, _stderr) = mem_write(
        &repo,
        "resolver-tiers",
        "the resolver keeps confidence tiers; AMBIGUOUS edges keep dst_id NULL",
        "decision",
    );
    assert_eq!(code, 0);
    assert_eq!(
        stdout.lines().nth(1),
        Some("ok: 'resolver-tiers' unchanged at rev 1 (identical content)"),
        "{stdout:?}"
    );
    // Ranked query hit.
    let (code, stdout, stderr) = mem_out(&repo, &["query", "confidence", "tiers"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines[0],
        "aw: memory — agent-authored notes, candidates only, never proof of current behavior"
    );
    assert!(
        lines[1].starts_with("resolver-tiers r1 decision durable "),
        "row header is `key rev kind scope created_at`: {:?}",
        lines[1]
    );
    assert_eq!(
        lines[2],
        "  the resolver keeps confidence tiers; AMBIGUOUS edges keep dst_id NULL"
    );
    // Exact-key get with revision count.
    let (code, stdout, _stderr) = mem_out(&repo, &["get", "resolver-tiers"]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("resolver-tiers r1 decision durable "),
        "{stdout}"
    );
    assert!(stdout.ends_with("revs: 1\n"), "{stdout:?}");
    // Unknown key: exit 1, caveat on stdout, diagnostic on stderr.
    let (code, stdout, stderr) = mem_out(&repo, &["get", "missing-key"]);
    assert_eq!(code, 1);
    assert!(stdout.starts_with("aw: memory — "), "{stdout:?}");
    assert_eq!(
        stderr.trim(),
        "aw: no-results: unknown memory key 'missing-key'",
        "{stderr}"
    );
}

#[test]
fn mem_query_cap_showing_line_and_stderr_notice() {
    let repo = temp_repo("mem-cap");
    for i in 0..15 {
        let (code, stdout, stderr) = mem_write(
            &repo,
            &format!("cap-note-{i:02}"),
            &format!("cap filler note number {i} shares the token filler"),
            "fact",
        );
        assert_eq!(code, 0, "{stdout}{stderr}");
    }
    let (code, stdout, stderr) = mem_out(&repo, &["query", "filler"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    // Caveat + 10 rows + `showing 10 of 15`.
    assert_eq!(
        lines[0],
        "aw: memory — agent-authored notes, candidates only, never proof of current behavior"
    );
    assert_eq!(lines.last(), Some(&"showing 10 of 15"), "{stdout:?}");
    assert_eq!(
        stderr.trim(),
        "[aw] capped at 10 results (15 total)",
        "{stderr}"
    );
    // --limit raises the cap deterministically.
    let (code, stdout, stderr) = mem_out(&repo, &["query", "filler", "--limit", "12"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.ends_with("showing 12 of 15\n"), "{stdout:?}");
    assert_eq!(
        stderr.trim(),
        "[aw] capped at 12 results (15 total)",
        "{stderr}"
    );
    // --all lists by recency with the same cap contract.
    let (code, stdout, _stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 0);
    assert!(stdout.ends_with("showing 10 of 15\n"), "{stdout:?}");
}

#[test]
fn mem_empty_and_missing_store_are_exit1() {
    let repo = temp_repo("mem-empty");
    // No .aw/memory.db at all: the read contract exits 1 with the no-memory class.
    let (code, stdout, stderr) = mem_out(&repo, &["query", "anything"]);
    assert_eq!(code, 1);
    // House convention for a missing store: stdout stays empty, the
    // diagnostic carries the no-memory class on stderr.
    assert!(stdout.is_empty(), "{stdout:?}");
    assert_eq!(
        stderr.trim(),
        format!(
            "aw: no-memory: no .aw/memory.db under {} — write with aw mem write",
            repo.display()
        ),
        "{stderr}"
    );
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no-memory"), "{stderr}");
    // With a store present, a non-matching query is a plain empty result.
    let _ = mem_write(
        &repo,
        "solo",
        "the only note here talks about orbit values",
        "fact",
    );
    let (code, stdout, stderr) = mem_out(&repo, &["query", "zzz-unmatchable"]);
    assert_eq!(code, 1);
    assert!(stdout.starts_with("aw: memory — "), "{stdout:?}");
    assert_eq!(
        stderr.trim(),
        "aw: no-results: no memory matches 'zzz-unmatchable'",
        "{stderr}"
    );
    // Usage errors: --all with terms, whitespace-only terms.
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "--all", "terms"]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("mutually exclusive"), "{stderr}");
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "   "]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("provide terms or --all"), "{stderr}");
    // Bare query without terms defaults to --all listing.
    let (code, stdout, stderr) = mem_out(&repo, &["query"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("solo"), "{stdout}");
}

#[test]
fn mem_revision_supersession_and_delete() {
    let repo = temp_repo("mem-revs");
    let _ = mem_write(&repo, "k", "body one", "decision");
    let (code, stdout, _stderr) = mem_write(&repo, "k", "body two", "decision");
    assert_eq!(code, 0);
    assert_eq!(
        stdout.lines().nth(1),
        Some("ok: 'k' rev 2 written (supersedes rev 1)"),
        "{stdout:?}"
    );
    let (code, stdout, _stderr) = mem_out(&repo, &["get", "k"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("k r2 decision durable "), "{stdout}");
    assert!(stdout.contains("  body two"), "{stdout}");
    assert!(stdout.ends_with("revs: 2\n"), "{stdout:?}");
    // Delete removes every revision; the FTS rows go with it (query misses).
    let (code, stdout, _stderr) = mem_out(&repo, &["delete", "k"]);
    assert_eq!(code, 0);
    assert_eq!(
        stdout.lines().nth(1),
        Some("deleted 'k' revs=2"),
        "{stdout:?}"
    );
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "body"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no memory matches"), "{stderr}");
    let (code, _stdout, stderr) = mem_out(&repo, &["delete", "k"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("unknown memory key 'k'"), "{stderr}");
    // Deleting with no store at all must not conjure one.
    let empty = temp_repo("mem-delete-empty");
    let (code, _stdout, stderr) = mem_out(&empty, &["delete", "k"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no-memory"), "{stderr}");
    assert!(!empty.join(".aw/memory.db").exists());
}

/// A misspelled `--kind`/`--scope` filter on `mem query` is a usage error
/// (exit 2), not a silently empty result: both surfaces parse the taxonomy
/// at the boundary into `MemKind`/`MemScope`.
#[test]
fn mem_query_rejects_unknown_kind_and_scope_exit2() {
    let repo = temp_repo("mem-query-usage");
    for flag in ["--kind", "--scope"] {
        let (code, _stdout, stderr) = aw(&repo)
            .args(["mem", "query", "x", flag, "mood"])
            .output()
            .map(|o| {
                (
                    o.status.code().unwrap_or(-1),
                    String::new(),
                    String::from_utf8_lossy(&o.stderr).to_string(),
                )
            })
            .unwrap();
        assert_eq!(code, 2, "flag {flag}: {stderr}");
        let expected = if flag == "--kind" {
            "kind must be one of decision|gotcha|fact|task"
        } else {
            "scope must be one of durable|session"
        };
        assert!(stderr.contains(expected), "flag {flag}: {stderr}");
    }
}

#[test]
fn mem_write_usage_rejections_are_exit2() {
    let repo = temp_repo("mem-usage");
    // Unknown kind (closed taxonomy).
    let (code, _stdout, stderr) = mem_write(&repo, "k", "body", "mood");
    assert_eq!(code, 2);
    assert!(
        stderr.contains("kind must be one of decision|gotcha|fact|task"),
        "{stderr}"
    );
    // Phase 2: session scope is now accepted; unknown scope still exits 2.
    let out = aw(&repo)
        .args([
            "mem", "write", "k", "--body", "b", "--kind", "fact", "--scope", "orbit",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("scope must be 'durable' or 'session'"));
    // TTL without the session lane.
    let out = aw(&repo)
        .args([
            "mem", "write", "k", "--body", "b", "--kind", "fact", "--ttl", "60",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--ttl is only valid with --scope session"),
    );
    // Body cap.
    let big = "x".repeat(8193);
    let (code, _stdout, stderr) = mem_write(&repo, "k", &big, "fact");
    assert_eq!(code, 2);
    assert!(stderr.contains("body exceeds 8192 bytes"), "{stderr}");
    // Key cap and control characters.
    let (code, _stdout, stderr) = mem_write(&repo, &"k".repeat(201), "b", "fact");
    assert_eq!(code, 2);
    assert!(stderr.contains("key exceeds 200 bytes"), "{stderr}");
    let (code, _stdout, stderr) = mem_write(&repo, "bad\nkey", "b", "fact");
    assert_eq!(code, 2);
    assert!(stderr.contains("control characters"), "{stderr}");
    // Empty body.
    let (code, _stdout, stderr) = mem_write(&repo, "k", "   ", "fact");
    assert_eq!(code, 2);
    assert!(stderr.contains("body must not be empty"), "{stderr}");
    // Nothing was written by the rejected calls.
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no .aw/memory.db"), "{stderr}");
}

#[test]
fn mem_schema_gates_newer_incomplete_and_corrupt() {
    use rusqlite::Connection;
    // Newer than the binary -> exit 5, export-backup message.
    let repo = temp_repo("mem-newer");
    let _ = mem_write(&repo, "k", "b", "fact");
    let conn = Connection::open(repo.join(".aw/memory.db")).unwrap();
    conn.execute_batch("PRAGMA user_version = 99").unwrap();
    drop(conn);
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 5);
    assert!(
        stderr.contains("memory.db schema v99 is newer than the binary expects (v1)"),
        "{stderr}"
    );
    assert!(
        stderr.contains("restore from an aw mem export backup"),
        "{stderr}"
    );

    // Older but failing the compiled manifest -> the other exit-5 message.
    let repo = temp_repo("mem-incomplete");
    let _ = mem_write(&repo, "k", "b", "fact");
    let conn = Connection::open(repo.join(".aw/memory.db")).unwrap();
    conn.execute_batch("PRAGMA user_version = 0; DROP TABLE memories_fts;")
        .unwrap();
    drop(conn);
    let (code, _stdout, stderr) = mem_out(&repo, &["get", "k"]);
    assert_eq!(code, 5);
    assert!(
        stderr.contains("is missing required structure for this binary"),
        "{stderr}"
    );

    // Truncated / non-SQLite file -> exit 3.
    let repo = temp_repo("mem-corrupt");
    std::fs::create_dir_all(repo.join(".aw")).unwrap();
    std::fs::write(repo.join(".aw/memory.db"), b"this is not sqlite at all").unwrap();
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 3);
    assert!(stderr.contains("corrupt-memory"), "{stderr}");
}

#[test]
fn mem_works_without_index_db_and_survives_rebuilds() {
    let repo = temp_repo("mem-no-index");
    // No build ever ran: mem write + query work alone.
    let (code, stdout, stderr) =
        mem_write(&repo, "standalone", "memory outlives the index", "fact");
    assert_eq!(code, 0, "{stdout}{stderr}");
    let (code, stdout, _stderr) = mem_out(&repo, &["query", "memory outlives"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("standalone r1"), "{stdout}");
    assert!(
        !repo.join(".aw/index.db").exists(),
        "no index.db was created"
    );
    // And it still answers after a full build + invalidate cycle.
    let (code, _stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stderr}");
    let (code, _stdout, stderr) = mem_out(&repo, &["get", "standalone"]);
    assert_eq!(code, 0, "{stderr}");
}
