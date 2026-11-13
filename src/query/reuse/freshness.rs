//! Per-pair and scope-level freshness for `eg reuse`. Stats are cheap; a
//! differing mtime triggers a bounded FNV-1a re-hash under one 16 MiB budget
//! per invocation. A missing index row is `unknown`, never silently fresh.

use std::collections::{HashMap, HashSet};

use rusqlite::OptionalExtension;

use super::status::ResolvedScope;
use super::*;

const HASH_BUDGET: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) struct CheckResult {
    state: Freshness,
    reason: Option<UnknownReason>,
}

struct HashJob {
    path: std::path::PathBuf,
    expected: i64,
}

enum Prepared {
    Ready(CheckResult),
    Hash(HashJob),
}

#[derive(Default)]
pub(crate) struct FreshnessReport {
    pub(super) state: Freshness,
    pub(super) changed_files: usize,
    pub(super) checked_files: usize,
    pub(super) unknown_count: usize,
    pub(super) unknown_reason: Option<UnknownReason>,
}

impl FreshnessReport {
    pub(super) fn headers(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.changed_files > 0 {
            out.push(format!(
                "eg: {} of {} result files changed since build — run eg build",
                self.changed_files, self.checked_files
            ));
        }
        if self.unknown_count > 0 {
            out.push(format!(
                "eg: freshness unknown ({}) — run eg build",
                self.unknown_reason
                    .map(|r| r.as_str())
                    .unwrap_or(UnknownReason::NewFile.as_str())
            ));
        }
        out
    }
}

struct Checker<'a> {
    ctx: &'a QueryCtx<'a>,
    remaining_bytes: u64,
}

impl<'a> Checker<'a> {
    fn new(ctx: &'a QueryCtx<'a>) -> Self {
        Self {
            ctx,
            remaining_bytes: HASH_BUDGET,
        }
    }

    #[cfg(test)]
    fn check(&mut self, rel: &str) -> Result<CheckResult> {
        match self.prepare(rel)? {
            Prepared::Ready(result) => Ok(result),
            Prepared::Hash(job) => Ok(hash_job(&job)),
        }
    }

    fn prepare(&mut self, rel: &str) -> Result<Prepared> {
        if !crate::query::staleness::safe_index_path(rel) {
            return Ok(Prepared::Ready(stale(UnknownReason::UnsafePath)));
        }
        let stored = self
            .ctx
            .conn
            .query_row(
                "SELECT mtime, size, content_hash FROM files WHERE path = ?1",
                [rel],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let abs = self.ctx.root.join(rel);
        let meta = match std::fs::symlink_metadata(&abs) {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Prepared::Ready(if stored.is_some() {
                    stale(UnknownReason::DeletedFile)
                } else {
                    fresh()
                }));
            }
            Err(_) => return Ok(Prepared::Ready(stale(UnknownReason::StatError))),
        };
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Ok(Prepared::Ready(stale(UnknownReason::UnsafePath)));
        }
        let Some((stored_mtime, stored_size, stored_hash)) = stored else {
            return Ok(Prepared::Ready(unknown(UnknownReason::NewFile)));
        };
        if meta.len() != stored_size as u64 {
            return Ok(Prepared::Ready(stale_without_reason()));
        }
        let Some(mtime) = crate::db::stat_mtime(&abs) else {
            return Ok(Prepared::Ready(stale(UnknownReason::StatError)));
        };
        if mtime == stored_mtime {
            return Ok(Prepared::Ready(fresh()));
        }
        if self.remaining_bytes < stored_size as u64 {
            return Ok(Prepared::Ready(unknown(UnknownReason::HashBudget)));
        }
        self.remaining_bytes -= stored_size as u64;
        Ok(Prepared::Hash(HashJob {
            path: abs,
            expected: stored_hash,
        }))
    }
}

fn hash_job(job: &HashJob) -> CheckResult {
    let Ok(bytes) = crate::walk::read_bounded(&job.path) else {
        return stale(UnknownReason::StatError);
    };
    if crate::hash::fnv1a64(&bytes) as i64 == job.expected {
        fresh()
    } else {
        stale_without_reason()
    }
}

fn fresh() -> CheckResult {
    CheckResult {
        state: Freshness::Fresh,
        reason: None,
    }
}

fn stale(reason: UnknownReason) -> CheckResult {
    CheckResult {
        state: Freshness::Stale,
        reason: Some(reason),
    }
}

