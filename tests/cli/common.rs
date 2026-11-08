//! Shared helpers and fixtures for the cli integration suite.

pub(crate) use std::fs;
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::process::Command;
pub(crate) use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) static TEMP_SEQ: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn temp_repo(name: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("aw-it-{}-{}-{seq}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub(crate) fn write(repo: &Path, rel: &str, content: &str) {
    let path = repo.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

pub(crate) fn aw(repo: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_aw"));
    c.arg("--root").arg(repo);
    c.env("AW_GLOBAL_DIR", repo.join(".test_global"));
    c
}

pub(crate) fn build(repo: &Path) -> (i32, String, String) {
    let out = aw(repo).args(["build"]).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Run one git command in a temp repo; fail the test on non-zero exit.
pub(crate) fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

// Raw strings: the previous `\`-continuation form stripped the leading
// indentation of every line, so class bodies collapsed to top-level defs and
// the fixtures never exercised method resolution at all.
pub(crate) const WIDGET: &str = r#"class Widget:
    def render(self):
        return normalize('x')

    def render2(self):
        return self.render()
"#;
pub(crate) const UTILS: &str = r#"def normalize(text):
    return text.strip()

def caller():
    return normalize('a')
"#;
pub(crate) const APP: &str = r#"from utils import normalize
from .widget import Widget as W

def run():
    w = W()
    return normalize(w.render())
"#;
pub(crate) const BROKEN: &str = "def broken(:\n    pass\n";

pub(crate) fn fixture(name: &str) -> PathBuf {
    let repo = temp_repo(name);
    write(&repo, "utils.py", UTILS);
    write(&repo, "widget.py", WIDGET);
    write(&repo, "app.py", APP);
    repo
}

/// Walk the tree collecting (path, len) for files outside .aw/ — the read-only
/// invariant probe.
pub(crate) fn snapshot(repo: &Path) -> Vec<(String, u64, u64)> {
    // (path, len, content hash) — hashes catch same-size edits (invariant 3).
    let mut out = Vec::new();
    fn rec(dir: &Path, acc: &mut Vec<(String, u64, u64)>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == ".aw") {
                continue;
            }
            if p.is_dir() {
                rec(&p, acc);
            } else {
                let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                let hash = fs::read(&p)
                    .map(|b| {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        b.hash(&mut h);
                        h.finish()
                    })
                    .unwrap_or(0);
                acc.push((p.to_string_lossy().into_owned(), len, hash));
            }
        }
    }
    rec(repo, &mut out);
    out.sort();
    out
}

pub(crate) fn output_with_timeout(
    mut cmd: std::process::Command,
    secs: u64,
) -> Option<std::process::Output> {
    use std::io::Read;
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut out) = child.stdout.take() {
                    out.read_to_string(&mut stdout).ok();
                }
                if let Some(mut err) = child.stderr.take() {
                    err.read_to_string(&mut stderr).ok();
                }
                return Some(std::process::Output {
                    status,
                    stdout: stdout.into_bytes(),
                    stderr: stderr.into_bytes(),
                });
            }
            Ok(None) => {}
            Err(_) => return None,
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

pub(crate) fn dump_edges(db: &Path) -> String {
    let sql = "SELECT e.kind, e.confidence, e.site_line,
                      sf.path, ss.qualname, ss.line_start,
                      COALESCE(df.path, ''), COALESCE(ds.qualname, e.dst_name),
                      COALESCE(ds.line_start, 0)
               FROM edges e
               JOIN symbols ss ON ss.id = e.src_id
               JOIN files sf ON sf.id = ss.file_id
               LEFT JOIN symbols ds ON ds.id = e.dst_id
               LEFT JOIN files df ON df.id = ds.file_id
               ORDER BY 1, 2, 3, 4, 5, 6, 7, 8, 9";
    let out = Command::new("sqlite3").arg(db).arg(sql).output().unwrap();
    assert!(out.status.success(), "sqlite3 failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub(crate) fn mem_write(repo: &Path, key: &str, body: &str, kind: &str) -> (i32, String, String) {
    let out = aw(repo)
        .args(["mem", "write", key, "--body", body, "--kind", kind])
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

pub(crate) fn mem_out(repo: &Path, args: &[&str]) -> (i32, String, String) {
    let out = aw(repo).args(["mem"]).args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}
