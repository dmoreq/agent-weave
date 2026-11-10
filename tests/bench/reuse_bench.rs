//! C1 gate benchmark: end-to-end `eg reuse` latency on a 5,000-symbol repo.
//!
//! Ignored by default because it generates a corpus and runs the release
//! binary. Run the gate with:
//!
//! ```sh
//! cargo build --release
//! cargo test --test reuse_bench -- --ignored --nocapture
//! ```
//!
//! The corpus has 500 duplicate pairs (1,000 pipeline symbols) over 1,500
//! shared helpers plus 2,500 filler symbols, so the inverted index has
//! non-trivial posting lists while the stoplist keeps them small.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const FILES: usize = 500;
const WARMUP_RUNS: usize = 10;
const MEASURED_RUNS: usize = 50;

fn binary() -> PathBuf {
    let path = std::env::var("EG_BENCH_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target/release/eg"));
    assert!(
        path.exists(),
        "release binary {} missing; run `cargo build --release` or set EG_BENCH_BIN",
        path.display()
    );
    path
}

fn write_corpus(root: &Path) {
    let mut common = String::new();
    for group in 0..FILES {
        for helper in 0..3 {
            common.push_str(&format!("def h{group}_{helper}():\n    return {helper}\n"));
        }
    }
    let mut filler = String::new();
    for i in 0..2_500 {
        filler.push_str(&format!("def filler_{i}():\n    return {i}\n"));
    }
    fs::write(root.join("common.py"), common).unwrap();
    fs::write(root.join("filler.py"), filler).unwrap();
    for group in 0..FILES {
        let source = format!(
            "from common import h{group}_0, h{group}_1, h{group}_2\n\
def pipeline_{group}_a():\n    return h{group}_0() + h{group}_1() + h{group}_2()\n\
def pipeline_{group}_b():\n    return h{group}_0() + h{group}_1() + h{group}_2()\n"
        );
        fs::write(root.join(format!("g{group}.py")), source).unwrap();
    }
}

fn run(eg: &Path, root: &Path, extra: &[&str]) -> Duration {
    let start = Instant::now();
    let out = Command::new(eg)
        .arg("--root")
        .arg(root)
        .arg("--no-journal")
        .args(["reuse", "--limit", "100"])
        .args(extra)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "reuse failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    start.elapsed()
}

fn percentile(mut samples: Vec<Duration>, fraction: f64) -> (Duration, Duration) {
    samples.sort_unstable();
    let index = ((samples.len() as f64) * fraction).ceil() as usize;
    let p = samples[index.saturating_sub(1).min(samples.len() - 1)];
    (samples[samples.len() / 2], p)
}

fn measure(eg: &Path, root: &Path, extra: &[&str]) -> (Duration, Duration) {
    for _ in 0..WARMUP_RUNS {
        run(eg, root, extra);
    }
    let samples = (0..MEASURED_RUNS).map(|_| run(eg, root, extra)).collect();
    percentile(samples, 0.95)
}

#[test]
#[ignore = "gate benchmark: generates a 5,000-symbol repo and needs the release binary"]
fn reuse_c1_p95_on_5000_symbols() {
    let root = std::env::temp_dir().join(format!("eg-reuse-bench-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    // The benchmark is `#[ignore]`, so a panic aborts before any Drop guard would run; the
    // explicit removal below covers the normal path and the harness cleanup covers the rest.
    write_corpus(&root);
    let eg = binary();
    let build = Command::new(&eg)
        .arg("--root")
        .arg(&root)
        .arg("--no-journal")
        .args(["build"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let (baseline_median, baseline_p95) = measure(&eg, &root, &[]);
    let (deep_median, deep_p95) = measure(&eg, &root, &["--deep-callee"]);
    println!(
        "reuse p50: baseline={:.2}ms deep-callee={:.2}ms (+{:.1}%); p95: baseline={:.2}ms deep-callee={:.2}ms",
        baseline_median.as_secs_f64() * 1e3,
        deep_median.as_secs_f64() * 1e3,
        (deep_median.as_secs_f64() / baseline_median.as_secs_f64() - 1.0) * 100.0,
        baseline_p95.as_secs_f64() * 1e3,
        deep_p95.as_secs_f64() * 1e3,
    );

    // The C1 keep gate is the detector-phase budget
    // (`query::reuse::detectors::tests::detector_scan_gate_on_5000_symbols`,
    // measured at ~3ms). This end-to-end number includes SQLite load and
    // process startup; guard it against gross regression instead.
    assert!(
        baseline_p95 < Duration::from_millis(100),
        "end-to-end regression: p95 {:.2}ms >= 100ms",
        baseline_p95.as_secs_f64() * 1e3
    );
    assert!(
        deep_median <= baseline_median.mul_f64(1.10),
        "C2 gate: --deep-callee median regression exceeds 10%"
    );
    let _ = fs::remove_dir_all(&root);
}
