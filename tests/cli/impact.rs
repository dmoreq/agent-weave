//! Integration tests: impact section.

use super::common::*;

#[test]
fn impact_depth_footer_only_when_frontier_extended() {
    let repo = temp_repo("impact-depth-footer");
    write(&repo, "c.py", "def h():\n    pass\n");
    write(&repo, "b.py", "from c import h\n\ndef g():\n    h()\n");
    build(&repo);

    let out = eg(&repo)
        .args(["impact", "c.py", "--depth", "5", "--edges", "calls"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        !stdout.contains("depth cap"),
        "exhausted BFS reported a cap: {stdout}"
    );
}

#[test]
fn callers_depth_two_reaches_transitive_callers() {
    let repo = temp_repo("callers-depth");
    write(&repo, "c.py", "def h():\n    pass\n");
    write(&repo, "b.py", "from c import h\n\ndef g():\n    h()\n");
    write(&repo, "a.py", "from b import g\n\ndef f():\n    g()\n");
    build(&repo);

    // Depth 1 keeps the existing direct-caller format and does not add tags.
    let direct = eg(&repo).args(["callers", "h"]).output().unwrap();
    let d1 = String::from_utf8_lossy(&direct.stdout);
    assert_eq!(direct.status.code(), Some(0), "{d1}");
    assert!(d1.contains("b.g"), "{d1}");
    assert!(!d1.contains("a.f"), "{d1}");
    assert!(!d1.contains("[depth="), "{d1}");

    let out = eg(&repo)
        .args(["callers", "h", "--depth", "2"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("transitive callers"), "{stdout}");
    assert!(stdout.contains("b.g [depth=1]"), "{stdout}");
    assert!(stdout.contains("a.f [depth=2]"), "{stdout}");
}

#[test]
fn p8a_impact_depth_two_reaches_transitive_callers() {
    let repo = temp_repo("impact-depth");
    write(&repo, "c.py", "def h():\n    pass\n");
    write(&repo, "b.py", "from c import h\n\ndef g():\n    h()\n");
    write(&repo, "a.py", "from b import g\n\ndef f():\n    g()\n");
    build(&repo);

    let out = eg(&repo)
        .args(["impact", "c.py", "--depth", "2", "--edges", "calls"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("b.py"), "{stdout}");
    assert!(stdout.contains("a.py"), "{stdout}");
}

#[test]
fn impact_depth_two_includes_module_level_caller() {
    let repo = temp_repo("impact-module-caller");
    write(&repo, "m1.py", "def f():\n    f()\n");
    write(&repo, "mid.py", "from m1 import f\n\ndef g():\n    f()\n");
    write(&repo, "other.py", "from m1 import f\n\ndef h():\n    f()\n");
    write(&repo, "top.py", "from mid import g\n\ng()\n");
    build(&repo);

    let out = eg(&repo)
        .args(["impact", "m1.py", "--depth", "2"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("mid.py"), "{stdout}");
    assert!(
        stdout.contains("top.py"),
        "module-level caller missing: {stdout}"
    );
}

#[test]
fn p8a_symbols_directory_groups_files_and_counts_symbols() {
    let repo = temp_repo("symbols-dir");
    write(&repo, "pkg/a.py", "def a():\n    pass\n");
    write(&repo, "pkg/b.py", "def b():\n    pass\n");
    build(&repo);

    let out = eg(&repo)
        .args(["symbols", "pkg", "--outline"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("eg: dir pkg — 2 files"), "{stdout}");
    assert!(stdout.contains("pkg/a.py"), "{stdout}");
    assert!(stdout.contains("pkg/b.py"), "{stdout}");
    assert!(stdout.contains("function pkg.a.a"), "{stdout}");
}

#[test]
fn import_edges_cover_plain_and_from_imports() {
    let repo = temp_repo("import-edges");
    write(&repo, "a.py", "def only():\n    pass\n");
    write(
        &repo,
        "b.py",
        "from a import only\n\ndef use():\n    only()\n",
    );
    write(&repo, "c.py", "import a\n");
    write(&repo, "d.py", "from a import only\n");
    build(&repo);

    let out = eg(&repo)
        .args(["impact", "a.py", "--edges", "imports"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("b.py:1"), "{stdout}");
    assert!(stdout.contains("c.py:1"), "plain import edge: {stdout}");
    assert!(stdout.contains("d.py:1"), "{stdout}");
}

#[test]
fn imports_lists_outbound_resolved_edges() {
    let repo = temp_repo("imports-cmd");
    write(&repo, "a.py", "def only():\n    pass\n");
    write(
        &repo,
        "b.py",
        "from a import only\n\ndef use():\n    only()\n",
    );
    build(&repo);

    let out = eg(&repo).args(["imports", "b.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.starts_with("eg: imports —"), "{stdout}");
    assert!(stdout.contains("b.py:1 a.only"), "{stdout}");
}

#[test]
fn import_ts_relative_imports_collapse_extensions_and_index() {
    let repo = temp_repo("ts-extensions");
    write(
        &repo,
        "src/app.ts",
        "import Widget from './widget';\nimport { helper } from './util.mjs';\nimport * as ns from './pkg/index';\nexport * from './reexport';\nconst dyn = import('./lazy.cjs');\nconst req = require('./legacy.mjs');\n",
    );
    write(&repo, "src/widget.ts", "export default class Widget {}\n");
    write(&repo, "src/util.mts", "export function helper() {}\n");
    write(&repo, "src/pkg/index.ts", "export function inner() {}\n");
    write(&repo, "src/reexport.ts", "export function re() {}\n");
    write(&repo, "src/lazy.cts", "export function lazy() {}\n");
    write(&repo, "src/legacy.cts", "export function legacy() {}\n");
    build(&repo);

    let out = eg(&repo).args(["imports", "src/app.ts"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    for row in [
        "src/app.ts:1 src.widget",
        "src/app.ts:2 src.util",
        "src/app.ts:2 src.util.helper",
        "src/app.ts:3 src.pkg",
        "src/app.ts:4 src.reexport",
        "src/app.ts:5 src.lazy",
        "src/app.ts:6 src.legacy",
    ] {
        assert!(stdout.contains(row), "missing {row}: {stdout}");
    }
    assert!(
        !stdout.contains("src.widget.default"),
        "default import leaked a symbol edge: {stdout}"
    );

    let out = eg(&repo).args(["unresolved"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "TS imports left unresolved rows: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn import_python_from_import_module_suppresses_redundant_parent_edge() {
    let repo = temp_repo("from-import-module");
    write(&repo, "pkg/__init__.py", "def b():\n    pass\n");
    write(&repo, "pkg/mod.py", "def inner():\n    pass\n");
    write(&repo, "app.py", "from pkg import mod\n");
    write(&repo, "app2.py", "from pkg import mod, b\n");
    build(&repo);

    let out = eg(&repo).args(["imports", "app.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let rows: Vec<&str> = stdout
        .lines()
        .filter(|line| line.starts_with("app.py:"))
        .collect();
    assert_eq!(
        rows,
        vec!["app.py:1 pkg.mod"],
        "module-only import keeps only the child edge: {stdout}"
    );

    let out = eg(&repo).args(["imports", "app2.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("app2.py:1 pkg"),
        "symbol import must keep the parent module edge: {stdout}"
    );
    assert!(stdout.contains("app2.py:1 pkg.mod"), "{stdout}");
    assert!(stdout.contains("app2.py:1 pkg.b"), "{stdout}");
}

#[test]
fn import_rust_use_tree_and_module_mapping_resolve() {
    let repo = temp_repo("rust-use-tree");
    write(&repo, "src/lib.rs", "pub mod a;\npub mod util;\n");
    write(&repo, "src/util.rs", "pub fn u() {}\n");
    write(
        &repo,
        "src/a/mod.rs",
        "pub fn f() {}\npub struct S;\nmod inner;\n",
    );
    write(&repo, "src/a/inner.rs", "pub fn deep() {}\n");
    write(&repo, "src/a/b.rs", "pub fn g() {}\n");
    write(
        &repo,
        "src/use_tree.rs",
        "use crate::a::{f, S as T, self};\nuse super::util::u;\nuse crate::a::b::g;\nuse crate::util;\nuse std::fmt::Debug;\n\nmod outer { use crate::util::u; }\nmod child;\n",
    );
    write(&repo, "src/use_tree/child.rs", "pub fn c() {}\n");
    build(&repo);

    let out = eg(&repo)
        .args(["imports", "src/use_tree.rs"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    for row in [
        "src/use_tree.rs:1 src.a",
        "src/use_tree.rs:1 src.a.S",
        "src/use_tree.rs:1 src.a.f",
        "src/use_tree.rs:2 src.util.u",
        "src/use_tree.rs:3 src.a.b.g",
        "src/use_tree.rs:4 src.util",
        "src/use_tree.rs:7 src.util.u",
        "src/use_tree.rs:8 src.use_tree.child",
    ] {
        assert!(stdout.contains(row), "missing {row}: {stdout}");
    }
    assert!(!stdout.contains("std"), "external crate leaked: {stdout}");

    let out = eg(&repo)
        .args(["imports", "src/a/mod.rs"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("src/a/mod.rs:3 src.a.inner"),
        "mod.rs child mapping: {stdout}"
    );

    let out = eg(&repo).args(["unresolved"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "std/external use must not leave unresolved imports"
    );
}

#[test]
fn import_star_suppression_is_per_import_statement() {
    let repo = temp_repo("star-per-statement");
    write(&repo, "pkg/__init__.py", "def f():\n    pass\n");
    write(&repo, "pkg/mod.py", "def f():\n    pass\n");
    write(
        &repo,
        "app.py",
        "import pkg\nfrom pkg import *\n\ndef use():\n    return pkg.mod.f()\n",
    );
    build(&repo);

    let out = eg(&repo).args(["imports", "app.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("app.py:1 pkg"),
        "plain import suppressed by a later star: {stdout}"
    );
    assert!(
        stdout.contains("app.py:2 pkg"),
        "star edge missing: {stdout}"
    );
    let out = eg(&repo).args(["unresolved"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "star must resolve pkg.mod.f");
}

#[test]
fn import_deleted_destination_heals_unresolved_and_restore() {
    let repo = temp_repo("deletion-heal");
    write(&repo, "pkg/__init__.py", "");
    write(&repo, "pkg/later.py", "def gone():\n    pass\n");
    write(&repo, "app.py", "from pkg.later import gone\n");
    build(&repo);

    let out = eg(&repo).args(["imports", "app.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("app.py:1 pkg.later.gone"),
        "resolved import before deletion: {stdout}"
    );

    fs::remove_file(repo.join("pkg/later.py")).unwrap();
    build(&repo);
    let out = eg(&repo).args(["unresolved", "--names"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("app.py:1 import pkg.later"),
        "deleted destination must leave an unresolved import: {stdout}"
    );

    write(&repo, "pkg/later.py", "def gone():\n    pass\n");
    build(&repo);
    let out = eg(&repo).args(["imports", "app.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("app.py:1 pkg.later.gone"),
        "restored destination must heal: {stdout}"
    );
    let out = eg(&repo).args(["unresolved"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "restored destination left an unresolved row"
    );
}

#[test]
fn p8c_unresolved_names_lists_sites_with_kind() {
    let repo = temp_repo("unresolved-names");
    write(&repo, "app.py", "def f():\n    missing_thing()\n");
    build(&repo);

    let out = eg(&repo).args(["unresolved", "--names"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("eg: unresolved=1 rows"), "{stdout}");
    assert!(stdout.contains("app.py:2 call missing_thing"), "{stdout}");
}

#[test]
fn import_edges_dedup_and_resolved_imports_are_not_unresolved() {
    let repo = temp_repo("import-dedup");
    write(&repo, "pkg/__init__.py", "");
    write(&repo, "pkg/mod.py", "def b():\n    pass\n");
    write(&repo, "c.py", "from pkg.mod import b\n");
    build(&repo);

    let out = eg(&repo).args(["imports", "c.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert_eq!(stdout.matches("c.py:1 pkg.mod.b").count(), 1, "{stdout}");
    assert!(stdout.contains("c.py:1 pkg.mod"), "{stdout}");

    let out = eg(&repo).args(["unresolved", "--names"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn nested_function_calls_resolve_and_dead_keeps_inner_alive() {
    let repo = temp_repo("nested-call");
    write(
        &repo,
        "app.py",
        "def outer():\n    def inner():\n        pass\n    inner()\n",
    );
    build(&repo);

    let out = eg(&repo).args(["callers", "inner"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("app.py:4 app.outer"), "{stdout}");

    let out = eg(&repo).args(["callees", "outer"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("app.py:2 app.outer.inner"), "{stdout}");

    let out = eg(&repo).args(["unresolved", "--names"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    let out = eg(&repo).args(["dead"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.lines().any(|line| line == "app.py:2"), "{stdout}");
}

#[test]
fn inheritance_ts_path_alias_is_pinned_unresolved() {
    let repo = temp_repo("inherit-ts-alias");
    write(&repo, "src/lib/base.ts", "export class Base {}\n");
    write(
        &repo,
        "src/child.ts",
        "import { Base } from '@/lib/base';\nexport class Child extends Base {}\n",
    );
    build(&repo);

    let out = eg(&repo).args(["unresolved", "--names"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("src/child.ts:2 inherit Base"), "{stdout}");
}

#[test]
fn inheritance_ignores_generic_arguments_and_external_paths() {
    let repo = temp_repo("inherit-false-edges");
    write(
        &repo,
        "a.ts",
        "class T {}\nclass Base<X> {}\nclass C extends Base<T> {}\n",
    );
    write(
        &repo,
        "lib.rs",
        "trait AppError: std::error::Error {}\nstruct std;\nstruct error;\nstruct Error;\n",
    );
    build(&repo);

    let out = eg(&repo)
        .args(["impact", "lib.rs", "--edges", "inherits"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn unresolved_reports_counts_and_zero_is_exit1() {
    let repo = temp_repo("unresolved");
    write(&repo, "m.py", "def go():\n    return ghost()\n");
    build(&repo);
    let out = eg(&repo).args(["unresolved"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("eg: unresolved=1 callees in 1 files"),
        "{stdout}"
    );
    assert!(stdout.contains("m.py unresolved=1"), "{stdout}");

    let clean = temp_repo("unresolved-clean");
    write(
        &clean,
        "a.py",
        "def g():\n    return 1\n\ndef go():\n    return g()\n",
    );
    build(&clean);
    let out = eg(&clean).args(["unresolved"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("0 unresolved in indexed scope"), "{stderr}");
}
