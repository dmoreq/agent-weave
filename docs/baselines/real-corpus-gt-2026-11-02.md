# Real-corpus P8 ground truth (2026-11-02)

Frozen on 2026-11-02 at kg `342ebc3`, vihap `efcd310`, finguard `dddfe07`,
Flutter `20f82749394`, food-dispatch `6bc5337`. Re-run on 2026-11-03 at
vihap `d060ace`, finguard `dddfe07`; the full finguard tree (not the p8
utility subset) is required for the two TS rows. Local-only gates: the
harness needs these working trees and is not run in CI.

**TL;DR** — 77 source-verified rows: imports 40/40, tests 20/20, inheritance
17/17 at 1.000 recall. Ground truth was derived from source ASTs before
measuring kg, not from kg output.

## 1. Files

| gate | GT file | rows | corpora |
|------|---------|------|----------|
| F7 imports | `import-gt.jsonl` | 40 | vihap 25, kg self 15 |
| F8 tests | `tests-gt.jsonl` | 20 | vihap 20 test files |
| F1 inheritance | `inheritance-gt.jsonl` | 17 | vihap 15, finguard 2 |

## 2. Construction

### 2.1 Imports

- Python rows: `ast.ImportFrom` / `ast.Import` with a `vihap.` module whose
  `module.py` or `module/__init__.py` exists in the vihap tree.
- Rust rows: tree-sitter `use_declaration` with a `crate::` path; the target
  module is the path prefix minus the imported leaf, mapped to `src.*`.
  Parent `src` prefix is required to exist as `.rs` or `/mod.rs`.
- One row per `(file, line, target)`, sorted by file, line, target. On
  2026-11-03 the kg-self `src/walk.rs` row moved from line 10 to 11 after a
  source change; the row was re-pinned at the current source.
- A row hits when `kg imports <file>` emits exactly
  `<file>:<line> <target>`.

### 2.2 Tests

- Python test files under `tests/test_*.py`.
- Imports are accepted only at module body; function-local imports are
  excluded because F8 pins them as out of scope.
- `vihap.*` re-exports are resolved through package `__init__.py` imports
  before the row is frozen (for example
  `vihap.adapters.AddressTransformer` -> `vihap.adapters.sklearn.AddressTransformer`).
- A test row records the target qualname, test file, and the `def test_*`
  line. It hits when `kg tests <target> --limit 10000` emits that exact
  test file and line.

### 2.3 Inheritance

- Python rows: `ClassDef` bases that resolve to a class in the same vihap
  module or to an imported `vihap.*` class. The 2026-11-03 extension adds
  five cross-file rows: `BaseSpanMatch` -> three grammar subclasses and
  `AddressCleaner` -> two cleaner subclasses.
- The finguard rows are TypeScript interface/class extensions; one same-file
  interface row and one cross-file class row
  (`SnapshotStore` -> `MemorySnapshotStore`).
- A row hits when `kg implementers <base> --limit 10000` lists the child
  qualname.
- tsconfig `paths` aliases are not resolved in v1; a pinned fixture asserts
  the unresolved `kind='inherit'` row instead of an edge.

## 3. Harness commands

```bash
python3 tests/scripts/import_recall.py \
  --pairs docs/baselines/import-gt.jsonl \
  --kg-root /Users/quy.doan/Workspace/kg \
  --vihap-root /Users/quy.doan/Workspace/mapkit/vihap \
  target/release/kg

python3 tests/scripts/tests_recall.py \
  --pairs docs/baselines/tests-gt.jsonl \
  --vihap-root /Users/quy.doan/Workspace/mapkit/vihap \
  target/release/kg

python3 tests/scripts/inheritance_recall.py \
  --pairs docs/baselines/inheritance-gt.jsonl \
  --vihap-root /Users/quy.doan/Workspace/mapkit/vihap \
  --finguard-root /Users/quy.doan/Workspace/personal/finguard \
  target/release/kg
```

Build the corpus indexes before running the gates:

```bash
target/release/kg --root /Users/quy.doan/Workspace/kg build --force
target/release/kg --root /Users/quy.doan/Workspace/mapkit/vihap build --force
target/release/kg --root /Users/quy.doan/Workspace/personal/finguard build --force
```

## 4. Results at freeze

| gate | recall | precision | Wilson 95% (recall) | gate |
|------|--------|-----------|---------------------|------|
| imports | 40/40 = 1.000 | 183/183 = 1.000 | 0.912-1.000 | recall/precision >= 0.90 |
| tests | 20/20 = 1.000 | 2679/2679 = 1.000 | 0.839-1.000 | recall/precision >= 0.90 |
| inheritance | 17/17 = 1.000 | not applicable | 0.816-1.000 | recall 1.00 |

Precision is measured over every emitted row for the GT files. An import row
is positive when its source line (or its multi-line import block) is
import-shaped and names the target leaf. A test row is positive when its
qualname matches the pinned `test_*`/`test[A-Z]` rule.

## 5. Flutter corpus

Dart is not indexed; P9 owns the Dart grammar. Flutter is a build-smoke
corpus for supported files, not a recall gate:

```bash
target/release/kg --root /Users/quy.doan/Packages/flutter build --force
# files=124 symbols=735 edges=1514 parse_errors=0 wall=3.46s
```

## 6. Pinned exclusions

- Function-local Python imports (F8 limit): excluded from `tests-gt.jsonl`.
- Cross-file Rust `impl Trait for Type` (F1 limit): no row; food-dispatch
  has only cross-file impls.
- TypeScript `tsconfig` `paths` aliases (F1 limit): no cross-file row; the
  pinned fixture asserts the unresolved inherit row instead.
- Dart imports/tests/inheritance: P9; not measured here.
- `kg tests` is queried with `--limit 10000` so GT rows behind the default
  50-row display cap are observable; the cap itself is covered by CLI tests.
