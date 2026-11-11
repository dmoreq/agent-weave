//! Bounded per-generation journal reads. `audit` treats an `Error` as fatal;
//! the human summaries ignore it, preserving their historical behavior.

use std::io::Read;
use std::path::{Path, PathBuf};

/// Bounded reader for one journal generation: reject non-regular files and
/// cap allocation at 16 MiB even when the file is oversized or growing.
const MAX_LOG_READ: u64 = 16 * 1024 * 1024;

/// Per-generation read outcome. `Absent` is normal (the file was never
/// written); `Error` is an I/O, symlink, or non-regular-file failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GenerationStatus {
    Absent,
    Read,
    Error,
}

pub(crate) struct GenerationRead {
    pub(crate) name: &'static str,
    pub(crate) status: GenerationStatus,
    pub(crate) text: String,
    pub(crate) bytes: u64,
    pub(crate) truncated: bool,
}

pub(crate) struct UsageLog {
    pub(crate) generations: Vec<GenerationRead>,
}

/// Read `.1` then the current generation separately.
pub(crate) fn read_usage_log(base: &Path) -> UsageLog {
    let mut generations = Vec::with_capacity(2);
    for (name, suffix) in [("usage.log.1", ".1"), ("usage.log", "")] {
        let mut path = base.to_path_buf().into_os_string();
        path.push(suffix);
        let path = PathBuf::from(path);
        let mut gen = GenerationRead {
            name,
            status: GenerationStatus::Absent,
            text: String::new(),
            bytes: 0,
            truncated: false,
        };
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if !meta.is_file() => gen.status = GenerationStatus::Error,
            Ok(_) => match std::fs::File::open(&path) {
                Ok(file) => {
                    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
                    let mut reader = file.take(MAX_LOG_READ);
                    let mut bytes = Vec::new();
                    match reader.read_to_end(&mut bytes) {
                        Ok(_) => {
                            gen.status = GenerationStatus::Read;
                            gen.bytes = bytes.len() as u64;
                            gen.truncated = len > MAX_LOG_READ;
                            gen.text = String::from_utf8_lossy(&bytes).into_owned();
                        }
                        Err(_) => gen.status = GenerationStatus::Error,
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => gen.status = GenerationStatus::Error,
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => gen.status = GenerationStatus::Error,
        }
        generations.push(gen);
    }
    UsageLog { generations }
}
