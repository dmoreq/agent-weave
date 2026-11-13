//! Usage journal: one JSONL line per journaled invocation appended to
//! `.engram/usage.log`. Best-effort by contract: a failed journal write is
//! silently ignored and can never alter a command's output or exit code.
//! The line is serialized by serde_json (the MCP boundary dependency); the
//! summary reader extracts only the fixed keys we write.

use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

mod reuse;
pub use reuse::{
    set_candidate_ids, set_freshness, set_judged, set_mode, set_reuse_counts, ReuseFreshness,
    ReuseJournalCounts,
};

/// Rotate `usage.log` to `usage.log.1` past this size (one prior generation).
pub(crate) const ROTATE_BYTES: u64 = 5 * 1024 * 1024;

pub fn usage_log_path(root: &Path) -> PathBuf {
    crate::db::eg_dir(root).join("usage.log")
}

/// Journal opt-out contract: env `ENGRAM_NO_JOURNAL=1` (checked in `begin`) or
/// the global `--no-journal` flag (checked in `main` — it skips `begin`).
pub(super) struct Pending {
    surface: &'static str,
    tool: String,
    pub(super) args: serde_json::Map<String, serde_json::Value>,
    result_count: usize,
    shown: usize,
    total: usize,
    ambiguous: usize,
    stale: bool,
    session: String,
    arm: String,
    agent: String,
    pub(super) judged: bool,
    pub(super) mode: &'static str,
    pub(super) truncated: bool,
    pub(super) reuse_counts: Option<ReuseJournalCounts>,
    pub(super) freshness: Option<Value>,
}

