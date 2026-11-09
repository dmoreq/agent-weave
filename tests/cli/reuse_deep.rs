//! Integration tests: reuse deep-callee (C2) section.

use super::common::*;

#[test]
fn reuse_deep_callee_catches_two_hop_wrapper() {
    let repo = temp_repo("reuse-deep-callee");
    write(
        &repo,
        "core.py",
        "def canonical_a():\n    return 1\ndef canonical_b():\n    return 2\ndef canonical_c():\n    return 3\n\
def canonical_pipeline():\n    return canonical_a() + canonical_b() + canonical_c()\n\
def mid_helper():\n    return canonical_a() + canonical_b() + canonical_c()\n\
def mid_helper_two():\n    return canonical_a() + canonical_b() + canonical_c()\n",
    );
    write(
        &repo,
        "dup.py",
        "from core import mid_helper, mid_helper_two\n\
def wrapped_pipeline():\n    x = 1\n    y = 2\n    z = x + y\n    w = z + 1\n    return mid_helper() + w\n\
def guarded_pipeline_two():\n    x = 1\n    y = 2\n    z = x + y\n    w = z + 1\n    return mid_helper_two() + w\n",
    );
    build(&repo);

    // The wrapper has one direct callee and a long body: D1 1-hop, D2, and D4
    // all miss it. Only the weighted 2-hop footprint sees canonical_a/b/c.
    let shallow = eg(&repo).args(["reuse"]).output().unwrap();
    let shallow_stdout = String::from_utf8_lossy(&shallow.stdout);
    assert!(
        !shallow_stdout.contains("wrapped_pipeline"),
        "1-hop detectors must miss the two-hop wrapper: {shallow_stdout}"
    );

    let deep = eg(&repo).args(["reuse", "--deep-callee"]).output().unwrap();
    let deep_stdout = String::from_utf8_lossy(&deep.stdout);
    assert_eq!(deep.status.code(), Some(0), "{deep_stdout}");
    assert!(
        deep_stdout.contains("wrapped_pipeline"),
        "--deep-callee must catch the first two-hop wrapper: {deep_stdout}"
    );
    assert!(
        deep_stdout.contains("guarded_pipeline_two"),
        "--deep-callee must catch the second two-hop wrapper: {deep_stdout}"
    );
    assert!(
        deep_stdout.contains("callee footprint"),
        "the match must still be reported as a D1 footprint: {deep_stdout}"
    );
}
