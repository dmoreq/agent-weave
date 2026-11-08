//! Regression tests for GitHub-tracked resolution bugs and alias chains.

use super::*;

#[cfg(test)]
mod gt_repro_tests {
    use super::*;
    use crate::parse::parse_source;
    use crate::parse::Lang;

    fn extract_src(src: &str) -> FileExtraction {
        let parsed = parse_source(src.as_bytes(), Lang::Python).unwrap();
        extract_file(0, "m.py", src.as_bytes(), &parsed)
    }

    #[test]
    fn typed_param_receiver_call_is_recorded() {
        let fx = extract_src("class A:\n    def f(self, h: Handler):\n        h.handle(1)\n");
        let calls: Vec<_> = fx
            .calls
            .iter()
            .map(|c| (&c.callee, &c.scope_func))
            .collect();
        assert_eq!(calls.len(), 1, "calls recorded: {:?}", calls.len());
        assert!(matches!(calls[0].0, Callee::Attr(_)));
        assert_eq!(
            fx.bindings.params.len(),
            1,
            "params: {:?}",
            fx.bindings.params.len()
        );
    }

    #[test]
    fn call_in_try_block_is_recorded() {
        let fx = extract_src(
            "def g():\n    try:\n        x = foo()\n    except ValueError:\n        pass\n",
        );
        assert_eq!(fx.calls.len(), 1, "calls: {}", fx.calls.len());
    }

    #[test]
    fn module_path_collapses_rust_and_ts_markers() {
        assert_eq!(module_path_for("src/lib.rs"), ("src".to_string(), true));
        assert_eq!(module_path_for("src/a/mod.rs"), ("src.a".to_string(), true));
        assert_eq!(module_path_for("src/a.rs"), ("src.a".to_string(), false));
        assert_eq!(module_path_for("lib.rs"), ("lib".to_string(), true));
        assert_eq!(module_path_for("pkg/index.mts"), ("pkg".to_string(), true));
        assert_eq!(
            module_path_for("pkg/mod.cjs"),
            ("pkg.mod".to_string(), false)
        );
    }

    #[test]
    fn rust_use_tree_emits_grouped_alias_self_and_super_candidates() {
        let src = "use crate::a::{f, S as T, self};\nuse super::util::u;\n";
        let parsed = parse_source(src.as_bytes(), Lang::Rust).unwrap();
        let fx = extract_file(0, "src/use_tree.rs", src.as_bytes(), &parsed);
        let mut edges: Vec<_> = fx
            .import_edges
            .iter()
            .map(|c| (c.line, format!("{:?}", c.target)))
            .collect();
        edges.sort();
        edges.dedup();
        assert_eq!(
            edges,
            vec![
                (1, "Module { path: \"src.a\" }".to_string()),
                (1, "Symbol { module: \"src.a\", name: \"S\" }".to_string()),
                (1, "Symbol { module: \"src.a\", name: \"f\" }".to_string()),
                (2, "Module { path: \"src.util\" }".to_string()),
                (
                    2,
                    "Symbol { module: \"src.util\", name: \"u\" }".to_string()
                ),
            ],
            "use-tree candidates"
        );
        assert!(
            fx.bindings.aliases.iter().any(|(_, a, _)| a == "T"),
            "alias T"
        );
        assert!(
            fx.bindings.aliases.iter().any(|(_, a, _)| a == "u"),
            "alias u"
        );
    }

    #[test]
    fn self_field_assignment_recorded() {
        let fx = extract_src("class C:\n    def __init__(self):\n        self.res = Resolver()\n");
        assert_eq!(
            fx.bindings.self_fields.len(),
            1,
            "self_fields: {}",
            fx.bindings.self_fields.len()
        );
    }
}

#[cfg(test)]
mod alias_repro {
    use super::*;
    use crate::parse::{parse_source, Lang};

    #[test]
    fn aliased_from_import_binds_and_emits_edge_candidate() {
        let src = "from ..knowledge.marshal_io import default_artifacts_dir as knowledge_artifacts_dir\n\ndef g():\n    return knowledge_artifacts_dir()\n";
        let parsed = parse_source(src.as_bytes(), Lang::Python).unwrap();
        let fx = extract_file(0, "vihap/validators/gso_loader.py", src.as_bytes(), &parsed);
        assert!(
            fx.bindings
                .aliases
                .iter()
                .any(|(_, a, _)| a == "knowledge_artifacts_dir"),
            "aliases: {:?}",
            fx.bindings.aliases
        );
        assert_eq!(
            fx.import_edges.len(),
            3,
            "import_edges: {} (module, child module, symbol)",
            fx.import_edges.len()
        );
        assert_eq!(fx.calls.len(), 1, "calls: {}", fx.calls.len());
    }
}
