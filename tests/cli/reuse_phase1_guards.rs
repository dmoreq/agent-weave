//! Phase 1 matrix cases that generate larger corpora: output cap and D1
//! balance boundaries.

use super::common::*;

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
