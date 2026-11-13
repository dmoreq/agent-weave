//! Data aggregation engine for the native dashboard.
//! Queries `.engram/usage.log`, `.engram/index.db`, and `.engram/memory.db` with sub-millisecond latency.

pub(crate) mod memories;
mod reuse;
mod summary;

pub use memories::{collect_memories, resolve_store, store_ids};
pub use reuse::collect_reuse_candidates;
pub use summary::compute_summary;
pub(crate) use summary::{classify, ToolClass};

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Discover candidate repo roots for a dashboard panel into `repos`. An
/// explicit root short-circuits the scan (its entry replaces any same-named
/// one); otherwise every journaled repo with `.engram/<db_rel>` present is added
/// without displacing existing entries, and the current directory joins last
/// under its own name. Shared by the reuse and memories panels so their store
/// discovery cannot drift.
fn discover_repos(
    root: Option<&Path>,
    entries: &[Value],
    db_rel: &str,
    repos: &mut HashMap<String, PathBuf>,
) {
    if let Some(r) = root {
        let db_path = r.join(".engram").join(db_rel);
        if db_path.is_file() {
            let name = r
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(crate::repo::UNATTRIBUTED);
            repos.insert(name.to_string(), r.to_path_buf());
        }
    } else {
        for e in entries {
            if let (Some(r), Some(r_name)) = (
                e.get("repo").and_then(Value::as_str),
                e.get("repo_name").and_then(Value::as_str),
            ) {
                let p = PathBuf::from(r);
                if p.join(".engram").join(db_rel).is_file() && !repos.contains_key(r_name) {
                    repos.insert(r_name.to_string(), p);
                }
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            let name = cwd
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(crate::repo::UNATTRIBUTED);
            if cwd.join(".engram").join(db_rel).is_file() && !repos.contains_key(name) {
                repos.insert(name.to_string(), cwd);
            }
        }
    }
}

/// Drop repos whose `.engram/` is git-tracked and return the security errors
/// for the panel payload. A tracked repo must not scan as a clean zero-result.
fn retain_untracked(repos: &mut HashMap<String, PathBuf>) -> (Vec<Value>, usize) {
    let mut errors = Vec::new();
    let mut blocked = 0usize;
    repos.retain(|name, path| {
        let gate = crate::db::reject_symlink(&path.join(".engram"), ".engram directory")
            .and_then(|()| crate::db::reject_tracked_eg_dir(path));
        match gate {
            Ok(()) => true,
            Err(err) => {
                blocked += 1;
                errors.push(serde_json::json!({
                    "repo": name,
                    "stage": "security",
                    "message": err.to_string(),
                }));
                false
            }
        }
    });
    (errors, blocked)
}

pub struct AggregatedData {
    pub summary: Value,
    pub reuse: Value,
    pub memories: Value,
}

/// The single choke point every journal reader goes through: `None` when the
/// line is not valid JSON or does not name a live repository — dead roots,
/// identityless legacy lines, and relative test artifacts all fail the same
/// one predicate. `read_usage_entries`, the SSE stream, and the monitor all
/// call it, so nothing without a live repository can reach any panel or feed.
/// One journal line is capped before parsing: a corrupted or hostile writer
/// must not force a multi-megabyte parse on every panel request.
const MAX_LINE_BYTES: usize = 256 * 1024;

pub fn live_event(trimmed: &str) -> Option<Value> {
    if trimmed.len() > MAX_LINE_BYTES {
        return None;
    }
    let val = serde_json::from_str::<Value>(trimmed).ok()?;
    crate::journal::liveness::repo_is_live(val.get("repo").and_then(Value::as_str)).then_some(val)
}

fn generation_stamp(path: &Path) -> (u64, u64) {
    match std::fs::metadata(path) {
        Ok(meta) => {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            (mtime, meta.len())
        }
        Err(_) => (0, 0),
    }
}

/// Cached parse keyed by both generations' (mtime, length). The three API
/// routes each call this, often within the same second; a burst of appends
/// still costs one parse per distinct stamp.
fn read_entries_cached(path: &Path) -> Vec<Value> {
    use std::sync::{Mutex, OnceLock};
    struct EntryCache {
        stamp: ((u64, u64), (u64, u64)),
        entries: Vec<Value>,
    }
    static CACHE: OnceLock<Mutex<Option<EntryCache>>> = OnceLock::new();
    let cell = CACHE.get_or_init(|| Mutex::new(None));
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let rotated = parent.join("usage.log.1");
    let stamp = (generation_stamp(&rotated), generation_stamp(path));
    if let Ok(guard) = cell.lock() {
        if let Some(cached) = guard.as_ref().filter(|c| c.stamp == stamp) {
            return cached.entries.clone();
        }
    }
    let entries = read_usage_entries_uncached(path);
    // Re-check: a write during the parse would otherwise be pinned under a
    // stale-but-matching stamp.
    let after = (generation_stamp(&rotated), generation_stamp(path));
    if after == stamp {
        if let Ok(mut guard) = cell.lock() {
            *guard = Some(EntryCache {
                stamp,
                entries: entries.clone(),
            });
        }
    }
    entries
}

pub fn read_usage_entries(log_path: &Path) -> Vec<Value> {
    if std::env::var("ENGRAM_DASHBOARD_NO_CACHE").as_deref() == Ok("1") {
        return read_usage_entries_uncached(log_path);
    }
    read_entries_cached(log_path)
}

fn read_usage_entries_uncached(log_path: &Path) -> Vec<Value> {
    let mut entries = Vec::new();
    let parent = log_path.parent().unwrap_or_else(|| Path::new("."));

    for filename in &["usage.log.1", "usage.log"] {
        let p = parent.join(filename);
        if let Ok(file) = File::open(&p) {
            let reader = BufReader::new(file);
            for line in reader.lines().map_while(std::result::Result::ok) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(val) = live_event(trimmed) {
                    entries.push(val);
                }
            }
        }
    }
    entries
}

