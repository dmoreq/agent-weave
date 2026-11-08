//! TypeScript resolution fixtures: methods, constructors, imports, and namespaces.

use agent_weave::extract::{extract_file, Confidence, EdgeKind};
use agent_weave::parse::{parse_source, Lang};

#[test]
fn ts_class_method_and_this_call() {
    // R2: this.m() must resolve like Python self.m() — assert the EDGE.
    let src = "export class Ledger {\n  balance = 0\n  add(n: number): number {\n    return this.normalize(n)\n  }\n  private normalize(n: number): number {\n    return Math.abs(n)\n  }\n}\n";
    let parsed =
        agent_weave::parse::parse_source(src.as_bytes(), agent_weave::parse::Lang::TypeScript)
            .unwrap();
    let fx = agent_weave::extract::extract_file(0, "apps/core/ledger.ts", src.as_bytes(), &parsed);
    let names: Vec<&str> = fx.symbols.iter().map(|s| s.qualname.as_str()).collect();
    assert!(names.contains(&"apps.core.ledger.Ledger"), "{names:?}");
    assert!(names.contains(&"apps.core.ledger.Ledger.add"), "{names:?}");
    assert!(
        names.contains(&"apps.core.ledger.Ledger.normalize"),
        "{names:?}"
    );
    let resolution = agent_weave::extract::resolve(vec![fx], Default::default());
    let hit = resolution.edges.iter().any(|e| {
        e.kind == EdgeKind::Calls
            && e.confidence == Confidence::Extracted
            && e.dst_name.contains("Ledger.normalize")
    });
    assert!(
        hit,
        "this.normalize(n) must resolve: {:?}",
        resolution.edges
    );
}

#[test]
fn ts_new_expression_resolves_to_constructor() {
    let src1 = "export class Widget {}\n";
    let src2 = "import { Widget } from './widget'\n\nexport function make(): Widget {\n  const w = new Widget()\n  return w\n}\n";
    let p1 =
        agent_weave::parse::parse_source(src1.as_bytes(), agent_weave::parse::Lang::TypeScript)
            .unwrap();
    let p2 =
        agent_weave::parse::parse_source(src2.as_bytes(), agent_weave::parse::Lang::TypeScript)
            .unwrap();
    let fx1 = agent_weave::extract::extract_file(0, "apps/core/widget.ts", src1.as_bytes(), &p1);
    let fx2 = agent_weave::extract::extract_file(1, "apps/core/factory.ts", src2.as_bytes(), &p2);
    let resolution = agent_weave::extract::resolve(vec![fx1, fx2], Default::default());
    let hit = resolution.edges.iter().any(|e| {
        e.kind == EdgeKind::Calls
            && e.confidence == Confidence::Extracted
            && e.dst_name.contains("Widget")
    });
    assert!(hit, "new Widget() must resolve: {:?}", resolution.edges);
}

#[test]
fn ts_relative_import_resolves_by_path() {
    let src1 = "export function format(n: number): string {\n  return String(n)\n}\n";
    let src2 = "import { format } from '../lib/format'\n\nexport function run(n: number): string {\n  return format(n)\n}\n";
    let p1 =
        agent_weave::parse::parse_source(src1.as_bytes(), agent_weave::parse::Lang::TypeScript)
            .unwrap();
    let p2 =
        agent_weave::parse::parse_source(src2.as_bytes(), agent_weave::parse::Lang::TypeScript)
            .unwrap();
    let fx1 =
        agent_weave::extract::extract_file(0, "apps/web/src/lib/format.ts", src1.as_bytes(), &p1);
    let fx2 =
        agent_weave::extract::extract_file(1, "apps/web/src/features/run.ts", src2.as_bytes(), &p2);
    let resolution = agent_weave::extract::resolve(vec![fx1, fx2], Default::default());
    let hit = resolution.edges.iter().any(|e| {
        e.kind == EdgeKind::Calls
            && e.confidence == Confidence::Extracted
            && e.dst_name.contains("format")
    });
    assert!(
        hit,
        "../lib/format import must resolve: {:?}",
        resolution.edges
    );
}

#[test]
fn ts_external_import_calls_drop() {
    let src = "import { useState } from 'react'\n\nexport function useCount() {\n  const [n] = useState(0)\n  return n\n}\n";
    let parsed =
        agent_weave::parse::parse_source(src.as_bytes(), agent_weave::parse::Lang::TypeScript)
            .unwrap();
    let fx =
        agent_weave::extract::extract_file(0, "apps/web/src/useCount.ts", src.as_bytes(), &parsed);
    let resolution = agent_weave::extract::resolve(vec![fx], Default::default());
    assert!(
        !resolution
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Calls && e.dst_name.contains("useState")),
        "no useState edge"
    );
    assert!(
        resolution.stats.builtin_dropped >= 1,
        "external call counted as dropped: {:?}",
        resolution.stats
    );
}

#[test]
fn typescript_namespaces_are_scopes() {
    let src = "namespace A { export function shared() {} }\nnamespace B { export function shared() {} }\n";
    let parsed = parse_source(src.as_bytes(), Lang::TypeScript).unwrap();
    let fx = extract_file(0, "pkg/mod.ts", src.as_bytes(), &parsed);
    let qns: Vec<&str> = fx.symbols.iter().map(|s| s.qualname.as_str()).collect();
    assert!(qns.iter().any(|q| q.ends_with(".A.shared")), "{qns:?}");
    assert!(qns.iter().any(|q| q.ends_with(".B.shared")), "{qns:?}");
}
