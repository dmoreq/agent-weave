//! Integration tests: p8_impact section.

use super::common::*;

#[test]
fn p8a_impact_defaults_to_files_with_site_counts() {
    let repo = temp_repo("impact-sites");
    write(&repo, "a.py", "def target():\n    return 1\n");
    write(
        &repo,
        "b.py",
        "from a import target\n\ndef one():\n    return target()\n\ndef two():\n    return target()\n",
    );
    build(&repo);
    let out = aw(&repo).args(["impact", "a.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert_eq!(
        stdout.lines().count(),
        1,
        "one row per caller file: {stdout}"
    );
    assert!(
        stdout.contains("b.py") && stdout.contains("(2 sites)"),
        "{stdout}"
    );

    let out = aw(&repo)
        .args(["impact", "a.py", "--sites"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("b.py:4") && stdout.contains("b.py:7"),
        "{stdout}"
    );
}

/// Split a documented command line into shell-like words, honoring single and
/// double quotes. Keeps the docs test dependency-free.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in line.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => cur.push(ch),
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            }
            None => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// Every `aw ...` invocation shown in README.md and docs/*.md must parse with
/// the real clap definition, so documentation cannot drift from `src/cli.rs`.
#[test]
fn documented_aw_commands_parse() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut docs = vec![root.join("README.md")];
    for entry in fs::read_dir(root.join("docs")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            docs.push(path);
        }
    }
    let mut checked = 0usize;
    let mut errors = Vec::new();
    for path in docs {
        let text = fs::read_to_string(&path).unwrap();
        let mut fenced = false;
        for (n, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                fenced = !fenced;
                continue;
            }
            if !fenced || !trimmed.starts_with("aw ") {
                continue;
            }
            // Skip placeholders, alternations, pipes, redirects, comments, and
            // help/version probes: these are templates, not literal invocations.
            if trimmed.contains('<')
                || trimmed.contains('[')
                || trimmed.contains('|')
                || trimmed.contains('>')
                || trimmed.contains('#')
                || trimmed.contains('$')
                || trimmed.contains('\\')
                || trimmed.contains("...")
                || trimmed.contains("--help")
                || trimmed.contains("--version")
            {
                continue;
            }
            let argv = shell_words(trimmed);
            match <agent_weave::cli::Cli as clap::Parser>::try_parse_from(&argv) {
                Ok(_) => checked += 1,
                Err(e) => errors.push(format!("{}:{}: `{trimmed}` -> {e}", path.display(), n + 1)),
            }
        }
    }
    assert!(
        errors.is_empty(),
        "documented commands failed:\n{}",
        errors.join("\n")
    );
    assert!(
        checked >= 25,
        "only {checked} literal aw invocations checked; docs test lost coverage"
    );
}

/// A gitignored tree can be huge and unreadable. `build` must not descend it:
/// the git-visible path list is enough to find the indexed files.
#[cfg(unix)]
#[test]
fn build_does_not_descend_unreadable_gitignored_tree() {
    use std::os::unix::fs::PermissionsExt;
    let repo = temp_repo("walk-ignored");
    write(&repo, ".gitignore", "generated/\n");
    write(&repo, "a.py", "def a():\n    return 1\n");
    let ignored = repo.join("generated");
    fs::create_dir_all(&ignored).unwrap();
    for i in 0..3000 {
        fs::write(ignored.join(format!("dummy_{i}.py")), "x = 1\n").unwrap();
    }
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "-A"]);
    fs::set_permissions(&ignored, fs::Permissions::from_mode(0o000)).unwrap();

    let started = std::time::Instant::now();
    let out = aw(&repo).args(["build"]).output().unwrap();
    let elapsed = started.elapsed();
    fs::set_permissions(&ignored, fs::Permissions::from_mode(0o755)).unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout={stdout} stderr={stderr}"
    );
    assert!(stdout.contains("files=1"), "{stdout}");
    // A healthy build of this fixture takes well under a second; the budget
    // guards against pathological slowness, not exact latency, so it is sized
    // for a loaded host where scheduling can inflate wall clock ~10x.
    assert!(
        elapsed < std::time::Duration::from_secs(60),
        "build took {elapsed:?}"
    );
}

