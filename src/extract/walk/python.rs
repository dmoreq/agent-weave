//! Python-specific extraction: import statements and their module targets.

use super::*;

pub(super) fn record_import(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, scope: &str) {
    let line = node.start_position().row + 1;
    match node.kind() {
        "import_statement" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "dotted_name" => {
                        let path = text(child, src);
                        let root = path.split('.').next().unwrap_or(&path).to_string();
                        fx.aliases.push((
                            scope.to_string(),
                            root.clone(),
                            ImportTarget::ModuleRoot { path: root },
                        ));
                        let parts: Vec<&str> = path.split('.').collect();
                        for i in 1..=parts.len() {
                            fx.import_edges.push(ImportEdgeCandidate {
                                line,
                                target: ImportTarget::Module {
                                    path: parts[..i].join("."),
                                },
                            });
                        }
                    }
                    // `import a.b as c` wraps in aliased_import; a bare
                    // "alias" node appears in some grammar versions.
                    "aliased_import" | "alias" => {
                        let Some(target) = child.child_by_field_name("name") else {
                            continue;
                        };
                        let Some(alias) = child.child_by_field_name("alias") else {
                            continue;
                        };
                        let target_path = text(target, src);
                        fx.aliases.push((
                            scope.to_string(),
                            text(alias, src),
                            ImportTarget::ModuleRoot {
                                path: target_path.clone(),
                            },
                        ));
                        let parts: Vec<&str> = target_path.split('.').collect();
                        for i in 1..=parts.len() {
                            fx.import_edges.push(ImportEdgeCandidate {
                                line,
                                target: ImportTarget::Module {
                                    path: parts[..i].join("."),
                                },
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        "import_from_statement" => {
            let module_node = node.child_by_field_name("module_name");
            let module_abs =
                module_node.map(|m| resolve_from_module(m, src, &fx.module, fx.is_package));
            let Some(m) = module_abs else { return };
            if !m.is_empty() {
                fx.import_edges.push(ImportEdgeCandidate {
                    line,
                    target: ImportTarget::Module { path: m.clone() },
                });
                fx.from_import_parents.insert((line, m.clone()));
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if Some(child) == module_node {
                    continue;
                }
                match child.kind() {
                    "wildcard_import" => {
                        fx.stars.push((m.clone(), line));
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::ModuleRoot { path: m.clone() },
                        });
                    }
                    "dotted_name" => {
                        let name = text(child, src);
                        // `from .. import X` at a first-level package has an
                        // empty parent: bind the root child directly.
                        let alias = if m.is_empty() {
                            ImportTarget::ModuleRoot { path: name.clone() }
                        } else {
                            ImportTarget::Symbol {
                                module: m.clone(),
                                name: name.clone(),
                            }
                        };
                        fx.aliases.push((scope.to_string(), name.clone(), alias));
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::Module {
                                path: if m.is_empty() {
                                    name.clone()
                                } else {
                                    format!("{m}.{name}")
                                },
                            },
                        });
                        if !m.is_empty() {
                            fx.import_edges.push(ImportEdgeCandidate {
                                line,
                                target: ImportTarget::Symbol {
                                    module: m.clone(),
                                    name,
                                },
                            });
                        }
                    }
                    // `from m import n as y` wraps the binding in an
                    // aliased_import node (import_statement uses "alias").
                    "aliased_import" | "alias" => {
                        let Some(target) = child.child_by_field_name("name") else {
                            continue;
                        };
                        let Some(alias) = child.child_by_field_name("alias") else {
                            continue;
                        };
                        let name = text(target, src);
                        let alias_target = if m.is_empty() {
                            ImportTarget::ModuleRoot { path: name.clone() }
                        } else {
                            ImportTarget::Symbol {
                                module: m.clone(),
                                name: name.clone(),
                            }
                        };
                        fx.aliases
                            .push((scope.to_string(), text(alias, src), alias_target));
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::Module {
                                path: if m.is_empty() {
                                    name.clone()
                                } else {
                                    format!("{m}.{name}")
                                },
                            },
                        });
                        if !m.is_empty() {
                            fx.import_edges.push(ImportEdgeCandidate {
                                line,
                                target: ImportTarget::Symbol {
                                    module: m.clone(),
                                    name,
                                },
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// `from <m> import ...`: resolve m to an absolute module path.
/// Relative levels: for a plain module file, level 1 = its package; for a
/// package file, level 1 = the package itself.
pub(super) fn resolve_from_module(
    module_node: Node<'_>,
    src: &[u8],
    file_module: &str,
    is_package: bool,
) -> String {
    let raw = text(module_node, src);
    if !raw.starts_with('.') {
        return raw;
    }
    let level = raw.chars().take_while(|c| *c == '.').count();
    let rest = raw.trim_start_matches('.');
    let mut parts: Vec<&str> = file_module.split('.').collect();
    if !is_package {
        parts.pop(); // plain module -> its containing package
    }
    for _ in 1..level {
        parts.pop();
    }
    // Root-level plain module: the containing package is the empty string, and
    // joining it must not leave a stray leading dot (".widget" never matches
    // the indexed module "widget" and misclassifies project calls as dropped).
    if rest.is_empty() {
        parts.join(".")
    } else if parts.is_empty() {
        rest.to_string()
    } else {
        format!("{}.{}", parts.join("."), rest)
    }
}

// ---------------------------------------------------------------------------
// TypeScript/TSX/JSX definitions (P2)
// ---------------------------------------------------------------------------
