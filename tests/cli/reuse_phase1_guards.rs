//! Phase 1 matrix cases that generate larger corpora: output cap and D1
//! balance boundaries.

use super::common::*;

#[test]
fn reuse_repo_root_and_inner_dot_targets_scan() {
    let repo = temp_repo("reuse-repo-root-dot");
    write(
        &repo,
        "src/a.py",
        "def parse_token(token: str):\n    return token.strip()\n",
    );
    write(
        &repo,
        "src/b.py",
        "def parse_token(token: str):\n    return token.strip()\n",
    );
    build(&repo);
    for target in [".", "./"] {
        let out = eg(&repo)
            .args(["reuse", target, "--explain"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{target}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("path_matches=0"), "{target}: {stdout}");
        assert!(stdout.contains("parse_token"), "{target}: {stdout}");
    }
    for target in ["src/.", "src/./", "./src/"] {
        let out = eg(&repo)
            .args(["reuse", target, "--explain"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{target}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("path_matches=2"), "{target}: {stdout}");
        assert!(stdout.contains("parse_token"), "{target}: {stdout}");
    }
}

#[test]
fn reuse_repo_root_cannot_mix_with_paths() {
    let repo = temp_repo("reuse-repo-root-mixed");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    let out = eg(&repo).args(["reuse", ".", "a.py"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot be combined with other targets"),
        "{stderr}"
    );
}

#[test]
fn empty_below_threshold_run_journals_counts() {
    let repo = temp_repo("reuse-below-journal");
    write(
        &repo,
        "a.py",
        "def parse_token(token: str):\n    return token.strip()\n",
    );
    write(
        &repo,
        "b.py",
        "def parse_token(token: str):\n    return token.strip()\n",
    );
    build(&repo);
    let out = eg(&repo)
        .args(["reuse", "--min-confidence", "high"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let log = std::fs::read_to_string(repo.join(".engram/usage.log")).unwrap();
    let last: serde_json::Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
    assert_eq!(last["mode"], "scan");
    assert!(
        last["reuse"]["below_threshold"].as_u64().unwrap_or(0) >= 1,
        "{last}"
    );
}

#[test]
fn fresh_higher_confidence_sorts_before_stale_low() {
    let repo = temp_repo("reuse-order-freshness");
    write(
        &repo,
        "helpers.py",
        "def h1():\n    return 1\n\ndef h2():\n    return 2\n\ndef h3():\n    return 3\n\ndef normalize(value):\n    return value\n",
    );
    write(
        &repo,
        "a1.py",
        "from helpers import h1, h2, h3\n\ndef alpha_case():\n    return h1() + h2() + h3()\n",
    );
    write(
        &repo,
        "b1.py",
        "from helpers import h1, h2, h3\n\ndef beta_case():\n    return h1() + h2() + h3()\n",
    );
    write(
        &repo,
        "c.py",
        "from helpers import normalize\n\ndef parse_token_value_data_extra(value):\n    return normalize(value)\n",
    );
    write(
        &repo,
        "d.py",
        "from helpers import normalize\n\ndef parse_token_value(value):\n    return normalize(value)\n",
    );
    build(&repo);
    write(
        &repo,
        "b1.py",
        "from helpers import h1, h2, h3\n\ndef beta_case():\n    return h1() + h2() + h3() + 1\n",
    );
    let out = eg(&repo).args(["reuse", "--limit", "1"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("parse_token_value") && !stdout.contains("confidence=low"),
        "fresh higher-confidence candidate must sort first: {stdout}"
    );
    assert!(!stdout.contains("beta_case"), "{stdout}");
}

#[test]
fn class_constructors_are_not_callee_footprints() {
    let repo = temp_repo("reuse-class-constructor-footprint");
    write(
        &repo,
        "models.py",
        "class Widget:\n    pass\n\nclass Parser:\n    pass\n\ndef normalize(value):\n    return value\n",
    );
    write(
        &repo,
        "a.py",
        "from models import Widget, Parser, normalize\n\ndef alpha_case(value):\n    Widget()\n    Parser()\n    return normalize(value)\n",
    );
    write(
        &repo,
        "b.py",
        "from models import Widget, Parser, normalize\n\ndef beta_case(value):\n    Widget()\n    Parser()\n    return normalize(value)\n",
    );
    build(&repo);
    let out = eg(&repo).args(["reuse", "--explain"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("detector=D1"),
        "constructors must not satisfy D1's footprint gates: {stdout}"
    );
    assert!(!stdout.contains("alpha_case"), "{stdout}");
}

#[test]
fn stoplisted_constructor_disqualifies_pass_through_wrapper() {
    let repo = temp_repo("reuse-d4-stoplisted-constructor");
    write(
        &repo,
        "models.py",
        "class W:\n    pass\n\ndef normalize(value):\n    return value\n",
    );
    write(
        &repo,
        "x.py",
        "from models import W, normalize\n\ndef wrap(value):\n    W()\n    return normalize(value)\n",
    );
    // Four constructor call sites push W past the dynamic callee stoplist.
    write(
        &repo,
        "y.py",
        "from models import W\n\ndef a():\n    W()\n\ndef b():\n    W()\n\ndef c():\n    W()\n",
    );
    build(&repo);
    let out = eg(&repo).args(["reuse", "--explain"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("pass-through wrapper"),
        "two resolved callees must not read as a wrapper: {stdout}"
    );
    assert!(!stdout.contains("x.wrap"), "{stdout}");
}

#[test]
fn scan_output_cap_30k() {
    let repo = temp_repo("reuse-output-cap");
    let mut a = String::new();
    let mut b = String::new();
    // Ten independent pairs. Each pair shares 20 very long-named helpers, so
    // its block exceeds 4 KiB while helper frequencies stay below the stoplist.
    let long_suffix = "x".repeat(380);
    for pair in 0..10 {
        let mut calls = Vec::new();
        for i in 0..20 {
            let name = format!("helper_{pair}_{i}_{long_suffix}");
            let line = format!("def {name}():\n    return {i}\n");
            a.push_str(&line);
            b.push_str(&line);
            calls.push(format!("{name}()"));
        }
        let call = calls.join(" + ");
        a.push_str(&format!("def dup_{pair}():\n    return {call}\n"));
        b.push_str(&format!("def dup_{pair}():\n    return {call}\n"));
    }
    write(&repo, "a.py", &a);
    write(&repo, "b.py", &b);
    build(&repo);
    let out = eg(&repo)
        .args(["reuse", "b.py", "--limit", "100"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("output capped at 30 KiB"), "{stdout}");
    assert!(stdout.contains("capped"), "{stdout}");
}

#[test]
fn scan_bal_boundaries() {
    for (target_n, candidate_n, expected) in [
        (29usize, 100usize, None),
        (30, 100, Some("low")),
        (49, 100, Some("low")),
        (50, 100, Some("medium")),
    ] {
        let repo = temp_repo(&format!("reuse-bal-{target_n}"));
        let mut helpers = String::new();
        for i in 0..candidate_n {
            helpers.push_str(&format!("def h{i}():\n    return {i}\n"));
        }
        let all_imports = (0..candidate_n)
            .map(|i| format!("h{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let all_calls = (0..candidate_n)
            .map(|i| format!("h{i}()"))
            .collect::<Vec<_>>()
            .join(" + ");
        let target_calls = (0..target_n)
            .map(|i| format!("h{i}()"))
            .collect::<Vec<_>>()
            .join(" + ");
        let target_imports = (0..target_n)
            .map(|i| format!("h{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        write(&repo, "helpers.py", &helpers);
        write(
            &repo,
            "a.py",
            &format!(
                "from helpers import {all_imports}\ndef alpha_case():\n    return {all_calls}\n"
            ),
        );
        write(
            &repo,
            "b.py",
            &format!(
                "from helpers import {target_imports}\ndef beta_case():\n    return {target_calls}\n"
            ),
        );
        build(&repo);
        let out = eg(&repo)
            .args(["reuse", "b.py", "--explain"])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        match expected {
            None => assert!(
                !stdout.contains("symbol 'b.beta_case'"),
                "bal 0.29 must reject: {stdout}"
            ),
            Some(level) => {
                assert!(
                    stdout.contains("symbol 'b.beta_case'"),
                    "bal {target_n}/100 must produce a candidate: {stdout}"
                );
                assert!(stdout.contains(&format!("confidence={level}")), "{stdout}");
                if level == "low" {
                    assert!(stdout.contains("reasons=[size-imbalance]"), "{stdout}");
                }
            }
        }
    }
}
