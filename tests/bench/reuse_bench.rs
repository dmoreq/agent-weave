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
use std::sync::Mutex;
use std::time::{Duration, Instant};

const FILES_5K: usize = 500;
const FILES_10K: usize = 1_000;
const WARMUP_RUNS: usize = 10;
const MEASURED_RUNS: usize = 50;
/// One benchmark at a time: parallel corpus generation and CPU contention make
/// the p95 gates meaningless.
static BENCH_LOCK: Mutex<()> = Mutex::new(());

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

fn write_corpus(root: &Path, files: usize, filler_count: usize) {
    let mut common = String::new();
    for group in 0..files {
        for helper in 0..3 {
            common.push_str(&format!("def h{group}_{helper}():\n    return {helper}\n"));
        }
    }
    let mut filler = String::new();
    for i in 0..filler_count {
        filler.push_str(&format!("def filler_{i}():\n    return {i}\n"));
    }
    fs::write(root.join("common.py"), common).unwrap();
    fs::write(root.join("filler.py"), filler).unwrap();
    for group in 0..files {
        let source = format!(
            "from common import h{group}_0, h{group}_1, h{group}_2\n\
def pipeline_{group}_a():\n    return h{group}_0() + h{group}_1() + h{group}_2()\n\
def pipeline_{group}_b():\n    return h{group}_0() + h{group}_1() + h{group}_2()\n"
        );
        fs::write(root.join(format!("g{group}.py")), source).unwrap();
    }
}

fn build(eg: &Path, root: &Path) {
    let build = Command::new(eg)
        .arg("--root")
        .arg(root)
        .arg("--no-journal")
        .args(["build"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
}

fn bench_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("eg-reuse-bench-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
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
fn bench_scan_5k_p95() {
    scan_gate("5k", FILES_5K, 2_500, Duration::from_millis(200));
}

#[test]
#[ignore = "gate benchmark: generates a 10,000-symbol repo and needs the release binary"]
fn bench_scan_10k_p95() {
    scan_gate("10k", FILES_10K, 5_000, Duration::from_millis(500));
}

fn scan_gate(name: &str, files: usize, filler: usize, budget: Duration) {
    let _guard = BENCH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = bench_root(name);
    // The benchmark is `#[ignore]`, so a panic aborts before any Drop guard
    // would run; the explicit removal below covers the normal path.
    write_corpus(&root, files, filler);
    let eg = binary();
    build(&eg, &root);

    let (baseline_median, baseline_p95) = measure(&eg, &root, &[]);
    let (deep_median, deep_p95) = measure(&eg, &root, &["--deep-callee"]);
    println!(
        "{name} reuse p50: baseline={:.2}ms deep-callee={:.2}ms (+{:.1}%); p95: baseline={:.2}ms deep-callee={:.2}ms",
        baseline_median.as_secs_f64() * 1e3,
        deep_median.as_secs_f64() * 1e3,
        (deep_median.as_secs_f64() / baseline_median.as_secs_f64() - 1.0) * 100.0,
        baseline_p95.as_secs_f64() * 1e3,
        deep_p95.as_secs_f64() * 1e3,
    );

    assert!(
        baseline_p95 < budget,
        "{name} end-to-end regression: p95 {:.2}ms >= {:?}",
        baseline_p95.as_secs_f64() * 1e3,
        budget
    );
    assert!(
        deep_median <= baseline_median.mul_f64(1.10),
        "{name} --deep-callee median regression exceeds 10%"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
#[ignore = "gate benchmark: hashes 16 MiB and needs the release binary"]
fn bench_hash_16mib() {
    let _guard = BENCH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = bench_root("hash");
    let eg = binary();
    let payload = 1024 * 1024;
    for i in 0..16 {
        let head = format!("def hash_target_{i}():\n    return {i}\n");
        let mut source = String::with_capacity(payload);
        source.push_str(&head);
        source.push_str(&"#".repeat(payload - head.len()));
        fs::write(root.join(format!("h{i}.py")), source).unwrap();
    }
    build(&eg, &root);
    let fresh = (0..20).map(|_| run(&eg, &root, &[])).collect();
    let (_, fresh_p95) = percentile(fresh, 0.95);
    // Same bytes, fresh mtime: every file reaches the hash phase.
    for i in 0..16 {
        let path = root.join(format!("h{i}.py"));
        let bytes = fs::read(&path).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let samples = (0..20).map(|_| run(&eg, &root, &[])).collect();
    let (_, hash_p95) = percentile(samples, 0.95);
    let added = hash_p95.saturating_sub(fresh_p95);
    println!(
        "hash 16MiB: fresh p95={:.2}ms, hashing p95={:.2}ms, added={:.2}ms",
        fresh_p95.as_secs_f64() * 1e3,
        hash_p95.as_secs_f64() * 1e3,
        added.as_secs_f64() * 1e3
    );
    assert!(
        added <= Duration::from_millis(25),
        "hash wall: added {:.2}ms > 25ms",
        added.as_secs_f64() * 1e3
    );
    let _ = fs::remove_dir_all(&root);
}