thread_local! {
    pub(super) static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
    static DISABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The opt-out is `1` in either spelling; any other value leaves journaling on.
fn no_journal_requested() -> bool {
    ["ENGRAM_NO_JOURNAL"]
        .iter()
        .any(|key| std::env::var(key).as_deref() == Ok("1"))
}

/// Disable journaling for this process without mutating the environment.
/// Used by the MCP server when `--no-journal` is requested.
pub fn disable() {
    DISABLED.with(|d| d.set(true));
}

/// Key/value arguments one journaled invocation carries: built by the
/// `service` layer's `journal_args` helpers and rendered into the usage
/// line's `args` object by `begin`.
pub type JournalArgs = Vec<(&'static str, String)>;

/// Arm the journal for this invocation. No-op when the env opt-out is set;
/// the `--no-journal` flag is enforced by the caller skipping `begin`.
pub fn begin(surface: &'static str, tool: &str, args: &JournalArgs) {
    if DISABLED.with(|d| d.get()) || no_journal_requested() {
        return;
    }
    let args: serde_json::Map<String, serde_json::Value> = args
        .iter()
        .map(|(k, v)| ((*k).to_string(), serde_json::Value::String(v.clone())))
        .collect();
    PENDING.with(|p| {
        *p.borrow_mut() = Some(Pending {
            surface,
            tool: tool.to_string(),
            args,
            result_count: 0,
            shown: 0,
            total: 0,
            ambiguous: 0,
            stale: false,
            session: env_tag(&["ENGRAM_SESSION"]),
            arm: env_tag(&["ENGRAM_ARM"]),
            agent: detect_agent(),
            judged: false,
            mode: "scan",
            truncated: false,
            reuse_counts: None,
            freshness: None,
        });
    });
}

/// Serialize and append the usage line. Called once per process after the
/// outcome is known; I/O failures are swallowed. A `None` root or an
/// identityless root skips the write (no `.engram/` location exists).
pub fn finalize(root: Option<&Path>, exit_code: i32, latency_us: u64) {
    let Some(pending) = PENDING.with(|p| p.borrow_mut().take()) else {
        return;
    };
    // One identity for every surface (see `repo::RepoRef`).
    let repo = root.map(crate::repo::RepoRef::from_root);
    let Some(repo_path) = repo.as_ref().and_then(crate::repo::RepoRef::journal_repo) else {
        return;
    };
    let repo_name = repo.as_ref().map(|r| r.name().to_string());
    let is_worktree = repo.as_ref().is_some_and(crate::repo::RepoRef::is_worktree);
    let reuse = pending
        .reuse_counts
        .map(|c| {
            serde_json::json!({
                "provisional": c.provisional,
                "low": c.low,
                "medium": c.medium,
                "high": c.high,
                "suppressed": c.suppressed,
                "guarded_out": c.guarded_out,
                "below_threshold": c.below_threshold,
                "omitted": c.omitted,
                "capped": c.capped,
                "truncated": c.truncated,
            })
        })
        .unwrap_or(Value::Null);

    let line = serde_json::json!({
        "ts": crate::db::unix_now(),
        "surface": pending.surface,
        "tool": pending.tool,
        "args": Value::Object(pending.args),
        "mode": pending.mode,
        "truncated": pending.truncated,
        "reuse": reuse,
        "freshness": pending.freshness.unwrap_or(Value::Null),
        "result_count": pending.result_count,
        "shown": pending.shown,
        "total": pending.total,
        "ambiguous": pending.ambiguous,
        "exit_code": exit_code,
        "latency_us": latency_us,
        "stale": pending.stale,
        "engram_version": env!("CARGO_PKG_VERSION"),
        "session": pending.session,
        "arm": pending.arm,
        "agent": pending.agent,
        "repo": repo_path,
        "repo_name": repo_name,
        "worktree": is_worktree,
        "judged": pending.judged,
    });
    let line_str = format!("{line}\n");
    // Never append to tracked, attacker-authored state.
    if root.is_some_and(|r| crate::db::reject_tracked_eg_dir(r).is_err()) {
        return;
    }
    if let Some(root) = root {
        append_line(&usage_log_path(root), &line_str);
    }
    if let Ok(global_log) = crate::db::global_usage_log_path() {
        if let Some(parent) = global_log.parent() {
            let should_append = if crate::db::global_dir_overridden() {
                parent.is_dir()
            } else {
                true
            };
            if should_append {
                append_line(&global_log, &line_str);
            }
        }
    }
}

/// Replace a pruned log through a same-directory temp file. Kept here because
/// this file owns the journal write allowlist.
pub(super) fn rewrite_pruned_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "log has no parent")
    })?;
    let temp = parent.join(format!(
        ".usage-prune-{}-{}",
        std::process::id(),
        crate::db::unix_now()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    if let Err(err) = file
        .write_all(text.as_bytes())
        .and_then(|_| file.sync_all())
    {
        let _ = fs::remove_file(&temp);
        return Err(err);
    }
    drop(file);
    let _ = crate::db::harden_file_mode(&temp);
    if let Err(err) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(err);
    }
    Ok(())
}

fn append_line(path: &Path, line: &str) {
    // Write confinement: the journal is derived state and may only grow a
    // regular file under a real `.engram/` directory. A symlinked directory or log
    // would append attacker-chosen content outside the repo (the read path
    // already rejects non-regular files).
    if let Some(dir) = path.parent() {
        if let Ok(meta) = fs::symlink_metadata(dir) {
            if !meta.is_dir() {
                return;
            }
        }
    }
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() {
            return;
        }
    }
    rotate_if_needed(path, ROTATE_BYTES);
    // Whether `.engram` exists before this append: the journal seeds the gitignore
    // marker only when IT creates the directory. A pre-existing `.engram` is owned
    // by build/doctor (or the user) — appending must not touch its marker.
    let eg_created_here = path
        .parent()
        .is_none_or(|dir| fs::symlink_metadata(dir).is_err());
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let mut file = match OpenOptions::new().append(true).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            recreate_journal(path, eg_created_here);
            match OpenOptions::new().create_new(true).append(true).open(path) {
                Ok(file) => file,
                Err(_) => return,
            }
        }
        Err(_) => return,
    };
    // Owner-only mode: the journal carries repository paths and tool usage.
    let _ = crate::db::harden_file_mode(path);
    let _ = file.write_all(line.as_bytes());
}