fn stale_without_reason() -> CheckResult {
    CheckResult {
        state: Freshness::Stale,
        reason: None,
    }
}

fn unknown(reason: UnknownReason) -> CheckResult {
    CheckResult {
        state: Freshness::Unknown,
        reason: Some(reason),
    }
}

fn reason_rank(reason: UnknownReason) -> u8 {
    match reason {
        UnknownReason::WalkError => 5,
        UnknownReason::NewFile => 4,
        UnknownReason::HashBudget => 3,
        UnknownReason::UnsafePath => 2,
        UnknownReason::StatError => 1,
        UnknownReason::DeletedFile => 0,
    }
}

/// Check the pool (or the empty scope, including a new-file walk), then apply
/// pair aggregates and reasons to each candidate.
pub(super) fn evaluate(
    ctx: &QueryCtx<'_>,
    scope: &ResolvedScope,
    candidates: &mut [ReuseCandidate],
) -> Result<FreshnessReport> {
    let mut checker = Checker::new(ctx);
    let mut results: HashMap<String, CheckResult> = HashMap::new();
    let files = if candidates.is_empty() {
        scope_files(ctx, scope)?
    } else {
        candidate_files(candidates)
    };
    let mut prepared = Vec::with_capacity(files.len());
    let mut seen = HashSet::new();
    for file in files {
        if !seen.insert(file.clone()) {
            continue;
        }
        prepared.push((file.clone(), checker.prepare(&file)?));
    }
    let mut outputs: Vec<Option<CheckResult>> = vec![None; prepared.len()];
    let mut jobs = Vec::new();
    for (index, (file, prepared)) in prepared.iter().enumerate() {
        if let Prepared::Hash(job) = prepared {
            jobs.push((index, file, job));
        }
    }
    for (index, (_, prepared)) in prepared.iter().enumerate() {
        if let Prepared::Ready(result) = prepared {
            outputs[index] = Some(*result);
        }
    }
    if !jobs.is_empty() {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(4)
            .min(jobs.len());
        if workers <= 1 {
            for (index, _, job) in &jobs {
                outputs[*index] = Some(hash_job(job));
            }
        } else {
            let chunk_size = jobs.len().div_ceil(workers);
            std::thread::scope(|scope| {
                let mut handles = Vec::new();
                for chunk in jobs.chunks(chunk_size) {
                    handles.push(scope.spawn(move || {
                        chunk
                            .iter()
                            .map(|(index, _, job)| (*index, hash_job(job)))
                            .collect::<Vec<_>>()
                    }));
                }
                for handle in handles {
                    for (index, result) in handle.join().expect("freshness hash worker") {
                        outputs[index] = Some(result);
                    }
                }
            });
        }
    }
    for (index, (file, _)) in prepared.into_iter().enumerate() {
        results.insert(file, outputs[index].expect("prepared freshness result"));
    }

    let mut report = FreshnessReport::default();
    if candidates.is_empty() {
        if let Some(reason) = new_file_walk(ctx, scope, &results, &mut report)? {
            if report
                .unknown_reason
                .is_none_or(|cur| reason_rank(reason) > reason_rank(cur))
            {
                report.unknown_reason = Some(reason);
            }
        }
    }

    for result in results.values() {
        match result.state {
            Freshness::Fresh => report.checked_files += 1,
            Freshness::Stale => {
                report.checked_files += 1;
                report.changed_files += 1;
            }
            Freshness::Unknown => {
                report.unknown_count += 1;
                if result.reason.is_some_and(|r| {
                    report
                        .unknown_reason
                        .is_none_or(|cur| reason_rank(r) > reason_rank(cur))
                }) {
                    report.unknown_reason = result.reason;
                }
            }
        }
    }
    report.state = if report.unknown_count > 0 {
        Freshness::Unknown
    } else if report.changed_files > 0 {
        Freshness::Stale
    } else {
        Freshness::Fresh
    };

    for candidate in candidates.iter_mut() {
        let mut state = Freshness::Fresh;
        let files = [candidate.file.clone(), candidate.canonical_file.clone()];
        for file in files {
            let Some(result) = results.get(&file) else {
                continue;
            };
            if result.state == Freshness::Unknown {
                state = Freshness::Unknown;
            } else if result.state == Freshness::Stale && state == Freshness::Fresh {
                state = Freshness::Stale;
            }
            if result.reason == Some(UnknownReason::UnsafePath) {
                push_reason(candidate, ReasonCode::UnsafePath);
            }
        }
        if state == Freshness::Unknown {
            candidate.freshness = Freshness::Unknown;
            push_reason(candidate, ReasonCode::FreshnessUnknown);
        } else if state == Freshness::Stale {
            candidate.freshness = Freshness::Stale;
            push_reason(candidate, ReasonCode::StaleIndex);
        }
        if state != Freshness::Fresh {
            // Stale or unknown evidence never carries Medium/High authority.
            candidate.confidence = ReuseConfidence::Low;
        }
    }
    Ok(report)
}

