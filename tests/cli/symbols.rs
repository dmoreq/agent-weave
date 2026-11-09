//! Integration tests: symbols section.

use super::common::*;

#[test]
fn symbols_lists_file_symbols_and_unknown_exit1() {
    let repo = fixture("symbols");
    build(&repo);
    let out = eg(&repo).args(["symbols", "utils.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("utils.py:1 module utils"), "{stdout}");
    assert!(
        stdout.contains("utils.py:1 function utils.normalize"),
        "{stdout}"
    );
    assert!(
        stdout.contains("utils.py:4 function utils.caller"),
        "{stdout}"
    );

    let out = eg(&repo).args(["symbols", "nope.py"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not indexed"), "{stderr}");
}

#[test]
fn symbols_outline_adds_signature_and_default_stays_byte_identical() {
    let repo = fixture("symbols-outline");
    build(&repo);
    let base = eg(&repo).args(["symbols", "utils.py"]).output().unwrap();
    assert_eq!(base.status.code(), Some(0));
    let plain = String::from_utf8_lossy(&base.stdout).into_owned();
    let out = eg(&repo)
        .args(["symbols", "utils.py", "--outline"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let outlined = String::from_utf8_lossy(&out.stdout).into_owned();

    // Amendment 12a: identical line count; each outline line is the plain
    // line plus a one-line signature (<= 160 chars, never a body).
    let plain_lines: Vec<&str> = plain.lines().collect();
    let outlined_lines: Vec<&str> = outlined.lines().collect();
    assert_eq!(plain_lines.len(), outlined_lines.len(), "{outlined}");
    for (p, o) in plain_lines.iter().zip(outlined_lines.iter()) {
        // Empty-signature rows (module symbols) stay byte-identical.
        let sig = match o.strip_prefix(format!("{p} ").as_str()) {
            Some(s) => s,
            None => {
                assert_eq!(p, o, "outline line '{o}' diverges from plain '{p}'");
                continue;
            }
        };
        assert!(sig.chars().count() <= 160, "signature over cap: {sig}");
        assert!(
            !sig.contains("return"),
            "signature must not carry bodies: {sig}"
        );
    }
    // At least one def line actually gained a signature.
    assert!(
        outlined_lines
            .iter()
            .zip(plain_lines.iter())
            .any(|(o, p)| o.len() > p.len() + 1),
        "no line gained a signature: {outlined}"
    );
}

#[test]
fn search_limit_counts_union_by_identity() {
    let repo = temp_repo("search-overcount");
    write(&repo, "a.py", "def run(): pass\n");
    write(&repo, "b.py", "def run(): pass\n");
    write(&repo, "c.py", "def run(): pass\ndef run_build(): pass\n");
    build(&repo);
    let out = eg(&repo)
        .args(["search", "run", "--limit", "1"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("showing 1 of 4"), "{text}");
}

#[test]
fn search_exact_first_merge_and_union_count_characterization() {
    let repo = temp_repo("search-characterize");
    write(&repo, "a.py", "def run(): pass\n");
    write(&repo, "b.py", "def run(): pass\n");
    write(&repo, "c.py", "def run_build(): pass\n");
    build(&repo);
    let out = eg(&repo)
        .args(["search", "run", "--limit", "10"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    let exact = lines
        .iter()
        .position(|line| line.contains(" a.run ") || line.contains(" b.run "))
        .expect("exact run row");
    let prefix = lines
        .iter()
        .position(|line| line.contains(" c.run_build "))
        .expect("prefix run_build row");
    assert!(exact < prefix, "exact-first merge order broken: {lines:?}");

    let capped = eg(&repo)
        .args(["search", "run", "--limit", "1"])
        .output()
        .unwrap();
    assert_eq!(capped.status.code(), Some(0), "{capped:?}");
    assert!(
        String::from_utf8_lossy(&capped.stdout).contains("showing 1 of 3"),
        "union count changed: {capped:?}"
    );
}

#[test]
fn search_finds_symbols_ranked_and_empty_or_nomatch_exit_codes() {
    let repo = fixture("search");
    build(&repo);
    // Hit: bm25-ranked pointers, tier position empty (symbol identities).
    let _ = fs::remove_file(repo.join(".engram/usage.log"));
    let out = eg(&repo).args(["search", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    // Every search exit path journals, including the exact-name fast path.
    let log = fs::read_to_string(repo.join(".engram/usage.log")).unwrap();
    let rec = log.lines().last().unwrap();
    assert!(
        rec.contains("\"tool\":\"search\"") && !rec.contains("\"result_count\":0"),
        "exact-name search must not journal an empty call: {rec}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("utils.py:1"), "{stdout}");
    assert!(
        !stdout.contains("[ambiguous]"),
        "search hits carry no tier tag: {stdout}"
    );
    // Amendment 14b: the hit line carries the stored signature.
    assert!(
        stdout.contains("def normalize(text)"),
        "search hit carries the signature: {stdout}"
    );
    // Amendment 14a: signature-only vocabulary is searchable (no symbol
    // name contains the `def` token; every signature does).
    let out = eg(&repo).args(["search", "def"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("utils.py:1"),
        "signature tokens reach symbols: {stdout}"
    );

    // No match: exit 1 with the documented reason line.
    let out = eg(&repo).args(["search", "zzzqqqxyz"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("0 symbols matched"), "{stderr}");

    // Empty terms: usage error, exit 2.
    let out = eg(&repo).args(["search", "   "]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn journal_written_and_usage_summarizes() {
    // Amendment 6: build + callers journal entries; eg usage aggregates them.
    let repo = fixture("journal");
    eg(&repo)
        .env_remove("ENGRAM_NO_JOURNAL")
        .args(["build"])
        .output()
        .unwrap();
    eg(&repo)
        .env_remove("ENGRAM_NO_JOURNAL")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    let log = fs::read_to_string(repo.join(".engram/usage.log")).unwrap();
    assert_eq!(log.lines().count(), 2, "one entry per invocation: {log}");
    assert!(log.contains("\"tool\":\"build\""), "{log}");
    assert!(log.contains("\"tool\":\"callers\""), "{log}");
    assert!(log.contains("\"surface\":\"cli\""), "{log}");

    let out = eg(&repo)
        .env_remove("ENGRAM_NO_JOURNAL")
        .args(["usage"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("eg usage: 2 journaled calls"), "{stdout}");
    assert!(stdout.contains("callers: 1 calls"), "{stdout}");
    assert!(stdout.contains("build: 1 calls"), "{stdout}");
}

#[test]
fn usage_quality_adds_empty_split_and_keeps_default_summary() {
    let repo = fixture("usage-quality");
    eg(&repo).args(["build"]).output().unwrap();
    eg(&repo).args(["callers", "normalize"]).output().unwrap();

    let default = eg(&repo).args(["usage"]).output().unwrap();
    assert_eq!(default.status.code(), Some(0));
    let default_stdout = String::from_utf8_lossy(&default.stdout);
    assert!(
        default_stdout.contains("callers: 1 calls"),
        "{default_stdout}"
    );
    assert!(!default_stdout.contains("empty_ok="), "{default_stdout}");

    let quality = eg(&repo).args(["usage", "--quality"]).output().unwrap();
    assert_eq!(quality.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&quality.stdout);
    assert!(stdout.contains("empty_ok="), "{stdout}");
    assert!(stdout.contains("empty_error="), "{stdout}");
    assert!(stdout.contains("capped="), "{stdout}");
    assert!(stdout.contains('%'), "{stdout}");
}

#[test]
fn journal_opt_out_via_env_and_flag() {
    let repo = fixture("journal-env");
    eg(&repo)
        .env("ENGRAM_NO_JOURNAL", "1")
        .args(["build"])
        .output()
        .unwrap();
    assert!(
        !repo.join(".engram/usage.log").exists(),
        "env opt-out must suppress the journal"
    );

    let repo = fixture("journal-flag");
    eg(&repo).args(["--no-journal", "build"]).output().unwrap();
    assert!(
        !repo.join(".engram/usage.log").exists(),
        "--no-journal must suppress the journal"
    );

    // usage on an empty journal: exit 1 with a reason.
    let out = eg(&repo).args(["usage"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no journal entries"), "{stderr}");
}

#[test]
fn usage_journal_creates_gitignore_marker() {
    // The journal self-creates `.engram` on the first write; the tool-owned
    // gitignore marker must exist so the directory never leaks into git status.
    let repo = temp_repo("journal-gitignore");
    write(&repo, "a.py", "def f(): pass\n");
    // Baseline commit: `git status --porcelain` must be empty before the
    // journal writes anything, so only `.engram` leakage could dirty it.
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "-A"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-qm",
            "init",
        ],
    );

    // `callers` fails with no index (exit 1) but still journals.
    let out = eg(&repo).args(["callers", "f"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        repo.join(".engram/usage.log").exists(),
        "journal file written"
    );
    let marker = fs::read_to_string(repo.join(".engram/.gitignore")).unwrap();
    assert_eq!(marker, "*\n", "tool-owned gitignore marker");
    let status = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&status.stdout).trim().is_empty(),
        ".engram must be invisible to git: {:?}",
        String::from_utf8_lossy(&status.stdout)
    );
}

#[cfg(unix)]
#[test]
fn fifo_gitignore_marker_fails_closed_without_hanging() {
    // A FIFO at .engram/.gitignore is fail-closed: rejected before any open (no
    // hang), left in place, and never written through.
    use std::os::unix::fs::FileTypeExt;
    let repo = temp_repo("fifo-gitignore");
    write(&repo, "a.py", "def f(): pass\n");
    fs::create_dir_all(repo.join(".engram")).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(repo.join(".engram/.gitignore"))
            .status()
            .unwrap()
            .success(),
        "mkfifo failed"
    );

    let mut cmd = eg(&repo);
    cmd.arg("build");
    // The budget only bounds the wait; a true FIFO hang is killed and still
    // fails via None. Sized for a loaded host, where a sub-second fail-closed
    // exit can inflate past a 20 s wall clock.
    let out =
        output_with_timeout(cmd, 60).expect("build must not hang on a FIFO .engram/.gitignore");
    assert_eq!(
        out.status.code(),
        Some(6),
        "FIFO marker must fail closed: {out:?}"
    );
    let meta = fs::symlink_metadata(repo.join(".engram/.gitignore")).unwrap();
    assert!(
        meta.file_type().is_fifo(),
        "marker must be left untouched, not replaced"
    );
}

#[cfg(unix)]
#[test]
fn fifo_indexed_file_does_not_hang_doctor() {
    // A FIFO replacing an indexed file must not block the staleness re-hash:
    // file_changed reads through the same bounded guard as the walk
    // (regular-file check + 1 MiB cap) and treats the oddity as changed. The
    // indexed file is empty so its stored size matches the FIFO's stat size
    // and the probe reaches the read path instead of short-circuiting on
    // size.
    use std::os::unix::fs::FileTypeExt;
    let repo = temp_repo("fifo-doctor");
    write(&repo, "empty.py", "");
    build(&repo);
    let fifo = repo.join("empty.py");
    fs::remove_file(&fifo).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success(),
        "mkfifo failed"
    );
    let meta = fs::symlink_metadata(&fifo).unwrap();
    assert!(meta.file_type().is_fifo(), "fixture must be a FIFO");

    let mut cmd = eg(&repo);
    cmd.args(["doctor"]);
    let out = output_with_timeout(cmd, 10)
        .expect("doctor must not hang on an indexed file replaced by a FIFO");
    assert_eq!(out.status.code(), Some(0), "doctor must succeed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("stale ratio: 100.0%"),
        "the FIFO must count as changed (1 of 1 files): {stdout}"
    );
}

#[test]
fn session_and_arm_tags_recorded_in_usage_log() {
    // ENGRAM_SESSION/ENGRAM_ARM tags, sanitized, land in the usage log.
    let repo = fixture("session-tags");
    build(&repo);
    eg(&repo)
        .env_remove("ENGRAM_NO_JOURNAL")
        .env("ENGRAM_SESSION", "  r1-task-1  ")
        .env("ENGRAM_ARM", "eg")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    let usage = fs::read_to_string(repo.join(".engram/usage.log")).unwrap();
    assert!(usage.contains("\"session\":\"r1-task-1\""), "{usage}");
    assert!(usage.contains("\"arm\":\"eg\""), "{usage}");
}

#[test]
fn g5_no_child_processes_survive() {
    // Invariant 2: eg spawns only short-lived git commands and leaves no
    // descendants. Poll the child tree during the build (the previous test
    // only looked after exit, R6e finding), then assert nothing tagged with
    // the fixture token survives.
    let repo = fixture("proc");
    let marker = format!("eg-proc-{}", std::process::id());
    write(&repo, &format!("{marker}.py"), "def f():\n    pass\n");
    for i in 0..200 {
        write(&repo, &format!("pad{i}.py"), "def p():\n    return 1\n");
    }
    let mut child = eg(&repo).arg("build").spawn().unwrap();
    let pid = child.id().to_string();
    let mut seen: std::collections::HashSet<String> = Default::default();
    let mut pgrep_ok = 0usize;
    loop {
        match Command::new("pgrep").args(["-P", &pid, "-l"]).output() {
            Ok(out) if out.status.code() == Some(0) => {
                pgrep_ok += 1;
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    if let Some((_, comm)) = line.split_once(char::is_whitespace) {
                        let comm = comm.trim();
                        if !comm.is_empty() {
                            seen.insert(comm.to_string());
                        }
                    }
                }
            }
            Ok(out) if out.status.code() == Some(1) => {
                pgrep_ok += 1;
            }
            Ok(out) => {
                if child.try_wait().unwrap().is_none() {
                    panic!("pgrep failed while build was alive: {out:?}");
                }
            }
            Err(e) => panic!("pgrep inspection failed: {e}"),
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let status = child.wait().unwrap();
    assert!(status.success());
    assert!(pgrep_ok > 0, "process inspection never succeeded");
    // A non-git fixture may have no child at all; the seeded-child test below
    // separately proves the inspection mechanism is not vacuous.
    for comm in &seen {
        assert!(
            comm == "git" || comm.ends_with("/git"),
            "unexpected child process during build: {comm} (seen: {seen:?})"
        );
    }
    let probe = Command::new("pgrep")
        .args(["-f", &marker])
        .output()
        .unwrap();
    let alive = String::from_utf8_lossy(&probe.stdout);
    assert!(
        probe.status.code() == Some(1),
        "pgrep inspection did not report a clean no-match: {probe:?}"
    );
    assert!(alive.trim().is_empty(), "processes still alive: {alive}");
}

#[test]
fn g5_process_inspection_detects_seeded_child() {
    // Guard the guard: pgrep must see a live descendant whose command line
    // carries the marker. Without this, an empty result could pass vacuously.
    let marker = format!("eg-seed-{}", std::process::id());
    let script = format!("sleep 30; : {marker}");
    let mut child = Command::new("sh").args(["-c", &script]).spawn().unwrap();
    let mut found = false;
    for _ in 0..100 {
        let out = Command::new("pgrep")
            .args(["-f", &marker])
            .output()
            .unwrap();
        assert!(out.status.code() == Some(0) || out.status.code() == Some(1));
        if out.status.code() == Some(0) {
            found = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(found, "pgrep did not detect the seeded child");
}

#[test]
fn p15_stats_line() {
    let repo = fixture("p15");
    build(&repo);
    let out = eg(&repo).args(["stats"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("files=3") && stdout.contains("symbols=") && stdout.contains("edges="),
        "{stdout}"
    );
}
