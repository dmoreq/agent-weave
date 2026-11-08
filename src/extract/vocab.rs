//! Shared vocabularies for extraction and resolution: the schema-kind enums
//! and the builtin/stdlib name tables that gate the dropped-callee counters.

/// Python builtins: calls drop with the builtin counter (spec step 4).
pub(super) const BUILTINS: &[&str] = &[
    "abs",
    "aiter",
    "all",
    "any",
    "anext",
    "ascii",
    "bin",
    "bool",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "__import__",
    "Exception",
    "BaseException",
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BlockingIOError",
    "BrokenPipeError",
    "BufferError",
    "ChildProcessError",
    "ConnectionError",
    "ConnectionRefusedError",
    "ConnectionResetError",
    "EOFError",
    "FileExistsError",
    "FileNotFoundError",
    "FloatingPointError",
    "GeneratorExit",
    "ImportError",
    "IndentationError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "NotADirectoryError",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PermissionError",
    "ProcessLookupError",
    "RecursionError",
    "ReferenceError",
    "RuntimeError",
    "StopAsyncIteration",
    "StopIteration",
    "SyntaxError",
    "SystemError",
    "SystemExit",
    "TabError",
    "TimeoutError",
    "TypeError",
    "UnboundLocalError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "UnicodeTranslateError",
    "ValueError",
    "ZeroDivisionError",
];

/// Well-known stdlib module roots: `os.path.join(...)` on unindexed externals
/// drops with the builtin counter instead of flooding the unresolved count.
pub(super) const STDLIB_MODULES: &[&str] = &[
    "os",
    "sys",
    "re",
    "json",
    "math",
    "time",
    "datetime",
    "logging",
    "pathlib",
    "collections",
    "itertools",
    "functools",
    "typing",
    "dataclasses",
    "enum",
    "abc",
    "io",
    "csv",
    "sqlite3",
    "argparse",
    "subprocess",
    "shutil",
    "tempfile",
    "textwrap",
    "unicodedata",
    "uuid",
    "random",
    "statistics",
    "copy",
    "hashlib",
    "base64",
    "urllib",
    "http",
    "socket",
    "threading",
    "multiprocessing",
    "concurrent",
    "asyncio",
    "contextlib",
    "warnings",
    "unittest",
    "pickle",
    "glob",
    "fnmatch",
    "string",
    "struct",
    "zlib",
    "gzip",
    "tarfile",
    "zipfile",
    "heapq",
    "bisect",
    "array",
    "weakref",
    "types",
    "operator",
    "inspect",
    "traceback",
    "atexit",
    "signal",
    "stat",
    "errno",
    "platform",
    "getpass",
    "difflib",
    "secrets",
    "gc",
    "os.path",
];

/// Generate the shared lexical-vocabulary impl for one enum: the `VARIANTS`
/// table plus a `parse` that derives accepted spellings from `as_str` (defined
/// beside each enum), so a new variant cannot drift between the write and read
/// boundaries. One body replaces the per-enum copies; `as_str` stays per enum
/// because the spellings differ per vocabulary.
macro_rules! lexical_parse {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        impl $name {
            /// Every variant, in declaration order.
            const VARIANTS: &'static [$name] = &[$($name::$variant),+];

            /// Read boundary: `None` for foreign spellings, so carried rows
            /// with unknown spellings stay dropped.
            pub(crate) fn parse(kind: &str) -> Option<Self> {
                Self::VARIANTS
                    .iter()
                    .find(|variant| variant.as_str() == kind)
                    .copied()
            }
        }
    };
}

/// Symbol vocabulary stored in `symbols.kind` (schema contract). DB rows spell
/// kinds through `as_str()` at the bind boundary and parse back through
/// `parse` at the read boundary; in-memory comparisons use the variants so a
/// misspelled kind is a compile error instead of a silent wrong filter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SymbolKind {
    #[default]
    Module,
    Class,
    Function,
    Method,
    Trait,
    Interface,
}

impl SymbolKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Class => "class",
            Self::Function => "function",
            Self::Method => "method",
            Self::Trait => "trait",
            Self::Interface => "interface",
        }
    }
}

lexical_parse!(SymbolKind {
    Module,
    Class,
    Function,
    Method,
    Trait,
    Interface,
});

/// Edge vocabulary stored in `edges.kind`. No surface reads `contains` rows
/// (`db::edge_kinds_sql` omits it, matching the dead-code and gap-heal
/// semantics) and extraction no longer emits them, so the variant is gone;
/// legacy indexes may still carry the rows unread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeKind {
    Calls,
    Imports,
    Inherits,
    TypeRef,
}

impl EdgeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Calls => "calls",
            Self::Imports => "imports",
            Self::Inherits => "inherits",
            Self::TypeRef => "type_ref",
        }
    }
}

lexical_parse!(EdgeKind {
    Calls,
    Imports,
    Inherits,
    TypeRef,
});

/// Edge confidence vocabulary stored in `edges.confidence` (schema contract).
/// DB rows spell confidences through `as_str()` at the bind boundary; SQL
/// filters render the spelling from the variant, and in-memory comparisons
/// use the variants so a misspelled confidence is a compile error instead of
/// a silent wrong filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    Extracted,
    Ambiguous,
}

impl Confidence {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Extracted => "EXTRACTED",
            Self::Ambiguous => "AMBIGUOUS",
        }
    }
}

lexical_parse!(Confidence {
    Extracted,
    Ambiguous,
});

/// Unresolved-site vocabulary stored in `unresolved_calls.kind` (schema
/// contract). Same boundary discipline as `SymbolKind`/`EdgeKind`: rows spell
/// kinds through `as_str()` at the bind boundary and in-memory comparisons
/// use the variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeSiteKind {
    Call,
    Import,
    Inherit,
}

impl EdgeSiteKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Import => "import",
            Self::Inherit => "inherit",
        }
    }
}
