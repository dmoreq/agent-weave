//! Explain, local scope, and legacy-mode acceptance.

use std::path::Path;
use std::process::Command;

use super::super::common::{eg, mem_out, mem_write, temp_repo};

fn sql(repo: &Path, statement: &str) {
    let out = Command::new("sqlite3")
        .arg(repo.join(".engram/memory.db"))
        .arg(statement)
        .output()
        .expect("sqlite3");
    assert!(
        out.status.success(),
        "sqlite3 failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write_global(repo: &Path, key: &str, body: &str) {
    let out = eg(repo)
        .args([
            "mem", "write", key, "--body", body, "--kind", "fact", "--global",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "global write failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn no_why_without_explain_and_one_why_with_it() {
    let repo = temp_repo("mem-explain-flag");
    mem_write(
        &repo,
        "note",
        "resolver tiers select lexical candidates",
        "fact",
    );

    let (code, out, _) = mem_out(&repo, &["query", "resolver"]);
    assert_eq!(code, 0);
    assert!(!out.contains("why:"), "{out}");

    let (code, out, _) = mem_out(&repo, &["query", "resolver", "--explain"]);
    assert_eq!(code, 0);
    let why_lines: Vec<&str> = out
        .lines()
        .filter(|l| l.trim_start().starts_with("why:"))
        .collect();
    assert_eq!(why_lines.len(), 1, "{out}");
    assert!(why_lines[0].contains("lexical match"), "{out}");
    assert!(why_lines[0].contains("age "), "{out}");
    for forbidden in ["bm25", "score", "weight", "0."] {
        assert!(!why_lines[0].contains(forbidden), "{why_lines:?}");
    }
}

#[test]
fn ref_only_candidate_has_no_lexical_label() {
    let repo = temp_repo("mem-explain-ref");
    mem_write(&repo, "plain", "resolver tiers select candidates", "fact");
    mem_write(&repo, "refnote", "other words", "fact");
    sql(
        &repo,
        "UPDATE memories SET refs = 'src/memory/rank.rs' WHERE key = 'refnote'",
    );

    let (code, out, err) = mem_out(&repo, &["query", "rank.rs", "--explain"]);
    assert_eq!(code, 0, "stderr: {err}");
    let lines: Vec<&str> = out.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.starts_with("refnote r"))
        .unwrap_or_else(|| panic!("refnote header missing: {out}"));
    let block: Vec<&str> = lines[start + 1..]
        .iter()
        .take_while(|l| l.starts_with("  "))
        .copied()
        .collect();
    let block = block.join("\n");
    assert!(block.contains("why: ref src/memory/rank.rs"), "{block}");
    assert!(!block.contains("lexical match"), "{block}");
}

#[test]
fn explain_redacts_a_secret_bearing_ref() {
    let repo = temp_repo("mem-explain-redact");
    mem_write(&repo, "leak", "other words", "fact");
    let secret_ref = format!("src/leak/sk-{}", "a".repeat(24));
    sql(
        &repo,
        &format!("UPDATE memories SET refs = '{secret_ref}' WHERE key = 'leak'"),
    );
    let (code, out, err) = mem_out(&repo, &["query", "leak", "--explain"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("ref <redacted>"), "{out}");
    assert!(!out.contains(&secret_ref), "{out}");
}

#[test]
fn local_excludes_global_and_missing_local_exits_one() {
    let repo = temp_repo("mem-local");
    mem_write(&repo, "localnote", "shared deploy local", "fact");
    write_global(&repo, "globalnote", "shared deploy global");

    let (code, out, _) = mem_out(&repo, &["query", "deploy"]);
    assert_eq!(code, 0);
    assert!(out.contains("globalnote r"), "{out}");

    let (code, out, _) = mem_out(&repo, &["query", "deploy", "--local"]);
    assert_eq!(code, 0);
    assert!(out.contains("localnote r"), "{out}");
    assert!(!out.contains("globalnote r"), "{out}");
    assert!(!out.contains("[global]"), "{out}");

    let empty = temp_repo("mem-local-missing");
    write_global(&empty, "globalnote", "only global");
    let (code, _, err) = mem_out(&empty, &["query", "only", "--local"]);
    assert_eq!(code, 1, "stderr: {err}");
    assert!(err.contains("no .engram/memory.db"), "{err}");
}

#[test]
fn local_global_conflict_is_exit_two_with_one_message() {
    let repo = temp_repo("mem-local-conflict");
    mem_write(&repo, "note", "body", "fact");
    let (code, _, err) = mem_out(&repo, &["query", "body", "--local", "--global"]);
    assert_eq!(code, 2, "stderr: {err}");
    assert!(
        err.contains("mem query: --local and --global are mutually exclusive"),
        "{err}"
    );
}

#[test]
fn legacy_flag_uses_the_ascii_contract() {
    let repo = temp_repo("mem-legacy");
    mem_write(&repo, "viet", "Bộ nhớ dự án lưu quyết định", "fact");

    let (code, _, _) = mem_out(&repo, &["query", "bo nho", "--legacy"]);
    assert_eq!(code, 1, "legacy tokens must not match an unaccented query");

    let (code, out, err) = mem_out(&repo, &["query", "bộ nhớ", "--legacy"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("viet r"), "{out}");
}