/// Seed the `.engram` gitignore marker on the recreate path, but only when THIS
/// append created the directory: a pre-existing `.engram` is owned by
/// build/doctor (or the user) and appending must not touch its marker.
fn recreate_journal(path: &Path, eg_created_here: bool) {
    if !eg_created_here {
        return;
    }
    if let Ok(global_dir) = crate::db::global_eg_dir() {
        if path.starts_with(&global_dir) {
            return;
        }
    }
    let Some(root) = path.parent().and_then(Path::parent) else {
        return;
    };
    let _ = crate::db::ensure_eg_gitignore(root);
}

fn rotate_if_needed(path: &Path, max: u64) {
    let oversized = fs::metadata(path).map(|m| m.len() > max).unwrap_or(false);
    if !oversized {
        return;
    }
    let mut rotated = path.as_os_str().to_owned();
    rotated.push(".1");
    let _ = fs::rename(path, PathBuf::from(rotated));
}

mod agent;
pub(crate) mod liveness;
mod outcome;
mod prune;
mod render;
mod tokens;

pub use prune::{
    prune_ephemeral_entries, prune_ephemeral_entries_all, prune_stale_entries,
    prune_stale_entries_all,
};

use agent::env_tag;
pub use agent::{detect_agent, session_tag, set_agent_context};
pub(crate) use outcome::*;
pub(crate) use render::*;
pub(crate) use tokens::*;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_moves_log_aside_past_threshold() {
        let dir = crate::db::temp_root("journal");
        let path = dir.join("usage.log");
        fs::write(&path, b"1234567").unwrap();
        rotate_if_needed(&path, 5);
        assert!(!path.exists());
        assert_eq!(fs::read(dir.join("usage.log.1")).unwrap(), b"1234567");
        rotate_if_needed(&path, 5); // absent -> no-op
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_fields_round_trip() {
        let line = "{\"ts\":1,\"surface\":\"cli\",\"tool\":\"callers\",\"args\":{\"name\":\"a\"},\"result_count\":3,\"shown\":3,\"total\":3,\"ambiguous\":1,\"exit_code\":0,\"latency_us\":9800,\"stale\":false,\"engram_version\":\"0.1.0\"}";
        let f = parse_usage_line(line).unwrap();
        assert_eq!(f.tool, "callers");
        assert_eq!(f.result_count, 3);
        assert_eq!(f.ambiguous, 1);
        assert_eq!(f.latency_us, Some(9800));
        assert!(!f.stale);
        // Lines without optional keys keep working.
        let legacy = parse_usage_line(
            "{\"ts\":1,\"surface\":\"cli\",\"tool\":\"callers\",\"result_count\":1,\"exit_code\":0}",
        )
        .unwrap();
        assert_eq!(legacy.tool, "callers");
        assert_eq!(legacy.latency_us, None);
        assert_eq!(legacy.shown, None);
        let zero = parse_usage_line(r#"{"tool":"callers","shown":0,"exit_code":0}"#).unwrap();
        assert_eq!(zero.shown, Some(0));
    }

    #[test]
    fn usage_parsing_unescapes_quoted_text() {
        let line = r#"{"tool":"search","args":{"terms":"say \"run\" \\ now"},"result_count":2,"exit_code":0,"latency_us":1200,"stale":true,"ambiguous":3}"#;
        let f = parse_usage_line(line).unwrap();
        assert_eq!(f.tool, "search");
        assert_eq!(f.result_count, 2);
        assert_eq!(f.latency_us, Some(1200));
        assert!(f.stale);
        assert_eq!(f.ambiguous, 3);
    }

    #[test]
    fn summary_quality_splits_empty_and_cap() {
        let root = crate::db::temp_root("journal-quality");
        fs::create_dir_all(root.join(".engram")).unwrap();
        let repo = root.to_string_lossy();
        let lines = [
            format!(
                r#"{{"tool":"callers","repo":"{repo}","result_count":2,"shown":2,"total":2,"exit_code":0,"latency_us":1000}}"#
            ),
            format!(
                r#"{{"tool":"callers","repo":"{repo}","result_count":0,"shown":0,"total":0,"exit_code":0,"latency_us":2000}}"#
            ),
            format!(
                r#"{{"tool":"callers","repo":"{repo}","result_count":0,"shown":0,"total":0,"exit_code":2,"latency_us":3000}}"#
            ),
            format!(
                r#"{{"tool":"callers","repo":"{repo}","result_count":50,"shown":50,"total":60,"exit_code":0,"latency_us":4000}}"#
            ),
        ];
        let lines = lines.iter().map(|l| l.as_str()).collect::<Vec<_>>();
        fs::write(usage_log_path(&root), lines.join("\n")).unwrap();

        let default = summary(&root, false).unwrap().join("\n");
        assert!(
            default.contains("callers: 4 calls, errors=0 empty=2 stale=0 ambiguous=0"),
            "{default}"
        );
        assert!(!default.contains("empty_ok"), "{default}");

        let quality = summary(&root, true).unwrap().join("\n");
        assert!(quality.contains("empty_ok=1 (25.0%)"), "{quality}");
        assert!(quality.contains("empty_error=1"), "{quality}");
        assert!(quality.contains("capped=1"), "{quality}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn env_tag_trims_and_caps_at_64() {
        // Safety: test-only env mutation, single-threaded test binary section.
        std::env::set_var("ENGRAM_TEST_TAG", "  r1-task-1  ");
        assert_eq!(env_tag(&["ENGRAM_TEST_TAG"]), "r1-task-1");
        std::env::set_var("ENGRAM_TEST_TAG", "x".repeat(100));
        assert_eq!(env_tag(&["ENGRAM_TEST_TAG"]).len(), 64);
        std::env::remove_var("ENGRAM_TEST_TAG");
        assert_eq!(env_tag(&["ENGRAM_TEST_TAG"]), "");
    }

    #[cfg(unix)]
    #[test]
    fn prune_preserves_owner_only_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::db::temp_root("journal-prune-mode");
        let log = dir.join("usage.log");
        fs::write(
            &log,
            "{\"tool\":\"build\",\"repo\":\"/missing/repo\",\"worktree\":true}\n",
        )
        .unwrap();
        fs::set_permissions(&log, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(prune_stale_entries(&log).unwrap(), 1);
        let mode = fs::metadata(&log).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn prune_refuses_symlinked_log() {
        let dir = crate::db::temp_root("journal-prune-link");
        let target = dir.join("user-config.json");
        fs::write(&target, b"{\"theme\":\"dark\"}\n").unwrap();
        let log = dir.join("usage.log");
        std::os::unix::fs::symlink(&target, &log).unwrap();
        let pruned = prune_stale_entries(&log).unwrap();
        assert_eq!(pruned, 0);
        assert_eq!(fs::read(&target).unwrap(), b"{\"theme\":\"dark\"}\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn append_refuses_symlinked_log_and_directory() {
        let dir = crate::db::temp_root("journal-link");
        let target = dir.join("target.txt");
        fs::write(&target, b"keep").unwrap();
        let log = dir.join("usage.log");
        std::os::unix::fs::symlink(&target, &log).unwrap();
        append_line(&log, "line\n");
        assert_eq!(fs::read(&target).unwrap(), b"keep");

        // A symlinked `.engram` directory must not redirect the write either.
        let outside = dir.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let linked_root = dir.join("repo");
        fs::create_dir_all(&linked_root).unwrap();
        std::os::unix::fs::symlink(&outside, linked_root.join(".engram")).unwrap();
        append_line(&linked_root.join(".engram/usage.log"), "line\n");
        assert!(!outside.join("usage.log").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_detection_and_tagging() {
        set_agent_context("custom-agent");
        assert_eq!(detect_agent(), "custom-agent");

        let line = r#"{"tool":"symbols","exit_code":0,"agent":"antigravity","worktree":false}"#;
        let f = parse_usage_line(line).unwrap();
        assert_eq!(f.agent.as_deref(), Some("antigravity"));
    }
}
