//! AST walk: symbol, call, import, and inheritance extraction for one file.
//!
//! The public entry (`extract_file`) lives in the parent; this module owns the
//! node dispatch and the per-file walking state.

use super::*;

mod bindings;
mod chain;
mod dispatch;
mod python;
mod rust;
mod rust_use;
mod typescript;

use bindings::*;
pub use chain::split_ident;
use chain::*;
pub(super) use dispatch::walk_children;
use dispatch::*;

pub(super) fn innermost_scope(ctx: &WalkCtx, module: &str) -> String {
    [
        ctx.func_stack.last(),
        ctx.class_stack.last(),
        ctx.mod_stack.last(),
    ]
    .into_iter()
    .flatten()
    .max_by_key(|qualname| qualname.len())
    .cloned()
    .unwrap_or_else(|| module.to_string())
}

pub(super) struct WalkCtx {
    class_stack: Vec<String>, // class/impl qualnames
    mod_stack: Vec<String>,   // inline `mod` qualnames (Rust)
    func_stack: Vec<String>,  // qualnames
    /// Test ancestry (schema v8): inside a Rust `#[test]`/`#[cfg(test)]` item
    /// or a TS `describe(...)`/`it(...)` callback. Save/restore per scope.
    test_scope: bool,
}

impl WalkCtx {
    pub(super) fn for_file(_lang: Lang, _src: &[u8]) -> WalkCtx {
        WalkCtx {
            class_stack: Vec::new(),
            mod_stack: Vec::new(),
            func_stack: Vec::new(),
            test_scope: false,
        }
    }

    fn scope_func(&self) -> String {
        self.func_stack.last().cloned().unwrap_or_default()
    }
}

/// Symbol fields shared by the callable constructors (function/method).
pub(super) struct CallableSpec<'a> {
    pub kind: SymbolKind,
    pub name: &'a str,
    pub qualname: &'a str,
    pub is_test: bool,
    /// Python pushes the scope before the body check and pops it when there
    /// is no body; Rust/TS push it only when a body exists.
    pub always_push_scope: bool,
}

/// One node's mutable walk state, so the callable helpers need only a spec.
pub(super) struct Walker<'a, 's, 'b> {
    pub(super) node: Node<'a>,
    pub(super) src: &'s [u8],
    pub(super) fx: &'b mut FileExtraction,
    pub(super) ctx: &'b mut WalkCtx,
    pub(super) work: &'b mut Vec<Step<'a>>,
}

impl<'a, 's> Walker<'a, 's, '_> {
    /// Push the symbol row.
    pub(super) fn push_callable(&mut self, spec: &CallableSpec<'_>) {
        self.fx.push_symbol(NewSymbol::from_node(
            self,
            spec.kind,
            spec.name.to_string(),
            spec.qualname.to_string(),
            self.node,
            self.node,
            spec.is_test,
        ));
    }

    /// Record parameter types, then queue the body with its scope pop.
    pub(super) fn finish_callable(&mut self, spec: &CallableSpec<'_>) {
        if let Some(params) = self.node.child_by_field_name("parameters") {
            collect_param_types(self, params, spec.qualname, spec.kind == SymbolKind::Method);
        }
        if spec.always_push_scope || self.node.child_by_field_name("body").is_some() {
            self.ctx.func_stack.push(spec.qualname.to_string());
        }
        if let Some(body) = self.node.child_by_field_name("body") {
            self.work.push(Step::PopFunc);
            push_children_rev(self.work, body);
        } else if spec.always_push_scope {
            self.ctx.func_stack.pop();
        }
    }

    /// Shared CallableSpec constructor: the base is-test flag comes
    /// from the walked node.
    pub(super) fn callable_spec<'c>(
        &self,
        kind: SymbolKind,
        name: &'c str,
        qualname: &'c str,
    ) -> CallableSpec<'c> {
        CallableSpec {
            kind,
            name,
            qualname,
            is_test: is_test_item(self.fx, self.ctx, name),
            always_push_scope: false,
        }
    }
}

impl NewSymbol {
    /// Shared node-backed constructor: the one-line signature comes from
    /// `sig_node`, both 1-based line numbers from `line_node`, and the file
    /// index plus source bytes from the walk state, so the `push_symbol`
    /// emitters cannot drift apart field by field.
    pub(super) fn from_node(
        w: &Walker<'_, '_, '_>,
        kind: SymbolKind,
        name: String,
        qualname: String,
        line_node: Node<'_>,
        sig_node: Node<'_>,
        is_test: bool,
    ) -> Self {
        NewSymbol {
            kind,
            name,
            qualname,
            file_idx: w.fx.file_idx,
            signature: signature_of(sig_node, w.src),
            line_start: line_node.start_position().row + 1,
            line_end: line_node.end_position().row + 1,
            is_test,
        }
    }
}
