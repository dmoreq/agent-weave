//! Integration tests: stats section.

use super::common::*;

#[test]
fn stats_path_scopes_counts() {
    // `--path` scopes every counter; the global line keeps its byte shape
    // (no `scope=` segment), and a scope with no indexed files is a
    // no-results, not an empty success line.
    let repo = temp_repo("stats-scope");
    write(
        &repo,
        "a.py",
        "def a1():\n    return a2()\n\ndef a2():\n    return 1\n",
    );
    write(
        &repo,
        "sub/c.py",
        "def c1():\n    return c2()\n\ndef c2():\n    return 1\n",
    );
    write(&repo, "sub/e.py", "def e1():\n    return 1\n");
    build(&repo);

    let out = aw(&repo).args(["stats"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let global = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        global.starts_with("aw stats: files=3 symbols=8 edges="),
        "global stats: {global}"
    );
    assert!(
        !global.contains("scope="),
        "global stats must not carry a scope: {global}"
    );

    let out = aw(&repo).args(["stats", "--path", "sub"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let scoped = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        scoped.starts_with("aw stats: scope=sub files=2 symbols=5 edges="),
        "scoped stats: {scoped}"
    );
    assert!(
        scoped.contains(" parse_errors=0 unresolved=0 "),
        "scoped stats: {scoped}"
    );

    // Trailing-slash and ./-prefixed spellings normalize to the same scope.
    let out = aw(&repo)
        .args(["stats", "--path", "./sub/"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        scoped,
        "./sub/ must normalize to sub"
    );

    // Scoped totals shrink with the scope on this fixture. Edge-kind totals
    // are not pinned: new kinds extend both lines.
    let field = |line: &str, key: &str| -> i64 {
        let start = line.find(&format!("{key}=")).unwrap() + key.len() + 1;
        line[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap()
    };
    assert!(
        field(&scoped, "calls") < field(&global, "calls"),
        "{scoped} vs {global}"
    );
    assert!(
        field(&scoped, "edges") < field(&global, "edges"),
        "{scoped} vs {global}"
    );

    let out = aw(&repo)
        .args(["stats", "--path", "nope"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "aw: no-results: no indexed files in 'nope'\n"
    );

    let out = aw(&repo).args(["stats", "--path", "."]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("invalid scope '.'"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn partial_secondary_index_rebuilds_fresh() {
    let repo = fixture("partial-index");
    build(&repo);
    let idx = repo.join(".aw/index.db");
    let status = std::process::Command::new("sqlite3")
        .args([
            idx.to_string_lossy().as_ref(),
            "DROP INDEX symbols_file_path;",
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "sqlite3 demotion failed (fixture-only tool)"
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(
        code, 0,
        "rebuild over partial index failed: {stdout}{stderr}"
    );
}

#[test]
fn partial_v7_file_path_column_rebuilds_fresh() {
    let repo = fixture("partial-file-path");
    build(&repo);
    let idx = repo.join(".aw/index.db");
    let status = std::process::Command::new("sqlite3")
        .args([
            idx.to_string_lossy().as_ref(),
            "DROP INDEX symbols_file_path;
             ALTER TABLE symbols DROP COLUMN file_path;",
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "sqlite3 demotion failed (fixture-only tool)"
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(
        code, 0,
        "rebuild over missing file_path failed: {stdout}{stderr}"
    );
    let out = aw(&repo).args(["search", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("utils.py"),
        "fresh index lost symbols after v10 rebuild"
    );
}

#[test]
fn old_version_database_rebuilds_to_v10() {
    // A v6 user_version disables the fast path; build rewrites the index at
    // the current schema version instead of carrying v6 rows.
    let repo = fixture("v6-to-v8");
    build(&repo);
    let idx = repo.join(".aw/index.db");
    let status = std::process::Command::new("sqlite3")
        .args([idx.to_string_lossy().as_ref(), "PRAGMA user_version = 6;"])
        .output()
        .unwrap();
    assert!(status.status.success(), "sqlite3 version demotion failed");
    let out = aw(&repo).args(["stats"]).output().unwrap();
    assert_eq!(out.status.code(), Some(5), "v6 query open must exit 5");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("schema v6, binary expects v10 — run aw build"),
        "{stderr}"
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "v6 rebuild failed: {stdout}{stderr}");
    let out = std::process::Command::new("sqlite3")
        .args([idx.to_string_lossy().as_ref(), "PRAGMA user_version;"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "10",
        "build did not stamp v10"
    );
}

#[test]
fn partial_v10_index_rebuilds_fresh() {
    let repo = fixture("partial-v4");
    build(&repo);
    let idx = repo.join(".aw/index.db");
    let status = std::process::Command::new("sqlite3")
        .args([
            idx.to_string_lossy().as_ref(),
            "ALTER TABLE symbols DROP COLUMN signature;",
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "sqlite3 demotion failed (fixture-only tool)"
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(
        code, 0,
        "rebuild over partial index failed: {stdout}{stderr}"
    );
    assert!(!stderr.contains("signature"), "{stdout}{stderr}");
}

// ---------------------------------------------------------------------------
// G17 (Amendment 13): `aw path` seeded fixtures — 10 scenarios, each asserting
// exact stdout/stderr bytes against a hand-pinned fixture graph.

/// Fixture repo for the path gate. Every file is pinned line-by-line; the
/// expected call sites below are derived from these exact byte layouts.
fn path_fixture(name: &str) -> PathBuf {
    let repo = temp_repo(name);
    write(
        &repo,
        "chain.py",
        "def ca():\n    return cb()\n\ndef cb():\n    return cc()\n\ndef cc():\n    return cd()\n\ndef cd():\n    return 1\n",
    );
    write(
        &repo,
        "diamond.py",
        "def dtop():\n    x = dm1()\n    return dm2()\n\ndef dm1():\n    return dbot()\n\ndef dm2():\n    return dbot()\n\ndef dbot():\n    return 0\n",
    );
    write(
        &repo,
        "cycle.py",
        "def cya():\n    return cyb()\n\ndef cyb():\n    return cyc()\n\ndef cyc():\n    return cya()\n",
    );
    write(
        &repo,
        "disconnected.py",
        "def iso1():\n    return 1\n\ndef iso2():\n    return 2\n",
    );
    let mut deep = String::new();
    for i in 1..=12 {
        deep.push_str(&format!("def f{i:02}():\n"));
        if i < 12 {
            deep.push_str(&format!("    return f{i:02}()\n", i = i + 1));
        } else {
            deep.push_str("    return 0\n");
        }
    }
    write(&repo, "deep.py", &deep);
    write(
        &repo,
        "multi.py",
        "def multi_caller():\n    x = mtarget()\n    y = mtarget()\n    return x + y\n\ndef mtarget():\n    return 1\n",
    );
    write(
        &repo,
        "amb.py",
        "class Apage:\n    def dup(self):\n        return target()\n\nclass Bpage:\n    def dup(self):\n        return 2\n\ndef target():\n    return 1\n",
    );
    build(&repo);
    repo
}

#[test]
fn path_seed_gate() {
    let repo = path_fixture("path-g17");

    // (1) 3-hop chain: hop call-site pointers in path order + resolved
    // qualname chain line.
    let out = aw(&repo).args(["path", "ca", "cd"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "aw: path — resolved static edges only; dynamic dispatch may hide paths\n\
         chain.py:2\nchain.py:5\nchain.py:8\n\
         aw: chain chain.ca -> chain.cb -> chain.cc -> chain.cd (3 hops)\n",
        "scenario 1: {stdout}"
    );

    // (2) Diamond tie-break: dtop reaches dbot through BOTH dm1 and dm2 at
    // 2 hops; dm1 expands before dm2 (qualname order), so the pinned
    // first-discoverer parent rule yields dtop -> dm1 -> dbot.
    let out = aw(&repo).args(["path", "dtop", "dbot"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "aw: path — resolved static edges only; dynamic dispatch may hide paths\n\
         diamond.py:2\ndiamond.py:6\n\
         aw: chain diamond.dtop -> diamond.dm1 -> diamond.dbot (2 hops)\n",
        "scenario 2: {stdout}"
    );

    // (3) Cycle: cya reaches cyc in 2 hops and terminates.
    let out = aw(&repo).args(["path", "cya", "cyc"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "aw: path — resolved static edges only; dynamic dispatch may hide paths\n\
         cycle.py:2\ncycle.py:5\n\
         aw: chain cycle.cya -> cycle.cyb -> cycle.cyc (2 hops)\n",
        "scenario 3: {stdout}"
    );

    // (4) Disconnected pair: definitive exhaustive negative.
    let out = aw(&repo).args(["path", "iso1", "iso2"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(stdout, "", "scenario 4 stdout: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "aw: no-results: no call path from 'disconnected.iso1' to 'disconnected.iso2' at any depth <= 10 (resolved edges only)\n",
        "scenario 4 stderr: {stderr}"
    );

    // (5) Depth-cap miss: the real path is 11 hops, beyond the 1-10 clamp,
    // so any --depth clamps to 10 and yields the cap-truncated negative.
    // Boundary: f01 -> f11 is exactly 10 hops and is found at the default;
    // --depth 11 behaves byte-identically (clamped).
    let out = aw(&repo).args(["path", "f01", "f12"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "aw: no-results: no call path from 'deep.f01' to 'deep.f12' within depth 10; a longer path may exist (resolved edges only)\n",
        "scenario 5 truncated stderr: {stderr}"
    );
    let mut expected =
        String::from("aw: path — resolved static edges only; dynamic dispatch may hide paths\n");
    for i in 1..=10 {
        expected.push_str(&format!("deep.py:{}\n", 2 * i));
    }
    expected.push_str("aw: chain deep.f01 -> deep.f02 -> deep.f03 -> deep.f04 -> deep.f05 -> deep.f06 -> deep.f07 -> deep.f08 -> deep.f09 -> deep.f10 -> deep.f11 (10 hops)\n");
    let out = aw(&repo).args(["path", "f01", "f11"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(stdout, expected, "scenario 5 boundary: {stdout}");
    let out = aw(&repo)
        .args(["path", "f01", "f11", "--depth", "11"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(stdout, expected, "scenario 5 clamp: {stdout}");

    // (5b) Min-clamp boundary: --depth 0 clamps to 1; ca -> cc needs 2 hops,
    // so the frontier stops with unvisited out-neighbors (cap-truncated).
    let out = aw(&repo)
        .args(["path", "ca", "cc", "--depth", "0"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "aw: no-results: no call path from 'chain.ca' to 'chain.cc' within depth 1; a longer path may exist (resolved edges only)\n",
        "scenario 5b min-clamp stderr: {stderr}"
    );

    // (6) from == to: same-symbol negative (exit 1); an ambiguous shared
    // name resolves FIRST and exits 2 (pinned order: ambiguity precedes the
    // same-symbol check).
    let out = aw(&repo).args(["path", "ca", "ca"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr, "aw: no-results: 'chain.ca' and 'chain.ca' resolve to the same symbol\n",
        "scenario 6 stderr: {stderr}"
    );
    let out = aw(&repo).args(["path", "dup", "dup"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "ambiguous shared name exits 2, not 1"
    );

    // (7) Ambiguous `from` endpoint: exit 2, role in the detail, candidates
    // on stdout; the Class.method form resolves.
    let out = aw(&repo).args(["path", "dup", "ca"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout, "amb.py:2 amb.Apage.dup\namb.py:6 amb.Bpage.dup\n",
        "scenario 7 stdout: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "aw: usage: ambiguous name 'dup' — 2 candidates as the from endpoint; use the Class.method qualname form\n",
        "scenario 7 stderr: {stderr}"
    );
    let out = aw(&repo)
        .args(["path", "amb.Apage.dup", "target"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "aw: path — resolved static edges only; dynamic dispatch may hide paths\n\
         amb.py:3\n\
         aw: chain amb.Apage.dup -> amb.target (1 hops)\n",
        "scenario 7 resolved form: {stdout}"
    );

    // (8) Ambiguous `to` endpoint: exit 2 with the `to` role.
    let out = aw(&repo).args(["path", "ca", "dup"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout, "amb.py:2 amb.Apage.dup\namb.py:6 amb.Bpage.dup\n",
        "scenario 8 stdout: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "aw: usage: ambiguous name 'dup' — 2 candidates as the to endpoint; use the Class.method qualname form\n",
        "scenario 8 stderr: {stderr}"
    );

    // (9) Multi-edge representative: two call sites of mtarget; the pointer
    // is the smallest site_line (line 2).
    let out = aw(&repo)
        .args(["path", "multi_caller", "mtarget"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "aw: path — resolved static edges only; dynamic dispatch may hide paths\n\
         multi.py:2\n\
         aw: chain multi.multi_caller -> multi.mtarget (1 hops)\n",
        "scenario 9: {stdout}"
    );

    // (10) Stale-index header: change a displayed file post-build; the
    // staleness line follows the caveat on the displayed set.
    write(
        &repo,
        "chain.py",
        "def ca():\n    return cb()\n\ndef cb():\n    return cc()\n\ndef cc():\n    return cd()\n\ndef cd():\n    return 1\n# touched\n",
    );
    let out = aw(&repo).args(["path", "ca", "cd"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "aw: path — resolved static edges only; dynamic dispatch may hide paths\n\
         aw: 1 of 1 result files changed since build — run aw build\n\
         chain.py:2\nchain.py:5\nchain.py:8\n\
         aw: chain chain.ca -> chain.cb -> chain.cc -> chain.cd (3 hops)\n",
        "scenario 10: {stdout}"
    );
}
