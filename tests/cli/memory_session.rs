//! Integration tests: memory_session section.

use super::common::*;

// ---------------------------------------------------------------------------
// Phase 2 — lifecycle: session lane + TTL, gc purge, --supersedes,
// --include-superseded, determinism under replay.
// ---------------------------------------------------------------------------

#[test]
fn mem_session_ttl_default_and_durable_ttl_rejection() {
    let repo = temp_repo("mem-session-ttl");
    // Session without --ttl: expires_at = created_at + 604800 (OQ2 default).
    let (code, _stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "session-note",
            "--body",
            "transient scratch work",
            "--kind",
            "fact",
            "--scope",
            "session",
        ],
    );
    assert_eq!(code, 0, "{stderr}");
    let (code, stdout, _stderr) = mem_out(&repo, &["export"]);
    assert_eq!(code, 0);
    let row: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(row["scope"], "session");
    let ttl = row["expires_at"].as_i64().unwrap() - row["created_at"].as_i64().unwrap();
    assert_eq!(ttl, 604800, "OQ2 default session TTL is 7 days: {row}");
    // Explicit --ttl wins.
    let _ = mem_out(
        &repo,
        &[
            "write",
            "session-short",
            "--body",
            "short lived",
            "--kind",
            "fact",
            "--scope",
            "session",
            "--ttl",
            "60",
        ],
    );
    let (code, stdout, _stderr) = mem_out(&repo, &["export"]);
    assert_eq!(code, 0);
    let rows: Vec<serde_json::Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let short = rows.iter().find(|r| r["key"] == "session-short").unwrap();
    assert_eq!(
        short["expires_at"].as_i64().unwrap() - short["created_at"].as_i64().unwrap(),
        60
    );
    // Durable + --ttl is a usage error; durable rows carry expires_at = 0.
    let (code, _stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "durable-ttl",
            "--body",
            "b",
            "--kind",
            "fact",
            "--ttl",
            "60",
        ],
    );
    assert_eq!(code, 2);
    assert!(
        stderr.contains("--ttl is only valid with --scope session"),
        "{stderr}"
    );
    let durable = rows.iter().find(|r| r["key"] == "session-note").unwrap();
    assert!(durable["scope"] == "session");
}

