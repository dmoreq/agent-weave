//! Integration tests: memory_doctor section.

use super::common::*;

#[test]
fn mem_doctor_invalidate_preserves_memory() {
    let repo = temp_repo("mem-invalidate");
    let _ = mem_write(&repo, "keeper", "memory survives the reset", "decision");
    let (code, _stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stderr}");
    let out = aw(&repo).args(["doctor", "--invalidate"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("kept primary memory.db"), "{text}");
    assert!(
        repo.join(".aw/memory.db").exists(),
        "memory.db must survive --invalidate"
    );
    assert!(
        !repo.join(".aw/index.db").exists(),
        "the derived index is reset"
    );
    // --help documents the new reset scope (contract bump).
    let help = aw(&repo).args(["doctor", "--help"]).output().unwrap();
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(
        help_text.contains("memory.db") && help_text.contains("primary data"),
        "--help must document the kept store: {help_text}"
    );
    // Memory still answers; journal/report rows are gone with the index.
    let (code, stdout, stderr) = mem_out(&repo, &["query", "memory survives"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("keeper"), "{stdout}");
    // Doctor report carries the memory line (the index was reset above, so
    // rebuild first — the report opens the index).
    let (code, _stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stderr}");
    let out = aw(&repo).args(["doctor"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("memory: integrity ok, 1 notes"), "{text}");
}

#[test]
fn mem_concurrent_writers_serialize_or_exit4_never_unique() {
    // (a) Same key, different bodies: the loser either lands as the next rev
    // (busy_timeout serialization) or exits 4 — never a UNIQUE(key, rev) hit.
    let repo = temp_repo("mem-race-same");
    let _ = mem_write(&repo, "shared", "the original body", "fact");
    let ra_repo = repo.clone();
    let a = std::thread::spawn(move || mem_write(&ra_repo, "shared", "writer A body", "fact"));
    let rb_repo = repo.clone();
    let b = std::thread::spawn(move || mem_write(&rb_repo, "shared", "writer B body", "fact"));
    let (ra, rb) = (a.join().unwrap(), b.join().unwrap());
    for (code, _stdout, stderr) in [&ra, &rb] {
        assert!(
            *code == 0 || *code == 4,
            "loser must serialize (0) or exit 4, got {code}: {stderr}"
        );
        assert!(
            !stderr.contains("UNIQUE"),
            "constraint leaks are defects: {stderr}"
        );
    }
    let (code, stdout, _stderr) = mem_out(&repo, &["get", "shared"]);
    assert_eq!(code, 0);
    let revs: usize = stdout
        .lines()
        .last()
        .and_then(|l| l.strip_prefix("revs: "))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let expected = 1 + [&ra, &rb].iter().filter(|r| r.0 == 0).count();
    assert_eq!(
        revs, expected,
        "every successful write is one rev: {stdout}"
    );

    // (b) Different keys share SQLite's single writer, so under load one may
    // still serialize (0) or hit the busy timeout (4). What must never happen
    // is a UNIQUE leak, and at least one write must land.
    let repo = temp_repo("mem-race-distinct");
    let ra = {
        let repo = repo.clone();
        std::thread::spawn(move || mem_write(&repo, "left", "left body", "fact"))
    };
    let rb = {
        let repo = repo.clone();
        std::thread::spawn(move || mem_write(&repo, "right", "right body", "fact"))
    };
    let (ra, rb) = (ra.join().unwrap(), rb.join().unwrap());
    for (code, _stdout, stderr) in [&ra, &rb] {
        assert!(
            *code == 0 || *code == 4,
            "distinct-key writer must serialize (0) or exit 4, got {code}: {stderr}"
        );
        assert!(
            !stderr.contains("UNIQUE"),
            "constraint leaks are defects: {stderr}"
        );
    }
    // A writer that hit the busy timeout left no row; retry it once so the
    // end state does not depend on machine load. The concurrency invariant
    // (no UNIQUE leak) was already checked on the racing attempts.
    if ra.0 != 0 {
        let retry = mem_write(&repo, "left", "left body", "fact");
        assert_eq!(retry.0, 0, "retry left: {}{}", retry.1, retry.2);
    }
    if rb.0 != 0 {
        let retry = mem_write(&repo, "right", "right body", "fact");
        assert_eq!(retry.0, 0, "retry right: {}{}", retry.1, retry.2);
    }
    for key in ["left", "right"] {
        let (code, _stdout, stderr) = mem_out(&repo, &["get", key]);
        assert_eq!(code, 0, "{stderr}");
    }
}

#[test]
fn mem_export_round_trip_restores_the_note_set() {
    let repo = temp_repo("mem-export-src");
    let notes = [
        (
            "alpha-decision",
            "decision",
            "settled: alpha path",
            "tags alpha",
            "src/one.rs",
        ),
        (
            "beta-gotcha",
            "gotcha",
            "never do beta; it exits 5",
            "",
            "src/two.rs src/three.rs",
        ),
        (
            "gamma-fact",
            "fact",
            "gamma notes with a much longer body about retrieval behavior",
            "tags gamma",
            "",
        ),
    ];
    for (key, kind, body, tags, refs) in notes {
        let mut args = vec!["write", key, "--body", body, "--kind", kind];
        if !tags.is_empty() {
            args.push("--tags");
            args.push(tags);
        }
        if !refs.is_empty() {
            args.push("--refs");
            args.push(refs);
        }
        let (code, _stdout, stderr) = mem_out(&repo, &args);
        assert_eq!(code, 0, "{stderr}");
    }
    // Supersede one note so the export carries only current revs.
    let _ = mem_write(
        &repo,
        "alpha-decision",
        "settled: alpha path v2",
        "decision",
    );

    let (code, stdout, stderr) = mem_out(&repo, &["export"]);
    assert_eq!(code, 0, "{stderr}");
    // stdout is PURE JSONL: the caveat travels on stderr.
    assert!(
        !stdout.contains("aw: memory"),
        "export stdout must not carry the caveat: {stdout:?}"
    );
    assert!(stderr.starts_with("aw: memory — "), "{stderr:?}");
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("pure JSONL"))
        .collect();
    assert_eq!(lines.len(), 3, "{stdout}");
    let fingerprint = |rows: &[serde_json::Value]| -> Vec<String> {
        let mut v: Vec<String> = rows
            .iter()
            .map(|r| {
                format!(
                    "{}|{}|{}|{}|{}|{}",
                    r["key"].as_str().unwrap(),
                    r["kind"].as_str().unwrap(),
                    r["scope"].as_str().unwrap(),
                    r["body"].as_str().unwrap(),
                    r["tags"].as_str().unwrap(),
                    r["refs"].as_str().unwrap(),
                )
            })
            .collect();
        v.sort();
        v
    };
    let original = fingerprint(&lines);

    // Restore into a fresh store by replaying mem write per line.
    let fresh = temp_repo("mem-export-fresh");
    for r in &lines {
        let mut args = vec![
            "write",
            r["key"].as_str().unwrap(),
            "--body",
            r["body"].as_str().unwrap(),
            "--kind",
            r["kind"].as_str().unwrap(),
        ];
        let tags = r["tags"].as_str().unwrap();
        let refs = r["refs"].as_str().unwrap();
        if !tags.is_empty() {
            args.push("--tags");
            args.push(tags);
        }
        if !refs.is_empty() {
            args.push("--refs");
            args.push(refs);
        }
        let (code, _stdout, stderr) = mem_out(&fresh, &args);
        assert_eq!(code, 0, "{stderr}");
    }
    let (code, stdout2, stderr) = mem_out(&fresh, &["export"]);
    assert_eq!(code, 0, "{stderr}");
    let restored: Vec<serde_json::Value> = stdout2
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(original, fingerprint(&restored), "restore is content-exact");
    // --limit takes the N most recent, deterministically.
    let (code, stdout3, _stderr) = mem_out(&repo, &["export", "--limit", "1"]);
    assert_eq!(code, 0);
    let limited: Vec<serde_json::Value> = stdout3
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(limited.len(), 1);
    // Empty store: exit 1, nothing on stdout.
    let bare = temp_repo("mem-export-empty");
    let _ = mem_write(&bare, "temp", "note", "fact");
    let _ = mem_out(&bare, &["delete", "temp"]);
    let (code, stdout4, stderr) = mem_out(&bare, &["export"]);
    assert_eq!(code, 1);
    assert!(stdout4.is_empty(), "{stdout4:?}");
    assert!(stderr.contains("no current notes to export"), "{stderr}");
}

#[test]
fn mem_journal_records_len_never_body() {
    let repo = temp_repo("mem-journal");
    let secret_body = "the resolver keeps confidence tiers; AMBIGUOUS edges keep dst_id NULL";
    let (code, _stdout, stderr) = mem_write(&repo, "secret-check", secret_body, "decision");
    assert_eq!(code, 0, "{stderr}");
    let _ = mem_out(&repo, &["query", "confidence"]);
    let log = std::fs::read_to_string(repo.join(".aw/usage.log")).unwrap();
    assert!(log.contains("\"tool\":\"mem_write\""), "{log}");
    assert!(log.contains("\"tool\":\"mem_query\""), "{log}");
    assert!(
        log.contains("body_len"),
        "journal args carry body_len: {log}"
    );
    assert!(
        !log.contains("confidence tiers"),
        "journal must never carry the body: {log}"
    );
    // usage --quality shows the mem tools and the query empty rate.
    let out = aw(&repo).args(["usage", "--quality"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("mem_write:"), "{text}");
    assert!(text.contains("mem_query:"), "{text}");
}

#[test]
fn mem_recall_smoke_oracle_reaches_threshold() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/memory/oracle.json")).unwrap();
    let repo = temp_repo("mem-oracle");
    for note in fixture["notes"].as_array().unwrap() {
        let (code, _stdout, stderr) = mem_out(
            &repo,
            &[
                "write",
                note["key"].as_str().unwrap(),
                "--body",
                note["body"].as_str().unwrap(),
                "--kind",
                note["kind"].as_str().unwrap(),
                "--tags",
                note["tags"].as_str().unwrap(),
                "--refs",
                note["refs"].as_str().unwrap(),
            ],
        );
        assert_eq!(code, 0, "{stderr}");
    }
    let queries = fixture["queries"].as_array().unwrap();
    let mut hits = 0usize;
    for q in queries {
        let (code, stdout, stderr) = mem_out(
            &repo,
            &["query", q["terms"].as_str().unwrap(), "--limit", "3"],
        );
        assert_eq!(code, 0, "{} {stderr}", q["terms"].as_str().unwrap());
        let expected = q["expect"].as_str().unwrap();
        if stdout.contains(expected) {
            hits += 1;
        }
    }
    let recall = hits as f64 / queries.len() as f64;
    assert!(
        recall >= 0.8,
        "recall@3 = {recall} ({hits}/{}), below the 0.8 smoke threshold",
        queries.len()
    );
}
