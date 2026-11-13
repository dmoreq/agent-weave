//! Phase 5 hardened judge. `eg reuse --judge` asks a local `eg-judge` binary
//! for a verdict on gray-zone candidates. Missing binaries, model failures,
//! timeouts, malformed output, and unknown verdicts never alter the static
//! result; they add one stderr note.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::*;

pub(super) const GRAY_LOW: u64 = 45;
pub(super) const GRAY_HIGH: u64 = 75;
const MAX_JUDGED: usize = 5;
const TIMEOUT: Duration = Duration::from_millis(15_000);
const POLL: Duration = Duration::from_millis(10);
const MAX_INPUT: usize = 64 * 1024;
const MAX_OUTPUT: usize = 64 * 1024;

#[derive(Debug, Clone)]
struct Verdict {
    decision: VerdictDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerdictDecision {
    Duplicate,
    Distinct,
}

struct Setup {
    binary: PathBuf,
    model: PathBuf,
    subcommand: &'static str,
}

pub(super) fn apply(
    ctx: &QueryCtx<'_>,
    pool: &mut Vec<ReuseCandidate>,
    enabled: bool,
    mode: ReuseMode,
) -> Option<String> {
    if !enabled {
        return None;
    }
    let setup = match detect_setup() {
        Ok(setup) => setup,
        Err(note) => return Some(format!("eg: judge unavailable: {note}")),
    };
    let mut judged = 0usize;
    pool.retain(|candidate| {
        if judged >= MAX_JUDGED || !is_judgeable(candidate, mode) {
            return true;
        }
        judged += 1;
        match arbitrate(ctx, &setup, candidate) {
            Some(verdict) => verdict.decision != VerdictDecision::Distinct,
            None => true,
        }
    });
    crate::journal::set_judged(judged > 0);
    None
}

fn is_judgeable(candidate: &ReuseCandidate, mode: ReuseMode) -> bool {
    if mode == ReuseMode::Propose {
        return candidate.overlap_pct.is_some();
    }
    candidate
        .overlap_pct
        .is_some_and(|pct| (GRAY_LOW..=GRAY_HIGH).contains(&pct))
}

fn detect_setup() -> std::result::Result<Setup, String> {
    let binary = match std::env::var("EG_JUDGE_BIN") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => which("eg-judge")
            .ok_or_else(|| "eg-judge binary not found; set EG_JUDGE_BIN".to_string())?,
    };
    let model = match std::env::var("EG_JUDGE_MODEL") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => {
            let home = std::env::var("HOME")
                .map_err(|_| "HOME is unset; set EG_JUDGE_MODEL".to_string())?;
            PathBuf::from(home)
                .join(".engram")
                .join("judge")
                .join("model")
        }
    };
    if !model.join("lazy.json").exists() {
        return Err(format!(
            "no checkpoint at {}; set EG_JUDGE_MODEL",
            model.display()
        ));
    }
    let subcommand = match std::fs::read_to_string(model.join("lazy.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.get("head_layers").and_then(Value::as_u64))
    {
        Some(0) => "similarity",
        _ => "judge-duplicate",
    };
    Ok(Setup {
        binary,
        model,
        subcommand,
    })
}

fn which(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn arbitrate(ctx: &QueryCtx<'_>, setup: &Setup, candidate: &ReuseCandidate) -> Option<Verdict> {
    let left = source(ctx, &candidate.file, &candidate.symbol)?;
    let right = source(ctx, &candidate.canonical_file, &candidate.canonical)?;
    run_child(setup, &left, &right).map(|decision| Verdict { decision })
}

/// Source text of one symbol from its indexed span. The path must stay inside
/// the repository and may not be a symlink.
fn source(ctx: &QueryCtx<'_>, file: &str, name: &str) -> Option<String> {
    if !crate::query::staleness::safe_index_path(file) {
        return None;
    }
    let abs = ctx.root.join(file);
    let meta = std::fs::symlink_metadata(&abs).ok()?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return None;
    }
    let canon_root = std::fs::canonicalize(ctx.root).ok()?;
    let canon = std::fs::canonicalize(&abs).ok()?;
    if !canon.starts_with(&canon_root) {
        return None;
    }
    let (start, end): (i64, i64) = ctx
        .conn
        .query_row(
            "SELECT line_start, line_end FROM symbols \
             WHERE file_path = ?1 AND (name = ?2 OR qualname = ?2) \
             ORDER BY line_start LIMIT 1",
            rusqlite::params![file, name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok()?;
    let text = std::fs::read_to_string(&canon).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let start = start.max(1) as usize - 1;
    let end = end.max(start as i64 + 1) as usize;
    if start >= lines.len() {
        return None;
    }
    Some(lines[start..end.min(lines.len())].join("\n"))
}

fn run_child(setup: &Setup, left: &str, right: &str) -> Option<VerdictDecision> {
    let payload = if setup.subcommand == "similarity" {
        serde_json::json!({"text_a": left, "text_b": right})
    } else {
        serde_json::json!({"code_a": left, "code_b": right})
    };
    let payload = serde_json::to_vec(&payload).ok()?;
    if payload.len() > MAX_INPUT {
        return None;
    }
    let mut command = Command::new(&setup.binary);
    command
        .arg(setup.subcommand)
        .arg("--stdin")
        .arg("--model")
        .arg(&setup.model)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear();
    if let Some(home) = std::env::var_os("HOME") {
        command.env("HOME", home);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().ok()?;
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    if stdin.write_all(&payload).is_err() {
        kill_group(&child);
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    drop(stdin);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => {
                kill_group(&child);
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut raw = String::new();
    child
        .stdout
        .take()?
        .take((MAX_OUTPUT + 1) as u64)
        .read_to_string(&mut raw)
        .ok()?;
    if raw.len() > MAX_OUTPUT {
        return None;
    }
    parse_verdict(&raw)
}

#[cfg(unix)]
fn kill_group(child: &std::process::Child) {
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(format!("-{}", child.id()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(unix))]
fn kill_group(_child: &std::process::Child) {}

fn parse_verdict(raw: &str) -> Option<VerdictDecision> {
    let value: Value = serde_json::from_str(raw.trim()).ok()?;
    let decision = match value.get("decision")?.as_str()? {
        "duplicate" => VerdictDecision::Duplicate,
        "distinct" => VerdictDecision::Distinct,
        _ => return None,
    };
    let probability = value
        .get("probability")
        .or_else(|| value.get("similarity"))?
        .as_f64()?;
    if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
        return None;
    }
    Some(decision)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gray_band_is_inclusive() {
        let candidate = |pct| ReuseCandidate {
            detector: DetectorId::D1,
            symbol: "a".into(),
            file: "a.py".into(),
            line: 1,
            canonical: "b".into(),
            canonical_file: "b.py".into(),
            canonical_line: 1,
            canonical_callers: 0,
            overlap_pct: Some(pct),
            shared_items: Vec::new(),
            raw_client: None,
            confidence: ReuseConfidence::Low,
            reasons: Vec::new(),
            freshness: Freshness::Fresh,
        };
        assert!(!is_judgeable(&candidate(44), ReuseMode::Scan));
        assert!(is_judgeable(&candidate(45), ReuseMode::Scan));
        assert!(is_judgeable(&candidate(75), ReuseMode::Scan));
        assert!(!is_judgeable(&candidate(76), ReuseMode::Scan));
        assert!(is_judgeable(&candidate(100), ReuseMode::Propose));
    }

    #[test]
    fn verdict_whitelist_and_probability_range() {
        assert_eq!(
            parse_verdict(r#"{"decision":"duplicate","probability":0.8}"#),
            Some(VerdictDecision::Duplicate)
        );
        assert_eq!(
            parse_verdict(r#"{"decision":"distinct","similarity":0.2}"#),
            Some(VerdictDecision::Distinct)
        );
        assert_eq!(
            parse_verdict(r#"{"decision":"maybe","probability":0.8}"#),
            None
        );
        assert_eq!(
            parse_verdict(r#"{"decision":"duplicate","probability":1.2}"#),
            None
        );
    }
}
