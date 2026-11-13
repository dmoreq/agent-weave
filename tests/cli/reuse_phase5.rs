//! Phase 5 judge tests: missing judge degrades without changing static output.

use super::common::*;

#[cfg(unix)]
#[test]
fn judge_reviews_proposal_candidates() {
    use std::os::unix::fs::PermissionsExt;

    let repo = temp_repo("reuse-judge-proposal");
    write(
        &repo,
        "a.py",
        "def normalize(value):\n    return value\n\ndef helper(value):\n    return value\n\ndef parse_token(value):\n    return normalize(value) + helper(value)\n",
    );
    build(&repo);
    let bin = repo.join("judge.sh");
    std::fs::write(
        &bin,
        "#!/bin/sh\nbody=$(cat)\ncase \"$body\" in *'\"code_a\"'*) printf '%s\\n' '{\"decision\":\"distinct\",\"probability\":0.2}' ;; *) printf '%s\\n' '{\"decision\":\"duplicate\",\"probability\":0.9}' ;; esac\n",
    )
    .unwrap();
    let mut perms = std::fs::metadata(&bin).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&bin, perms).unwrap();
    std::fs::create_dir_all(repo.join("model")).unwrap();
    std::fs::write(repo.join("model/lazy.json"), "{\"head_layers\":1}\n").unwrap();

    let out = eg(&repo)
        .env("EG_JUDGE_BIN", &bin)
        .env("EG_JUDGE_MODEL", repo.join("model"))
        .args([
            "reuse",
            "--propose",
            "def parse_token(value)",
            "--callees",
            "normalize,helper",
            "--judge",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("0 duplication candidate(s) found"),
        "{stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("judge unavailable"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn judge_receives_source_on_stdin_and_filters_distinct() {
    use std::os::unix::fs::PermissionsExt;

    let repo = temp_repo("reuse-judge-stdin");
    write(
        &repo,
        "a.py",
        "def normalize(value):\n    return value\n\ndef parse_token_value_data_extra(value):\n    return normalize(value)\n",
    );
    write(
        &repo,
        "b.py",
        "def normalize(value):\n    return value\n\ndef parse_token_value(value):\n    return normalize(value)\n",
    );
    build(&repo);
    let bin = repo.join("judge.sh");
    std::fs::write(
        &bin,
        "#!/bin/sh\nbody=$(cat)\ncase \"$body\" in *'\"code_a\"'*) printf '%s\\n' '{\"decision\":\"distinct\",\"probability\":0.4}' ;; *) printf '%s\\n' '{\"decision\":\"duplicate\",\"probability\":0.9}' ;; esac\n",
    )
    .unwrap();
    let mut perms = std::fs::metadata(&bin).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&bin, perms).unwrap();
    std::fs::create_dir_all(repo.join("model")).unwrap();
    std::fs::write(repo.join("model/lazy.json"), "{\"head_layers\":1}\n").unwrap();

    let out = eg(&repo)
        .env("EG_JUDGE_BIN", &bin)
        .env("EG_JUDGE_MODEL", repo.join("model"))
        .args(["reuse", "--judge"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("0 duplication candidate(s) found"),
        "{stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("judge unavailable"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn judge_malformed_output_keeps_static_result_with_note() {
    use std::os::unix::fs::PermissionsExt;

    let repo = temp_repo("reuse-judge-malformed");
    write(
        &repo,
        "a.py",
        "def normalize(value):\n    return value\n\ndef parse_token_value_data_extra(value):\n    return normalize(value)\n",
    );
    write(
        &repo,
        "b.py",
        "def normalize(value):\n    return value\n\ndef parse_token_value(value):\n    return normalize(value)\n",
    );
    build(&repo);
    let bin = repo.join("judge.sh");
    std::fs::write(&bin, "#!/bin/sh\ncat >/dev/null\nprintf 'not-json\\n'\n").unwrap();
    let mut perms = std::fs::metadata(&bin).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&bin, perms).unwrap();
    std::fs::create_dir_all(repo.join("model")).unwrap();
    std::fs::write(repo.join("model/lazy.json"), "{\"head_layers\":1}\n").unwrap();

    let out = eg(&repo)
        .env("EG_JUDGE_BIN", &bin)
        .env("EG_JUDGE_MODEL", repo.join("model"))
        .args(["reuse", "--judge"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("duplication candidate(s) found"),
        "{stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stderr.matches("judge unavailable").count(), 1, "{stderr}");
}

#[test]
fn judge_missing_binary_degrades_with_one_note() {
    let repo = temp_repo("reuse-judge-missing");
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
    let out = eg(&repo)
        .env("EG_JUDGE_BIN", "/nonexistent/eg-judge")
        .env("EG_JUDGE_MODEL", "/nonexistent/model")
        .args(["reuse", "--judge"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("duplication candidate(s) found"),
        "{stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stderr.matches("judge unavailable").count(), 1, "{stderr}");
}