/// Read one line window without materializing the whole file: skip to the
/// start line, then take at most `max_lines` lines.
pub fn read_snippet(file_path: &Path, line_start: i64, line_end: i64, max_lines: usize) -> String {
    if !file_path.is_file() {
        return String::new();
    }
    let Ok(file) = File::open(file_path) else {
        return String::new();
    };
    let reader = BufReader::new(file);
    let start = (line_start.max(1) - 1) as usize;
    let end = (line_end.max(line_start) as usize).max(start + 1);
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for line in reader.lines().map_while(std::result::Result::ok) {
        if skipped < start {
            skipped += 1;
            continue;
        }
        if out.len() >= max_lines {
            out.push(format!(
                " ... ({} more lines)",
                end.saturating_sub(start + out.len())
            ));
            break;
        }
        if start + out.len() >= end {
            break;
        }
        out.push(format!("{:4} | {}", start + out.len() + 1, line));
    }
    out.join("\n")
}

/// One incremental journal tail shared by the SSE stream and both monitor
/// paths. Tracks file identity (device, inode) so a rotation or an in-place
/// prune is a `Reset`, never a silent replay: the caller reloads its state
/// instead of re-counting lines it already served.
pub struct Tail {
    pos: u64,
    identity: Option<(u64, u64)>,
}

/// One poll result: a complete journal line, or a reset signal.
pub enum TailEvent<'a> {
    Line(&'a str),
    Reset,
}

fn file_identity(meta: &std::fs::Metadata) -> (u64, u64) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (meta.dev(), meta.ino())
    }
    #[cfg(not(unix))]
    {
        (0, meta.len())
    }
}

impl Tail {
    /// Start at the current end of the journal: only new lines are delivered.
    pub fn new(path: &Path) -> Self {
        match std::fs::metadata(path) {
            Ok(meta) => Self {
                pos: meta.len(),
                identity: Some(file_identity(&meta)),
            },
            Err(_) => Self {
                pos: 0,
                identity: None,
            },
        }
    }

    /// Start at an explicit byte offset (tests and replay-on-demand paths).
    pub fn starting_at(path: &Path, pos: u64) -> Self {
        let identity = std::fs::metadata(path).ok().map(|m| file_identity(&m));
        Self { pos, identity }
    }

    pub fn position(&self) -> u64 {
        self.pos
    }