#[test]
fn class_method_suffix_lookup_finds_callers_and_ambiguity() {
    let repo = temp_repo("suffix");
    write(
        &repo,
        "a.py",
        "class Pointer:\n    def render(self):\n        return 1\n\ndef go(p: Pointer):\n    return p.render()\n\ndef make():\n    return Pointer().render()\n",
    );
    build(&repo);
    let out = aw(&repo)
        .args(["callers", "Pointer.render"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("a.go"), "{stdout}");
    assert!(stdout.contains("a.make"), "{stdout}");

    // Two same-suffix methods stay ambiguous (exit 2 + candidates).
    let repo2 = temp_repo("suffix-ambiguous");
    write(
        &repo2,
        "m1.py",
        "class Pointer:\n    def render(self):\n        return 1\n",
    );
    write(
        &repo2,
        "m2.py",
        "class Pointer:\n    def render(self):\n        return 2\n",
    );
    build(&repo2);
    let out = aw(&repo2)
        .args(["callers", "Pointer.render"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stdout} {stderr}");
    assert!(stdout.contains("m1.Pointer.render"), "{stdout}");
    assert!(stdout.contains("m2.Pointer.render"), "{stdout}");
}

#[test]
fn search_keeps_prefix_matches_after_an_exact_symbol() {
    let repo = temp_repo("search-prefix");
    write(
        &repo,
        "a.py",
        "def run():\n    return 1\n\ndef run_build():\n    return 2\n",
    );
    build(&repo);
    let out = aw(&repo).args(["search", "run"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    let exact = stdout.find("a.run ").or_else(|| stdout.find("a.run\n"));
    let prefix = stdout.find("a.run_build");
    assert!(exact.is_some(), "exact run missing: {stdout}");
    assert!(prefix.is_some(), "run_build dropped: {stdout}");
    assert!(
        exact.unwrap() < prefix.unwrap(),
        "exact must rank first: {stdout}"
    );
}

#[test]
fn ambiguous_pointer_keeps_space_before_the_tag() {
    let repo = temp_repo("ambiguous-render");
    write(
        &repo,
        "a.py",
        "class A:\n    def m(self):\n        return 1\n\nclass B(A):\n    def m(self):\n        return 2\n\nclass C(A):\n    def m(self):\n        return 3\n\nclass D(B, C):\n    def go(self):\n        return self.m()\n",
    );
    build(&repo);
    let out = aw(&repo).args(["callees", "D.go"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains(" [ambiguous] a.D.m"), "{stdout}");
    assert!(!stdout.contains(":15[ambiguous]"), "{stdout}");
}

#[test]
fn root_symbols_work() {
    let repo = temp_repo("root-symbols");
    write(&repo, "a.py", "def orphan():\n    return 1\n");
    build(&repo);

    let root = aw(&repo).args(["symbols", "."]).output().unwrap();
    let root_stdout = String::from_utf8_lossy(&root.stdout);
    assert_eq!(root.status.code(), Some(0), "{root_stdout}");
    assert!(root_stdout.contains("aw: dir . —"), "{root_stdout}");
    let dotted = aw(&repo).args(["symbols", "./"]).output().unwrap();
    assert_eq!(dotted.status.code(), Some(0));
}

#[test]
fn stats_is_not_journaled_as_empty() {
    let repo = temp_repo("journal-results");
    write(&repo, "a.py", "def a():\n    return 1\n");
    build(&repo);
    let out = aw(&repo)
        .arg("stats")
        .env_remove("AW_NO_JOURNAL")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "stats");
    let usage = aw(&repo).args(["usage"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&usage.stdout);
    assert_eq!(usage.status.code(), Some(0), "{stdout}");
    let line = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("stats:"))
        .unwrap_or_else(|| panic!("stats missing from journal: {stdout}"));
    assert!(!line.contains("empty=1"), "stats misreported empty: {line}");
}

#[test]
fn doctor_does_not_warn_about_a_released_build_lock() {
    let repo = temp_repo("doctor-lock");
    write(&repo, "a.py", "def a():\n    return 1\n");
    build(&repo);
    let out = aw(&repo).args(["doctor"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(!stdout.contains("lock file:"), "{stdout}");
}

#[test]
fn path_endpoints_accept_class_method_suffixes() {
    let repo = temp_repo("path-suffix");
    write(
        &repo,
        "a.py",
        "class A:\n    def m(self):\n        return done()\n\ndef done():\n    return 1\n\ndef start(a: A):\n    return a.m()\n",
    );
    build(&repo);
    let out = aw(&repo).args(["path", "start", "A.m"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stdout}{stderr}");
    assert!(stdout.contains("a.A.m"), "{stdout}");
}

#[cfg(unix)]
fn external_symlink_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aw-ext-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(unix)]
#[test]
fn symlinked_aw_directory_is_rejected_without_external_writes() {
    use std::os::unix::fs::symlink;
    let repo = temp_repo("symlink-aw-dir");
    write(&repo, "sample.py", "def task():\n    pass\n");
    let ext = external_symlink_dir("aw-dir");
    let sentinel = ext.join("sentinel.txt");
    fs::write(&sentinel, "keep").unwrap();
    let _ = fs::remove_dir_all(repo.join(".aw"));
    symlink(&ext, repo.join(".aw")).unwrap();

    let out = aw(&repo).args(["build"]).output().unwrap();
    assert_ne!(out.status.code(), Some(0), "symlinked .aw must be refused");
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");
    assert!(!ext.join("index.db").exists());
    let _ = fs::remove_dir_all(&ext);
}

#[cfg(unix)]
#[test]
fn symlinked_aw_directory_to_missing_target_is_not_created() {
    use std::os::unix::fs::symlink;
    let repo = temp_repo("symlink-aw-missing");
    write(&repo, "sample.py", "def task():\n    pass\n");
    let target = std::env::temp_dir().join(format!("aw-ext-aw-missing-{}", std::process::id()));
    let _ = fs::remove_dir_all(&target);
    let _ = fs::remove_dir_all(repo.join(".aw"));
    symlink(&target, repo.join(".aw")).unwrap();

    let out = aw(&repo).args(["build"]).output().unwrap();
    assert_ne!(out.status.code(), Some(0));
    assert!(!target.exists(), "symlink target must not be created");
    let _ = fs::remove_file(repo.join(".aw"));
}

#[cfg(unix)]
#[test]
fn symlinked_build_lock_and_gitignore_are_rejected() {
    use std::os::unix::fs::symlink;
    let repo = temp_repo("symlink-lock");
    write(&repo, "sample.py", "def task():\n    pass\n");
    let ext = external_symlink_dir("lock");
    let sentinel = ext.join("sentinel.txt");
    fs::write(&sentinel, "keep").unwrap();
    fs::create_dir_all(repo.join(".aw")).unwrap();
    symlink(&sentinel, repo.join(".aw/build.lock")).unwrap();
    let lock = aw(&repo).args(["build"]).output().unwrap();
    assert_ne!(lock.status.code(), Some(0));
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");

    fs::remove_file(repo.join(".aw/build.lock")).unwrap();
    symlink(&sentinel, repo.join(".aw/.gitignore")).unwrap();
    let ignore = aw(&repo).args(["build"]).output().unwrap();
    assert_ne!(ignore.status.code(), Some(0));
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");
    let _ = fs::remove_file(repo.join(".aw/.gitignore"));
    let _ = fs::remove_dir_all(&ext);
}

#[cfg(unix)]
#[test]
fn invalidate_refuses_while_build_lock_is_held() {
    let repo = temp_repo("invalidate-lock");
    write(&repo, "sample.py", "def task():\n    pass\n");
    let _ = build(&repo);
    let sentinel = repo.join(".aw/sentinel");
    fs::write(&sentinel, "keep").unwrap();
    let lock = agent_weave::db::BuildLock::acquire(&repo).unwrap();

    let blocked = aw(&repo).args(["doctor", "--invalidate"]).output().unwrap();
    assert_eq!(blocked.status.code(), Some(4), "{blocked:?}");
    assert!(sentinel.exists(), "invalidate ran while root lock was held");

    drop(lock);
    let after = aw(&repo).args(["doctor", "--invalidate"]).output().unwrap();
    assert_eq!(after.status.code(), Some(0), "{after:?}");
    assert!(!repo.join(".aw").exists());
}

#[test]
fn removed_inheritance_does_not_survive_incremental_build() {
    let repo = temp_repo("removed-base");
    write(
        &repo,
        "sample.py",
        "class A:\n    def foo(self):\n        pass\n\nclass B(A):\n    pass\n\ndef invoke(b: B):\n    b.foo()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let before = aw(&repo).args(["callers", "A.foo"]).output().unwrap();
    assert_eq!(before.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&before.stdout).contains("sample.invoke"));

    write(
        &repo,
        "sample.py",
        "class A:\n    def foo(self):\n        pass\n\nclass B:\n    pass\n\ndef invoke(b: B):\n    b.foo()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let after = aw(&repo).args(["callers", "A.foo"]).output().unwrap();
    assert_eq!(after.status.code(), Some(1), "{after:?}");

    let force = aw(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(force.status.code(), Some(0));
    let forced = aw(&repo).args(["callers", "A.foo"]).output().unwrap();
    assert_eq!(forced.status.code(), Some(1), "{forced:?}");
}

#[test]
fn added_duplicate_candidate_invalidates_resolved_edges() {
    let repo = temp_repo("added-candidate");
    write(
        &repo,
        "caller.py",
        "from pkg import foo\n\ndef invoke():\n    foo()\n",
    );
    write(&repo, "pkg.py", "def foo():\n    pass\n");
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let before = aw(&repo).args(["callees", "invoke"]).output().unwrap();
    assert_eq!(before.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&before.stdout).contains("pkg.foo"),
        "{before:?}"
    );

    write(&repo, "pkg/__init__.py", "def foo():\n    pass\n");
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let after = aw(&repo).args(["callees", "invoke"]).output().unwrap();
    assert_eq!(after.status.code(), Some(0), "{after:?}");
    assert!(
        String::from_utf8_lossy(&after.stdout).contains("[ambiguous]"),
        "incremental build kept a stale resolved edge: {after:?}"
    );

    let incremental_db = repo.join("incremental.db");
    fs::copy(repo.join(".aw/index.db"), &incremental_db).unwrap();
    let force = aw(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(force.status.code(), Some(0));
    let forced = aw(&repo).args(["callees", "invoke"]).output().unwrap();
    assert_eq!(after.stdout, forced.stdout);
    assert_eq!(
        dump_edges(&incremental_db),
        dump_edges(&repo.join(".aw/index.db")),
        "incremental and fresh edge sets differ"
    );

    let ctx = aw(&repo).args(["context", "invoke"]).output().unwrap();
    assert_eq!(ctx.status.code(), Some(0), "{ctx:?}");
    let text = String::from_utf8_lossy(&ctx.stdout);
    assert!(
        text.starts_with("aw: context — resolved static edges plus tagged [ambiguous] candidates;"),
        "{text}"
    );
    assert!(text.contains("[ambiguous] foo"), "{text}");
}
