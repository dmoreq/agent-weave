//! Integration tests: dual-format export and symmetric import (Phase B/C of
//! the portable-memory amendment): Markdown framing, JSONL piping, provenance,
//! and the reject-on-conflict default.

use std::io::Write;
use std::process::Stdio;

use super::common::*;

fn write_file(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, text).unwrap();
    path
}

fn export_markdown(repo: &Path) -> String {
    let (code, markdown, stderr) = mem_out(repo, &["export", "--format", "markdown"]);
    assert_eq!(code, 0, "{markdown}{stderr}");
    assert!(markdown.starts_with("@@ engram-note "), "{markdown}");
    markdown
}

#[test]
fn mem_export_markdown_and_import_round_trip_with_procedure() {
    let repo = temp_repo("mem-import-md");
    let (code, stdout, stderr) = mem_write(
        &repo,
        "run:build",
        "Run cargo build before merge",
        "procedure",
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let markdown = export_markdown(&repo);
    assert!(markdown.contains("\"kind\":\"procedure\""), "{markdown}");
    assert!(markdown.contains("@@ end engram-note "), "{markdown}");

    // Restore into a fresh repository (the fresh-clone CUJ).
    let fresh = temp_repo("mem-import-fresh");
    let path = write_file(&fresh, "memory.md", &markdown);
    let (code, stdout, stderr) = mem_out(&fresh, &["import", path.to_str().unwrap()]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("imported 1"), "{stdout}");
    let (code, stdout, _stderr) = mem_out(&fresh, &["get", "run:build"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("procedure"), "{stdout}");
    assert!(stdout.contains("[imported]"), "{stdout}");
}

#[test]
fn mem_import_conflicts_require_force() {
    let repo = temp_repo("mem-import-conflict");
    let _ = mem_write(&repo, "auth", "original body", "decision");
    let markdown = export_markdown(&repo);
    let edited = markdown.replace("original body", "divergent body");
    let path = write_file(&repo, "memory.md", &edited);

    // Default: reject and skip a key whose local content differs (exit 2).
    let (code, _stdout, stderr) = mem_out(&repo, &["import", path.to_str().unwrap()]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("conflict"), "{stderr}");
    let (code, stdout, _stderr) = mem_out(&repo, &["get", "auth"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("original body"), "{stdout}");

    // --force opens a new revision with the imported body.
    let (code, stdout, stderr) = mem_out(&repo, &["import", path.to_str().unwrap(), "--force"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let (code, stdout, _stderr) = mem_out(&repo, &["get", "auth"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("divergent body"), "{stdout}");
    assert!(stdout.contains("r2"), "{stdout}");
    assert!(stdout.contains("[imported]"), "{stdout}");
}

#[test]
fn mem_import_duplicate_is_noop_and_jsonl_pipes_over_stdin() {
    let repo = temp_repo("mem-import-stdin");
    let _ = mem_write(&repo, "k", "streamed body", "fact");
    let markdown = export_markdown(&repo);
    let path = write_file(&repo, "memory.md", &markdown);
    let (code, stdout, _stderr) = mem_out(&repo, &["import", path.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(stdout.contains("skipped 1"), "{stdout}");

    let jsonl = eg(&repo)
        .args(["mem", "export", "--format", "jsonl"])
        .output()
        .unwrap();
    assert_eq!(jsonl.status.code(), Some(0));
    let mut child = eg(&repo)
        .args(["mem", "import", "-", "--format", "jsonl"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&jsonl.stdout)
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("skipped 1"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn mem_import_accepts_crlf_and_rejects_usage_errors() {
    let repo = temp_repo("mem-import-usage");
    let _ = mem_write(&repo, "crlf", "windows body", "fact");
    let markdown = export_markdown(&repo).replace('\n', "\r\n");
    let path = write_file(&repo, "memory-crlf.md", &markdown);
    let (code, stdout, stderr) = mem_out(&repo, &["import", path.to_str().unwrap()]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("skipped 1"), "{stdout}");

    // Unknown --format and zero --limit are usage errors (exit 2).
    let garbage = write_file(&repo, "garbage.md", "not a note\n");
    let (code, _stdout, stderr) = mem_out(
        &repo,
        &["import", garbage.to_str().unwrap(), "--format", "yaml"],
    );
    assert_eq!(code, 2);
    assert!(stderr.contains("--format must be"), "{stderr}");
    let (code, _stdout, stderr) = mem_out(&repo, &["export", "--limit", "0"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("--limit must be at least 1"), "{stderr}");
    let (code, _stdout, stderr) = mem_out(
        &repo,
        &["import", garbage.to_str().unwrap(), "--format", "markdown"],
    );
    assert_eq!(code, 2);
    assert!(stderr.contains("expected '@@ engram-note"), "{stderr}");
}
