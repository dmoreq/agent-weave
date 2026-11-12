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
pub use reuse::{set_candidate_ids, set_reuse_counts, ReuseJournalCounts};

/// Rotate `usage.log` to `usage.log.1` past this size (one prior generation).
pub(crate) const ROTATE_BYTES: u64 = 5 * 1024 * 1024;

pub fn usage_log_path(root: &Path) -> PathBuf {
    crate::db::eg_dir(root).join("usage.log")
}

/// Session/arm attribution: R1/R2 A/B tags, sanitized. The first key that is
/// set wins, so the pre-rename spelling still feeds attribution.
fn env_tag(keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| std::env::var(key).ok())
        .map(|v| v.trim().chars().take(64).collect())
        .unwrap_or_default()
}

/// Session attribution for memory rows (`memories.session`): the same
/// sanitized `ENGRAM_SESSION` tag the journal line carries. Public because the
/// memory write path stamps it at write time.
pub fn session_tag() -> String {
    env_tag(&["ENGRAM_SESSION"])
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
        });
    });
}

/// Serialize and append the usage line. Called exactly once per process,
/// after the outcome is known. All I/O failures are swallowed by contract.
/// An invocation that resolved no repository (a `None` root, or any root
/// without an identity) skips the write entirely: there is no `.engram/`
/// location, and an unattributable line would only surface as a ghost row.
pub fn finalize(root: Option<&Path>, exit_code: i32, latency_us: u64) {
    let Some(pending) = PENDING.with(|p| p.borrow_mut().take()) else {
        return;
    };
    // One identity for every surface. The `file_name` and `.git` probes that
    // used to live here belong to `repo::RepoRef`.
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

/// Prune usage.log entries that carry no live repository: dead absolute roots
/// (deleted worktrees, temp scratch repos, ordinary repositories — the purge
/// policy) and unjudgeable lines without a usable `repo` identity (legacy
/// binaries, test artifacts). Rewrites the log file in place. Returns the
/// count of pruned lines.
pub fn prune_stale_entries(log_path: &Path) -> std::io::Result<usize> {
    if !log_path.is_file() {
        return Ok(0);
    }
    let content = match fs::read_to_string(log_path) {
        Ok(c) => c,
        Err(_) => return Ok(0),
    };

    let mut kept = Vec::new();
    let mut pruned = 0;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if liveness::entry_is_stale(&val) || liveness::entry_is_unattributed(&val) {
                pruned += 1;
                continue;
            }
        }
        kept.push(trimmed);
    }

    if pruned > 0 {
        let mut new_text = kept.join("\n");
        if !new_text.is_empty() {
            new_text.push('\n');
        }
        fs::write(log_path, new_text)?;
    }

    Ok(pruned)
}

/// Prune stale entries from both current and rotated (.1) generations.
pub fn prune_stale_entries_all(base_path: &Path) -> std::io::Result<usize> {
    let mut total = prune_stale_entries(base_path)?;
    if let Some(parent) = base_path.parent() {
        let rotated = parent.join("usage.log.1");
        if rotated.is_file() {
            total += prune_stale_entries(&rotated)?;
        }
    }
    Ok(total)
}

mod agent;
pub(crate) mod liveness;
mod outcome;
mod render;
mod tokens;

pub use agent::{detect_agent, set_agent_context};
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

    #[test]
    fn prune_stale_entries_removes_dead_repos_and_unjudgeable_lines() {
        let dir = crate::db::temp_root("prune-wt");
        let active_wt = dir.join("active-wt");
        let deleted_wt = dir.join("deleted-wt");
        fs::create_dir_all(&active_wt).unwrap();

        let log_path = dir.join("usage.log");
        let line1 = format!(
            r#"{{"tool":"build","repo":"{}","worktree":true}}"#,
            active_wt.display()
        );
        let line2 = format!(
            r#"{{"tool":"build","repo":"{}","worktree":true}}"#,
            deleted_wt.display()
        );
        let line3 = r#"{"tool":"symbols","repo":"/persistent/repo","worktree":false}"#.to_string();
        // Identityless legacy lines (pre-repo-field binaries, test artifacts)
        // are pruned alongside the dead roots.
        let line4 =
            r#"{"tool":"symbols","repo":".tmp/verify/repo1","repo_name":"repo1"}"#.to_string();
        let line5 = r#"{"tool":"mem_query","exit_code":0}"#.to_string();

        fs::write(
            &log_path,
            format!("{line1}\n{line2}\n{line3}\n{line4}\n{line5}\n"),
        )
        .unwrap();

        let pruned = prune_stale_entries(&log_path).unwrap();
        assert_eq!(
            pruned, 4,
            "deleted worktree, dead repo, relative and missing repo"
        );

        let remaining = fs::read_to_string(&log_path).unwrap();
        assert!(remaining.contains(&active_wt.to_string_lossy().to_string()));
        assert!(!remaining.contains(&deleted_wt.to_string_lossy().to_string()));
        assert!(!remaining.contains("/persistent/repo"));
        assert!(!remaining.contains(".tmp/verify/repo1"));

        let _ = fs::remove_dir_all(&dir);
    }
}
