#!/usr/bin/env python3
"""Build sanitized corpora and source-derived oracles for the P8 A/B.

This script is intentionally independent of kg output for I1/H1/D1/A1. T1 is
source-derived from the two pinned static routes (direct calls plus the
pipeline streaming route) and was audited against the source before freeze.

Usage:
  python3 -I derive_oracles.py [--rebuild]
"""
from __future__ import annotations

import argparse
import ast
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
KG_SOURCE = Path("/Users/quy.doan/Workspace/kg")
VIHAP_SOURCE = Path("/Users/quy.doan/Workspace/mapkit/vihap")
FINGUARD_SOURCE = Path("/Users/quy.doan/Workspace/personal/finguard")
CORPORA = Path("/tmp/p8-utility-corpus-2026-11-03")
KG = CORPORA / "kg"
VIHAP = CORPORA / "vihap"
FINGUARD = CORPORA / "finguard"
ANSWERS = Path("/tmp/p8-utility-answers-2026-11-03")


def run(args: list[str], cwd: Path | None = None) -> str:
    return subprocess.run(args, cwd=cwd, check=True, capture_output=True,
                          text=True).stdout.strip()


def revision(repo: Path) -> str:
    try:
        return run(["git", "rev-parse", "--short", "HEAD"], cwd=repo)
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def copy_tree(src: Path, dst: Path, rebuild: bool) -> None:
    if dst.exists():
        if not rebuild:
            return
        shutil.rmtree(dst)
    shutil.copytree(
        src,
        dst,
        ignore=shutil.ignore_patterns(
            ".git", ".kg", "__pycache__", "*.pyc", ".venv", "venv",
            "node_modules", "target", "dist", "build", ".next", ".turbo",
        ),
    )


def build_corpora(rebuild: bool) -> None:
    CORPORA.mkdir(parents=True, exist_ok=True)
    copy_tree(KG_SOURCE / "src", KG / "src", rebuild)
    for name in ("Cargo.toml", "Cargo.lock"):
        src = KG_SOURCE / name
        if src.exists():
            shutil.copy2(src, KG / name)
    copy_tree(VIHAP_SOURCE, VIHAP, rebuild)
    copy_tree(FINGUARD_SOURCE / "apps/web/src", FINGUARD / "apps/web/src", rebuild)
    copy_tree(FINGUARD_SOURCE / "packages/core-ledger/src",
              FINGUARD / "packages/core-ledger/src", rebuild)
    ANSWERS.mkdir(parents=True, exist_ok=True)
    (HERE / "logs").mkdir(exist_ok=True)


def write_expected(task: str, items: list[str], note: str) -> None:
    payload = {"task": task, "items": sorted(items), "note": note}
    (HERE / "expected" / f"{task}.json").write_text(
        json.dumps(payload, indent=2) + "\n"
    )


def derive_i1() -> list[str]:
    path = KG / "src/query.rs"
    items: list[str] = []
    for line_no, line in enumerate(path.read_text().splitlines(), 1):
        stripped = line.strip()
        if not stripped.startswith("use crate::"):
            continue
        raw = stripped.removeprefix("use crate::").removesuffix(";").strip()
        parts = [p for p in raw.split("::") if p]
        # `use crate::db;` imports module db. A braced/path import binds the
        # last segment; the imported crate module is its parent.
        module_parts = parts if len(parts) == 1 else parts[:-1]
        items.append(f"{line_no} src.{'.'.join(module_parts)}")
    return items


def python_test_functions(path: Path) -> list[tuple[int, str]]:
    tree = ast.parse(path.read_text(encoding="utf-8", errors="ignore"))
    out: list[tuple[int, str]] = []
    module = ".".join(path.with_suffix("").relative_to(VIHAP).parts)
    stack: list[tuple[ast.AST, str]] = [(tree, "")]
    while stack:
        node, prefix = stack.pop()
        for child in ast.iter_child_nodes(node):
            if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef)):
                name = f"{prefix}.{child.name}" if prefix else child.name
                if name.startswith("test_") or (
                    name.startswith("test") and len(name) > 4 and name[4].isupper()
                ):
                    out.append((child.lineno, f"{module}.{name}"))
                stack.append((child, name))
            elif isinstance(child, ast.ClassDef):
                stack.append((child, f"{prefix}.{child.name}" if prefix else child.name))
    return out


