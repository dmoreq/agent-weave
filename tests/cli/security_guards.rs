//! Security regressions: tracked/symlinked `.engram` state and untrusted
//! index text reaching terminal output.

use super::common::*;

#[test]
fn tracked_index_is_rejected_before_query() {
    let repo = temp_repo("tracked-index");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    write(&repo, ".engram/index.db.new", "tracked new\n");
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &[
            "add",
            "-f",
            ".engram/index.db",
            ".engram/build.lock",
            ".engram/index.db.new",
        ],
    );
    for args in [
        vec!["search", "unique_one"],
        vec!["reuse"],
        vec!["usage"],
        vec!["audit"],
        vec!["build"],
    ] {
        let out = eg(&repo).args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("git-tracked"), "{args:?}: {stderr}");
    }
    // A rejected invocation must not mutate the tracked confinement state.
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["status", "--porcelain", "--", ".engram"])
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&status.stdout);
    assert!(
        !status
            .lines()
            .any(|line| line.starts_with(" M") || line.starts_with(" D")),
        "{status}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn tracked_case_variant_engram_is_rejected() {
    let repo = temp_repo("tracked-case-variant");
    write(&repo, "a.py", "def unique_case_marker():\n    return 1\n");
    build(&repo);
    std::fs::rename(repo.join(".engram"), repo.join(".ENGRAM")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "-f", ".ENGRAM/index.db"]);
    let out = eg(&repo)
        .args(["search", "unique_case_marker"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "case-variant bypass");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("git-tracked"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn symlinked_engram_journal_is_rejected() {
    let repo = temp_repo("symlink-journal");
    let outside = temp_repo("symlink-journal-outside");
    write(
        &outside,
        "usage.log",
        "{\"tool\":\"build\",\"exit_code\":0,\"result_count\":1,\"shown\":1,\"repo\":\"/x\",\"ts\":1}\n",
    );
    std::os::unix::fs::symlink(&*outside, repo.join(".engram")).unwrap();
    for args in [vec!["usage"], vec!["audit"]] {
        let out = eg(&repo).args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(6), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("refusing symlinked"), "{args:?}: {stderr}");
    }
}

#[cfg(unix)]
#[test]
fn symlinked_journal_file_is_rejected() {
    let repo = temp_repo("symlink-journal-file");
    let outside = temp_repo("symlink-journal-file-outside");
    write(
        &outside,
        "usage.log",
        "{\"tool\":\"build\",\"exit_code\":0,\"result_count\":1,\"shown\":1,\"repo\":\"/x\",\"ts\":1}\n",
    );
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    let link = repo.join(".engram/usage.log");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(outside.join("usage.log"), &link).unwrap();
    for args in [vec!["usage"], vec!["audit"]] {
        let out = eg(&repo).args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(6), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("refusing unsafe journal"),
            "{args:?}: {stderr}"
        );
    }
}

#[test]
fn index_derived_control_bytes_are_escaped_in_output() {
    let repo = temp_repo("control-bytes");
    let evil = "evil\u{1b}[31mred\u{7}.py";
    write(
        &repo,
        evil,
        "def unique_control_marker():\n    return 1\n\ndef unique_control_other():\n    return 2\n",
    );
    build(&repo);
    for args in [
        vec!["search", "unique_control_marker"],
        vec!["symbols", evil],
        vec!["context", "unique_control_marker"],
    ] {
        let out = eg(&repo).args(&args).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(!stdout.contains('\u{1b}'), "{args:?}: raw ESC: {stdout}");
        assert!(!stdout.contains('\u{7}'), "{args:?}: raw BEL: {stdout}");
        assert!(stdout.contains("\\x1b[31m"), "{args:?}: {stdout}");
    }
    let out = eg(&repo)
        .args(["path", "unique_control_marker", "unique_control_other"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains('\u{1b}'),
        "raw ESC in path error: {stderr}"
    );
    assert!(!stderr.contains('\u{7}'), "raw BEL in path error: {stderr}");
    assert!(stderr.contains("\\x1b[31m"), "{stderr}");
}