#[test]
fn mem_gc_cli_purges_seeded_expired_and_reports_counts() {
    use rusqlite::Connection;
    let repo = temp_repo("mem-gc-cli");
    let _ = mem_write(&repo, "keeper", "durable note", "fact");
    let _ = mem_out(
        &repo,
        &[
            "write",
            "session-note",
            "--body",
            "expired scratch",
            "--kind",
            "fact",
            "--scope",
            "session",
            "--ttl",
            "60",
        ],
    );
    // Inject an expired deadline for the session note (injected clock).
    let conn = Connection::open(repo.join(".engram/memory.db")).unwrap();
    conn.execute(
        "UPDATE memories SET expires_at = 1 WHERE key = 'session-note'",
        [],
    )
    .unwrap();
    drop(conn);
    let (code, stdout, stderr) = mem_out(&repo, &["gc"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        stdout.lines().nth(1),
        Some("expired=1 removed, superseded=0 removed, durable=1 rows"),
        "{stdout:?}"
    );
    // The expired row and its FTS entry are gone: the query misses it.
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "expired scratch"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no memory matches"), "{stderr}");
    // gc with no store at all must not conjure one.
    let bare = temp_repo("mem-gc-missing");
    let (code, _stdout, stderr) = mem_out(&bare, &["gc"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no-memory"), "{stderr}");
    assert!(!bare.join(".engram/memory.db").exists());
}

#[test]
fn mem_supersedes_chain_cli_contract() {
    let repo = temp_repo("mem-supersedes-cli");
    let _ = mem_write(
        &repo,
        "old-conclusion",
        "the old way of doing things",
        "decision",
    );
    let (code, stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "new-conclusion",
            "--body",
            "the new way, measured",
            "--kind",
            "decision",
            "--supersedes",
            "old-conclusion",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert_eq!(
        stdout.lines().nth(1),
        Some("ok: 'new-conclusion' rev 1 written"),
        "{stdout:?}"
    );
    // Default listing hides the superseded note; --include-superseded shows it tagged.
    let (code, stdout, _stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("new-conclusion") && !stdout.contains("old-conclusion"),
        "{stdout}"
    );
    let (code, stdout, _stderr) = mem_out(&repo, &["query", "--all", "--include-superseded"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("old-conclusion"), "{stdout}");
    let superseded_line = stdout
        .lines()
        .find(|l| l.starts_with("old-conclusion"))
        .unwrap();
    assert!(
        superseded_line.ends_with(" [superseded]"),
        "{superseded_line}"
    );
    // --include-superseded without --all is a usage error.
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "measured", "--include-superseded"]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains("--include-superseded requires --all"),
        "{stderr}"
    );
    // Unknown target: exit 1 and no new note.
    let (code, _stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "never-lands",
            "--body",
            "x",
            "--kind",
            "fact",
            "--supersedes",
            "missing",
        ],
    );
    assert_eq!(code, 1);
    assert!(stderr.contains("unknown memory key 'missing'"), "{stderr}");
    let (code, _stdout, _stderr) = mem_out(&repo, &["get", "never-lands"]);
    assert_eq!(code, 1, "the failed supersedes write must leave no row");
    // Same key as the new note: exit 2 (same-key updates go through revs).
    let (code, _stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "self",
            "--body",
            "x",
            "--kind",
            "fact",
            "--supersedes",
            "self",
        ],
    );
    assert_eq!(code, 2);
    assert!(stderr.contains("different key"), "{stderr}");
}

#[test]
fn mem_query_output_is_deterministic_under_replay() {
    let repo = temp_repo("mem-determinism");
    let writes: [(&str, &str, &str, &str); 3] = [
        (
            "alpha",
            "alpha body about caching rows",
            "fact",
            "cache rows",
        ),
        (
            "beta",
            "beta body about resolver passes",
            "gotcha",
            "resolver passes",
        ),
        (
            "gamma",
            "gamma body about build timing",
            "decision",
            "build timing",
        ),
    ];
    let run_queries = || {
        let mut outs = Vec::new();
        for (terms, kind) in [
            ("caching rows", "--kind fact"),
            ("resolver passes", "--kind gotcha"),
            ("build timing", "--kind decision"),
        ] {
            let kind_flag = if kind == "--kind fact" {
                vec!["--kind", "fact"]
            } else if kind == "--kind gotcha" {
                vec!["--kind", "gotcha"]
            } else {
                vec!["--kind", "decision"]
            };
            let (code, stdout, stderr) =
                mem_out(&repo, &[&["query", terms][..], &kind_flag[..]].concat());
            assert_eq!(code, 0, "{stdout}{stderr}");
            outs.push(stdout);
        }
        outs
    };
    for (key, body, kind, tags) in writes {
        let (code, _stdout, stderr) = mem_out(
            &repo,
            &["write", key, "--body", body, "--kind", kind, "--tags", tags],
        );
        assert_eq!(code, 0, "{stderr}");
    }
    let first = run_queries();
    // Replay the exact write sequence: every write is an idempotent no-op,
    // and the query output is byte-identical.
    for (key, body, kind, tags) in writes {
        let (code, stdout, _stderr) = mem_out(
            &repo,
            &["write", key, "--body", body, "--kind", kind, "--tags", tags],
        );
        assert_eq!(code, 0);
        assert!(
            stdout.contains("unchanged at rev 1"),
            "replay must be a no-op: {stdout}"
        );
    }
    let second = run_queries();
    assert_eq!(first, second, "same DB + same queries -> same bytes");
}