def derive_t1() -> list[str]:
    """Direct-test oracle for exact_text_key: one source-verified caller."""
    path = VIHAP / "tests/test_centralized_grammar_patterns.py"
    tree = ast.parse(path.read_text(encoding="utf-8", errors="ignore"))
    module = ".".join(path.with_suffix("").relative_to(VIHAP).parts)
    items: list[str] = []

    def calls(node: ast.AST) -> set[str]:
        out: set[str] = set()
        for child in ast.walk(node):
            if isinstance(child, ast.Call):
                if isinstance(child.func, ast.Name):
                    out.add(child.func.id)
                elif isinstance(child.func, ast.Attribute):
                    out.add(child.func.attr)
        return out

    for node in tree.body:
        if (isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
                and node.name.startswith("test_")
                and "exact_text_key" in calls(node)):
            items.append(f"tests/test_centralized_grammar_patterns.py:{node.lineno} {module}.{node.name}")
    return items


def derive_h1() -> list[str]:
    path = VIHAP / "vihap/exceptions.py"
    tree = ast.parse(path.read_text(encoding="utf-8", errors="ignore"))
    items: list[str] = []
    for node in tree.body:
        if not isinstance(node, ast.ClassDef):
            continue
        bases = [b.id for b in node.bases if isinstance(b, ast.Name)]
        if "VihapError" in bases:
            items.append(
                f"vihap/exceptions.py:{node.lineno} vihap.exceptions.{node.name}"
            )
    return items


def derive_d1() -> list[str]:
    items: list[str] = []
    for path in VIHAP.rglob("*.py"):
        if any(part in {".git", ".kg", "build", ".venv"} for part in path.parts):
            continue
        try:
            tree = ast.parse(path.read_text(encoding="utf-8", errors="ignore"))
        except SyntaxError:
            continue
        hit = False
        for node in ast.walk(tree):
            if isinstance(node, ast.ImportFrom):
                module = node.module or ""
                if node.level:
                    package_parts = list(path.parent.relative_to(VIHAP).parts)
                    if path.name == "__init__.py":
                        base = package_parts
                    else:
                        base = package_parts[:-1]
                    for _ in range(node.level - 1):
                        if base:
                            base.pop()
                    module = ".".join(base + ([module] if module else []))
                if module in {"vihap.reform.temporal", "temporal"}:
                    hit = True
                    break
            elif isinstance(node, ast.Import):
                if any(alias.name == "vihap.reform.temporal"
                       for alias in node.names):
                    hit = True
                    break
        if hit:
            items.append(path.relative_to(VIHAP).as_posix())
    return items


def derive_a1() -> list[str]:
    path = FINGUARD / "packages/core-ledger/src/types.ts"
    items: list[str] = []
    for line_no, line in enumerate(path.read_text().splitlines(), 1):
        stripped = line.strip()
        if stripped.startswith("export interface "):
            name = stripped.removeprefix("export interface ").split("{", 1)[0]
            items.append(f"{line_no} {name.strip()}")
    return items


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--rebuild", action="store_true")
    args = ap.parse_args()
    build_corpora(args.rebuild)
    oracles = {
        "I1": (derive_i1(), "crate-internal imports in src/query.rs"),
        "T1": (derive_t1(), "direct static tests reaching exact_text_key"),
        "H1": (derive_h1(), "direct VihapError implementers"),
        "D1": (derive_d1(), "direct importers of vihap/reform/temporal.py"),
        "A1": (derive_a1(), "exported interfaces in core-ledger types.ts"),
    }
    for task, (items, note) in oracles.items():
        if not items:
            print(f"empty oracle for {task}", file=sys.stderr)
            return 1
        write_expected(task, items, note)
    manifest = {
        "date": "2026-11-03",
        "kg_revision": revision(KG_SOURCE),
        "vihap_revision": revision(VIHAP_SOURCE),
        "finguard_revision": revision(FINGUARD_SOURCE),
        "kg_binary_sha256": hashlib.sha256(
            (KG_SOURCE / "target/release/kg").read_bytes()).hexdigest(),
        "corpora": {name: str(path) for name, path in
                    {"kg": KG, "vihap": VIHAP, "finguard": FINGUARD}.items()},
        "oracle_counts": {task: len(items) for task, (items, _) in oracles.items()},
    }
    (HERE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
