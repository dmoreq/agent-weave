//! Optional semantic arbitration of gray-zone reuse candidates by a local `eg-laya` binary.
//!
//! The static detectors are precise at the extremes and blind in the middle band. With
//! `eg reuse --judge`, candidates whose overlap sits in 45-75% are sent to `eg-laya`, which
//! answers from either a proposition head (`judge-duplicate`) or a bi-encoder cosine
//! (`similarity`). Every failure — missing binary, missing model, timeout, malformed output —
//! leaves the static verdict untouched and surfaces one stderr note.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::Value;

use crate::query::reuse::ReuseCandidate;

/// Overlap band the static detectors cannot resolve (plan section 1.1).
pub const GRAY_LOW: u64 = 45;
pub const GRAY_HIGH: u64 = 75;
/// One model call costs a process spawn plus a checkpoint load, so cap the work per run.
pub const MAX_JUDGED: usize = 5;
const TIMEOUT: Duration = Duration::from_millis(15_000);
const POLL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone)]
pub struct Verdict {
    pub decision: String,
    pub probability: f32,
    pub mode: &'static str,
}

/// Where the model lives and which subcommand answers it.
pub struct Setup {
    binary: PathBuf,
    model: PathBuf,
    subcommand: &'static str,
}

pub fn is_gray(overlap_pct: u64) -> bool {
    (GRAY_LOW..=GRAY_HIGH).contains(&overlap_pct)
}

/// Resolve the binary and model, or explain once why the judge is unavailable.
pub fn detect_setup() -> Result<Setup, String> {
    let binary = match std::env::var("EG_LAYA_BIN") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => which("eg-laya").ok_or_else(|| {
            "eg-laya binary not found; set EG_LAYA_BIN or add it to PATH".to_string()
        })?,
    };
    let model = match std::env::var("EG_LAYA_MODEL") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => {
            let home = std::env::var("HOME")
                .map_err(|_| "HOME is unset; set EG_LAYA_MODEL".to_string())?;
            PathBuf::from(home)
                .join(".engram")
                .join("laya")
                .join("model")
        }
    };
    if !model.join("lazy.json").exists() {
        return Err(format!(
            "no checkpoint at {}; set EG_LAYA_MODEL",
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

/// Arbitrate one candidate. `None` means "keep the static verdict".
pub fn arbitrate(
    setup: &Setup,
    conn: &Connection,
    root: &Path,
    candidate: &ReuseCandidate,
) -> Option<Verdict> {
    let left = source(conn, root, &candidate.file, &candidate.symbol)?;
    let right = source(conn, root, &candidate.canonical_file, &candidate.canonical)?;
    let (decision, probability) = run_child(setup, &left, &right)?;
    Some(Verdict {
        decision,
        probability,
        mode: setup.subcommand,
    })
}

/// Source text of one symbol, from its indexed line span.
fn source(conn: &Connection, root: &Path, file: &str, name: &str) -> Option<String> {
    let (start, end): (i64, i64) = conn
        .query_row(
            "SELECT line_start, line_end FROM symbols \
             WHERE file_path = ?1 AND (name = ?2 OR qualname = ?2) \
             ORDER BY line_start LIMIT 1",
            rusqlite::params![file, name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok()?;
    let text = std::fs::read_to_string(root.join(file)).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let start = start.max(1) as usize - 1;
    let end = end.max(start as i64 + 1) as usize;
    if start >= lines.len() {
        return None;
    }
    Some(lines[start..end.min(lines.len())].join("\n"))
}

fn run_child(setup: &Setup, left: &str, right: &str) -> Option<(String, f32)> {
    let (flag_a, flag_b) = if setup.subcommand == "similarity" {
        ("--text-a", "--text-b")
    } else {
        ("--code-a", "--code-b")
    };
    let mut child = Command::new(&setup.binary)
        .arg(setup.subcommand)
        .arg(flag_a)
        .arg(left)
        .arg(flag_b)
        .arg(right)
        .arg("--model")
        .arg(&setup.model)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut raw = String::new();
    child.stdout.take()?.read_to_string(&mut raw).ok()?;
    parse_verdict(&raw)
}

/// `eg-laya` prints one JSON line; both subcommands carry `decision` plus either
/// `probability` (proposition head) or `similarity` (bi-encoder cosine).
pub(crate) fn parse_verdict(raw: &str) -> Option<(String, f32)> {
    let value: Value = serde_json::from_str(raw.trim()).ok()?;
    let decision = value.get("decision")?.as_str()?.to_string();
    let probability = value
        .get("probability")
        .or_else(|| value.get("similarity"))?
        .as_f64()? as f32;
    Some((decision, probability))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gray_band_is_inclusive() {
        assert!(!is_gray(44));
        assert!(is_gray(45));
        assert!(is_gray(60));
        assert!(is_gray(75));
        assert!(!is_gray(76));
    }

    #[test]
    fn parses_both_subcommand_shapes() {
        let judge =
            parse_verdict(r#"{"decision":"duplicate","probability":0.83,"calibrated":false}"#);
        assert_eq!(judge, Some(("duplicate".to_string(), 0.83)));
        let cosine = parse_verdict(r#"{"decision":"distinct","similarity":0.41,"threshold":0.5}"#);
        assert_eq!(cosine, Some(("distinct".to_string(), 0.41)));
    }

    #[test]
    fn malformed_output_is_ignored() {
        assert_eq!(parse_verdict("not json"), None);
        assert_eq!(parse_verdict(r#"{"decision":"duplicate"}"#), None);
    }
}
