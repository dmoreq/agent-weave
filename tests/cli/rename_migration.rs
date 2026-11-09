//! Integration tests: pre-rename storage migration and the git-provenance
//! security gate on repo-local `.engram/` state.

use super::common::*;

#[test]
fn legacy_aw_memory_store_migrates_on_first_query() {
    let repo = temp_repo("rename-migrate");
    let (code, _stdout, stderr) = mem_write(&repo, "arch", "legacy decision body", "decision");
    assert_eq!(code, 0, "{stderr}");
    // Simulate an installation that only ever ran the pre-rename binary: the
    // store lives under `.aw/` and no `.engram/` exists at all.
    fs::rename(repo.join(".engram"), repo.join(".aw")).unwrap();
    assert!(!repo.join(".engram").exists());

    let (code, stdout, stderr) = mem_out(&repo, &["query", "legacy decision"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("arch"), "{stdout}");
    assert!(
        repo.join(".engram/memory.db").is_file(),
        "the note store is migrated into the new confinement directory"
    );
    assert!(
        repo.join(".aw/memory.db").is_file(),
        "the legacy store is copied, never moved or deleted"
    );
}

#[test]
fn git_tracked_confinement_dir_is_rejected_not_trusted() {
    let repo = temp_repo("rename-tracked");
    let (code, _stdout, stderr) = mem_write(&repo, "planted", "attacker note", "fact");
    assert_eq!(code, 0, "{stderr}");
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "-f", ".engram"]);

    let (code, stdout, stderr) = mem_out(&repo, &["query", "attacker"]);
    assert_eq!(code, 2, "{stdout}{stderr}");
    assert!(
        stderr.contains("eg: security: .engram/ contains git-tracked files"),
        "{stderr}"
    );

    // `eg build` reads repo-local state too: same refusal, same class.
    let (code, _stdout, stderr) = build(&repo);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("eg: security:"), "{stderr}");
}

#[test]
fn untracked_confinement_dir_still_queries() {
    let repo = temp_repo("rename-untracked");
    let (code, _stdout, stderr) = mem_write(&repo, "local", "tracked repo note", "fact");
    assert_eq!(code, 0, "{stderr}");
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "-A"]);
    let (code, stdout, stderr) = mem_out(&repo, &["query", "tracked repo"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("local"), "{stdout}");
}
