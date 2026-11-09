//! Integration tests: memory_enhancements (Phase 1, 2, 3 verification).

use super::common::*;

#[test]
fn test_mem_query_empty_or_whitespace_normalizes_to_all() {
    let repo = temp_repo("mem-empty-query");
    let (code, stdout, stderr) = mem_write(
        &repo,
        "note-empty-test",
        "content for empty query normalization test",
        "fact",
    );
    assert_eq!(code, 0, "{stdout}{stderr}");

    // Query empty string: should exit 0 and return all notes instead of exiting 2
    let (code, stdout, stderr) = mem_out(&repo, &["query", ""]);
    assert_eq!(
        code, 0,
        "Querying empty string must exit 0: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("note-empty-test"),
        "stdout should contain note: {stdout}"
    );

    // Query whitespace string: should exit 2 per usage error contract
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "   "]);
    assert_eq!(code, 2, "Querying whitespace must exit 2: {stderr}");
    assert!(stderr.contains("provide terms or --all"), "{stderr}");
}

#[test]
fn test_mem_query_ref_filter() {
    let repo = fixture("mem-ref-filter");
    build(&repo);

    let out = aw(&repo)
        .args([
            "mem",
            "write",
            "note-utils",
            "--body",
            "utilities guidelines",
            "--kind",
            "decision",
            "--refs",
            "utils.py",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = aw(&repo)
        .args([
            "mem",
            "write",
            "note-widget",
            "--body",
            "widget guidelines",
            "--kind",
            "decision",
            "--refs",
            "widget.py",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    // Query with --ref utils.py
    let (code, stdout, stderr) = mem_out(&repo, &["query", "--all", "--ref", "utils.py"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("note-utils"),
        "must contain note-utils: {stdout}"
    );
    assert!(
        !stdout.contains("note-widget"),
        "must not contain note-widget: {stdout}"
    );

    // Query with --ref widget.py
    let (code, stdout, stderr) = mem_out(&repo, &["query", "--all", "--ref", "widget.py"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("note-widget"),
        "must contain note-widget: {stdout}"
    );
    assert!(
        !stdout.contains("note-utils"),
        "must not contain note-utils: {stdout}"
    );
}

#[test]
fn test_mem_touch_and_mem_write_touch() {
    let repo = temp_repo("mem-touch-test");
    let (code, stdout, stderr) = mem_write(&repo, "touch-note", "persistent body", "gotcha");
    assert_eq!(code, 0, "{stdout}{stderr}");

    // aw mem touch <key>
    let (code, stdout, stderr) = mem_out(&repo, &["touch", "touch-note"]);
    assert_eq!(code, 0, "aw mem touch failed: {stdout}{stderr}");
    assert!(
        stdout.contains("ok: 'touch-note' rev 1 touched"),
        "{stdout}"
    );

    // aw mem write --touch on duplicate content
    let out = aw(&repo)
        .args([
            "mem",
            "write",
            "touch-note",
            "--body",
            "persistent body",
            "--kind",
            "gotcha",
            "--touch",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ok: 'touch-note' touched at rev 1"),
        "{stdout}"
    );
}

#[test]
fn test_broken_ref_vs_modified_ref() {
    let repo = fixture("mem-ref-status");
    build(&repo);

    // Note 1: guard-utils referencing utils.py
    let out = aw(&repo)
        .args([
            "mem",
            "write",
            "guard-utils",
            "--body",
            "caution with utils",
            "--kind",
            "gotcha",
            "--refs",
            "utils.py",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    // Note 2: guard-missing referencing nonexistent_file.py
    let out = aw(&repo)
        .args([
            "mem",
            "write",
            "guard-missing",
            "--body",
            "caution with missing",
            "--kind",
            "gotcha",
            "--refs",
            "nonexistent_file.py",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    // Initially guard-missing has broken ref, while guard-utils is fresh (not stale)
    let (code, stdout, stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("[broken-ref]"),
        "must tag broken ref: {stdout}"
    );
    assert!(
        !stdout.contains("[modified-ref]"),
        "fresh note must not be modified: {stdout}"
    );

    // Now modify utils.py to bump mtime > created_at
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write(&repo, "utils.py", &format!("{UTILS}\n# modified\n"));
    build(&repo);

    let (code, stdout, stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("[modified-ref]"),
        "must tag modified ref: {stdout}"
    );
    assert!(
        stdout.contains("[broken-ref]"),
        "must still tag broken ref: {stdout}"
    );
}

#[test]
fn test_ambient_nudges_in_context_impact_and_build() {
    let repo = fixture("mem-ambient");
    build(&repo);

    // Write a gotcha referencing Widget.render
    let out = aw(&repo)
        .args([
            "mem",
            "write",
            "gotcha-render",
            "--body",
            "render method is prone to recursion loops",
            "--kind",
            "gotcha",
            "--refs",
            "Widget.render",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    // Write a decision referencing utils.py
    let out = aw(&repo)
        .args([
            "mem",
            "write",
            "decision-utils",
            "--body",
            "utils must have zero external dependencies",
            "--kind",
            "decision",
            "--refs",
            "utils.py",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    // 1. aw context Widget.render should co-retrieve the gotcha
    let out = aw(&repo)
        .args(["context", "Widget.render"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("memory: [gotcha] gotcha-render") || stdout.contains("recursion loops"),
        "context must show ambient gotcha: {stdout}"
    );

    // 2. aw impact utils.py should show decision warning
    let out = aw(&repo).args(["impact", "utils.py"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("warning: guarded by decision 'decision-utils'"),
        "impact must show decision guard: {stdout}"
    );

    // 3. Modifying utils.py and rebuilding should emit stderr hint
    write(&repo, "utils.py", &format!("{UTILS}\n# modified content\n"));
    let (code, _stdout, stderr) = build(&repo);
    assert_eq!(code, 0);
    assert!(
        stderr.contains("modified file 'utils.py' is guarded by decision 'decision-utils'"),
        "build must emit memory hint: {stderr}"
    );
}