    /// Deliver every complete new line. A shrink or identity change yields one
    /// `Reset` and advances to the new end (no replay). A torn trailing line
    /// stays unread until its newline arrives.
    pub fn poll(&mut self, path: &Path, mut on: impl FnMut(TailEvent<'_>)) {
        let Ok(meta) = std::fs::metadata(path) else {
            return;
        };
        let id = file_identity(&meta);
        if self.identity != Some(id) || meta.len() < self.pos {
            on(TailEvent::Reset);
            self.pos = meta.len();
            self.identity = Some(id);
            return;
        }
        if meta.len() == self.pos {
            return;
        }
        let Ok(file) = File::open(path) else {
            return;
        };
        let mut reader = BufReader::new(file);
        if reader.seek(SeekFrom::Start(self.pos)).is_err() {
            return;
        }
        let mut start = self.pos;
        let mut buf = String::new();
        loop {
            buf.clear();
            let Ok(n) = reader.read_line(&mut buf) else {
                return;
            };
            if n == 0 || !buf.ends_with('\n') {
                // Incomplete tail: leave the offset before it so the next
                // poll reads the whole line once its writer finishes.
                break;
            }
            let consumed = n as u64;
            let trimmed = buf.trim();
            if !trimmed.is_empty() {
                on(TailEvent::Line(trimmed));
            }
            start += consumed;
            self.pos = start;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn live_event_keeps_only_live_repo_entries() {
        let live_root = crate::db::test_support::temp_root("dashboard-live-event");
        let live = live_root.to_string_lossy().into_owned();
        let kept = live_event(
            json!({ "tool": "build", "repo": live, "repo_name": "live-repo" })
                .to_string()
                .as_str(),
        )
        .unwrap();
        assert_eq!(kept["repo"], live);
        assert_eq!(kept["repo_name"], "live-repo");

        // Dead absolute root, legacy relative spelling, identityless line, and
        // garbage: all dropped by the same predicate (no unattributed bucket).
        for line in [
            json!({ "tool": "build", "repo": "/nonexistent-eg-dash/dead", "repo_name": "agent-skills" }).to_string(),
            json!({ "tool": "build", "repo": ".tmp/verify/repo1", "repo_name": "repo1" }).to_string(),
            json!({ "tool": "build" }).to_string(),
            "not json".to_string(),
        ] {
            assert!(live_event(&line).is_none(), "{line}");
        }
        crate::db::test_support::remove_tree(&live_root);
    }
}

#[cfg(test)]
mod tail_tests {
    use super::*;
    use crate::db::test_support::{append_file, remove_tree, temp_root, write_file};

    #[test]
    fn tail_delivers_new_lines_and_signals_reset_on_shrink() {
        let dir = temp_root("dashboard-tail");
        let log = dir.join("usage.log");
        write_file(&log, b"{\"a\":1}\n").unwrap();
        let mut tail = Tail::new(&log);

        let mut lines: Vec<String> = Vec::new();
        tail.poll(&log, |e| {
            if let TailEvent::Line(l) = e {
                lines.push(l.to_string())
            }
        });
        assert!(lines.is_empty(), "start at EOF: no backlog");

        append_file(&log, b"{\"b\":2}\n").unwrap();
        tail.poll(&log, |e| {
            if let TailEvent::Line(l) = e {
                lines.push(l.to_string())
            }
        });
        assert_eq!(lines, vec!["{\"b\":2}"]);

        // In-place prune: same inode, smaller file -> Reset, no replay.
        write_file(&log, b"{\"c\":3}\n").unwrap();
        let mut resets = 0;
        let mut after: Vec<String> = Vec::new();
        tail.poll(&log, |e| match e {
            TailEvent::Reset => resets += 1,
            TailEvent::Line(l) => after.push(l.to_string()),
        });
        assert_eq!(resets, 1);
        assert!(after.is_empty(), "no replay after reset");
        remove_tree(&dir);
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn oversized_journal_lines_are_dropped_before_parsing() {
        let huge = format!(
            "{{\"tool\":\"build\",\"repo\":\"/x\",\"pad\":\"{}\"}}",
            "a".repeat(300 * 1024)
        );
        assert!(live_event(&huge).is_none());
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use crate::db::test_support::{append_file, remove_tree, temp_root, write_file};

    #[test]
    fn journal_parse_is_cached_until_the_stamp_changes() {
        let dir = temp_root("dashboard-entries-cache");
        let log = dir.join("usage.log");
        let repo = dir.to_string_lossy().into_owned();
        write_file(
            &log,
            format!("{{\"tool\":\"build\",\"repo\":\"{repo}\",\"repo_name\":\"r\"}}\n").as_bytes(),
        )
        .unwrap();
        assert_eq!(read_usage_entries(&log).len(), 1);
        // Second read hits the cache; an append changes the stamp and re-parses.
        assert_eq!(read_usage_entries(&log).len(), 1);
        append_file(
            &log,
            format!("{{\"tool\":\"build\",\"repo\":\"{repo}\",\"repo_name\":\"r\"}}\n").as_bytes(),
        )
        .unwrap();
        assert_eq!(read_usage_entries(&log).len(), 2);
        remove_tree(&dir);
    }

    #[test]
    fn retain_untracked_reports_security_errors() {
        let dir = temp_root("dashboard-retain-tracked");
        let repo = dir.join("repo");
        crate::db::test_support::make_dir_all(repo.join(".engram")).unwrap();
        write_file(repo.join(".engram/index.db"), b"x").unwrap();
        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["init", "-q"])
            .status();
        if !matches!(init, Ok(status) if status.success()) {
            remove_tree(&dir);
            return;
        }
        let added = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "-f", ".engram/index.db"])
            .status()
            .unwrap();
        assert!(added.success());
        let mut repos = HashMap::new();
        repos.insert("repo".to_string(), repo);
        let (errors, blocked) = retain_untracked(&mut repos);
        assert!(repos.is_empty());
        assert_eq!(blocked, 1);
        assert_eq!(errors[0]["stage"], "security");
        remove_tree(&dir);
    }
}