fn push_reason(candidate: &mut ReuseCandidate, reason: ReasonCode) {
    if !candidate.reasons.contains(&reason) {
        candidate.reasons.push(reason);
        candidate.reasons.sort_by_key(|r| r.as_str());
    }
}

fn candidate_files(candidates: &[ReuseCandidate]) -> Vec<String> {
    let mut files = HashSet::new();
    for candidate in candidates {
        files.insert(candidate.file.clone());
        files.insert(candidate.canonical_file.clone());
    }
    let mut out: Vec<String> = files.into_iter().collect();
    out.sort();
    out
}

fn scope_files(ctx: &QueryCtx<'_>, scope: &ResolvedScope) -> Result<Vec<String>> {
    if scope.is_repo() {
        let mut stmt = ctx.conn.prepare("SELECT path FROM files ORDER BY path")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        return Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?);
    }
    Ok(scope.files.clone())
}

/// A source file on disk but absent from the index is `unknown/new-file`.
/// The walk is scope-limited and only runs for an empty candidate pool.
fn new_file_walk(
    ctx: &QueryCtx<'_>,
    scope: &ResolvedScope,
    results: &HashMap<String, CheckResult>,
    report: &mut FreshnessReport,
) -> Result<Option<UnknownReason>> {
    let indexed: HashSet<String> = {
        let mut stmt = ctx.conn.prepare("SELECT path FROM files")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<HashSet<_>>>()?
    };
    let stats = match crate::walk::walk(ctx.root, &crate::walk::source_exts()) {
        Ok(stats) => stats,
        Err(_) => return Ok(Some(UnknownReason::WalkError)),
    };
    let mut unknown_files = HashSet::new();
    for file in stats.files {
        let rel = file.rel;
        if !in_scope(&rel, scope)
            || indexed.contains(&rel)
            || rel.starts_with(".git/")
            || rel.starts_with(".engram/")
            || rel.starts_with(".tmp/")
            || rel.starts_with("target/")
            || results.contains_key(&rel)
        {
            continue;
        }
        unknown_files.insert(rel);
    }
    if unknown_files.is_empty() {
        return Ok(None);
    }
    report.unknown_count += unknown_files.len();
    Ok(Some(UnknownReason::NewFile))
}

fn in_scope(path: &str, scope: &ResolvedScope) -> bool {
    scope.is_repo() || super::target::path_matches(path, &scope.paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> &'static std::path::Path {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn hash_budget_returns_unknown_instead_of_stale() {
        let root = repo_root();
        let size = std::fs::metadata(root.join("Cargo.toml")).unwrap().len() as i64;
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE files (path TEXT, mtime INTEGER, size INTEGER, content_hash INTEGER);",
        )
        .unwrap();
        conn.execute("INSERT INTO files VALUES ('Cargo.toml', 0, ?1, 0)", [size])
            .unwrap();
        let ctx = QueryCtx { conn: &conn, root };
        let mut checker = Checker::new(&ctx);
        checker.remaining_bytes = 0;
        let result = checker.check("Cargo.toml").unwrap();
        assert_eq!(result.state, Freshness::Unknown);
        assert_eq!(result.reason, Some(UnknownReason::HashBudget));
    }

    #[test]
    fn missing_files_row_but_existing_file_is_new_file() {
        let root = repo_root();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE files (path TEXT, mtime INTEGER, size INTEGER, content_hash INTEGER);",
        )
        .unwrap();
        let ctx = QueryCtx { conn: &conn, root };
        let mut checker = Checker::new(&ctx);
        let result = checker.check("Cargo.toml").unwrap();
        assert_eq!(result.state, Freshness::Unknown);
        assert_eq!(result.reason, Some(UnknownReason::NewFile));
    }
}
