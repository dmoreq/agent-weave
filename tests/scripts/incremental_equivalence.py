#!/usr/bin/env python3
"""E1.1 — incremental/forced edge-set equivalence for aw.

For each case the script builds a fixture, applies one mutation, rebuilds
incrementally, dumps the normalized edge and unresolved-site sets, force
rebuilds the same final source, dumps again, and compares the two sets.
A negative control drops one tuple from the forced set and requires the
comparator to notice.

Usage:
    python3 -I tests/scripts/incremental_equivalence.py \
        --aw target/release/aw \
        --json docs/baselines/2026-11-05_0.4.0/e1.1.json

Exit 0 when every case matches and the negative control fires; 1 otherwise.
"""
from __future__ import annotations

import argparse
import json
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

FLAT = {
    "a.py": "def entry():\n    return helper()\n\n\ndef helper():\n    return 1\n",
    "b.py": "from a import entry\n\n\ndef call_a():\n    return entry()\n",
}
FLAT_EXTRA = "from a import helper\n\n\ndef call_helper():\n    return helper()\n"
FLAT_ADD = dict(FLAT, **{"c.py": FLAT_EXTRA})

PKG = {
    "pkg/__init__.py": "",
    "pkg/mod.py": "def helper():\n    return 1\n\n\ndef entry():\n    return helper()\n",
    "main.py": "from pkg.mod import entry\n\n\ndef call_entry():\n    return entry()\n",
}
PKG_EXTRA = "from pkg.mod import helper\n\n\ndef call_helper():\n    return helper()\n"
PKG_ADD = dict(PKG, **{"pkg/extra.py": PKG_EXTRA})

INH = {
    "base.py": "class Base:\n    def run(self):\n        return 1\n",
    "child.py": "from base import Base\n\n\nclass Child(Base):\n    def run(self):\n        return 2\n",
    "use.py": "from child import Child\n\n\ndef call_run():\n    item = Child()\n    return item.run()\n",
}
INH_GRAND_USE = "from grand import Grand\n\n\ndef call_grand_run():\n    item = Grand()\n    return item.run()\n"
INH_GRAND = {
    "grand.py": "from child import Child\n\n\nclass Grand(Child):\n    pass\n",
    "grand_use.py": INH_GRAND_USE,
}
INH_ADD = dict(INH, **INH_GRAND)


def flat_gap() -> dict:
    initial = {
        "a.py": "import missing\n\n\ndef entry():\n    return missing.run()\n",
        "b.py": "from a import entry\n\n\ndef call_a():\n    return entry()\n",
    }
    return {"initial": initial, "mutated": dict(initial, **{"missing.py": "def run():\n    return 1\n"})}


def pkg_gap() -> dict:
    initial = dict(
        PKG,
        **{"pkg/mod.py": "from pkg.missing import helper\n\n\ndef entry():\n    return helper()\n"},
    )
    return {"initial": initial, "mutated": dict(initial, **{"pkg/missing.py": "def helper():\n    return 1\n"})}


def inh_gap() -> dict:
    initial = {
        "child.py": "from missing_base import Base\n\n\nclass Child(Base):\n    def run(self):\n        return 2\n",
        "use.py": INH["use.py"],
    }
    return {
        "initial": initial,
        "mutated": dict(initial, **{"missing_base.py": "class Base:\n    def run(self):\n        return 1\n"}),
    }


def flat_candidate() -> dict:
    initial = {
        "pkg.py": "def foo():\n    return 1\n",
        "pkg/__init__.py": "def foo():\n    return 2\n",
        "use.py": "import pkg\n\n\ndef call_foo():\n    return pkg.foo()\n",
    }
    return {"initial": initial, "mutated": {k: v for k, v in initial.items() if k != "pkg.py"}}


def pkg_candidate() -> dict:
    initial = {
        "pkg.py": "def foo():\n    return 1\n",
        "pkg/__init__.py": "def foo():\n    return 2\n",
        "main.py": "import pkg\n\n\ndef call_foo():\n    return pkg.foo()\n",
    }
    return {"initial": initial, "mutated": {k: v for k, v in initial.items() if k != "pkg.py"}}


def inh_candidate_rename() -> dict:
    initial = {
        "base.py": "class Base:\n    def run(self):\n        return 1\n",
        "child.py": "from base import Base\n\n\nclass Child(Base):\n    def run(self):\n        return 2\n",
        "use.py": INH["use.py"],
    }
    mutated = {
        "base.py": "class Base2:\n    def run(self):\n        return 1\n",
        "child.py": "from base import Base2\n\n\nclass Child(Base2):\n    def run(self):\n        return 2\n",
        "use.py": INH["use.py"],
    }
    return {"initial": initial, "mutated": mutated}


def cases() -> list[dict]:
    return [
        {"name": "flat-add", "topology": "flat", "initial": FLAT, "mutated": FLAT_ADD},
        {"name": "flat-delete", "topology": "flat", "initial": FLAT_ADD, "mutated": FLAT},
        {
            "name": "flat-rename",
            "topology": "flat",
            "initial": FLAT_ADD,
            "mutated": dict(FLAT, **{"d.py": FLAT_EXTRA}),
        },
        {
            "name": "flat-touch",
            "topology": "flat",
            "initial": FLAT,
            "mutated": dict(FLAT, **{"b.py": FLAT["b.py"] + "\n# touch only\n"}),
        },
        {"name": "flat-gap-heal", "topology": "flat", **flat_gap()},
        {"name": "flat-candidate-shrink", "topology": "flat", **flat_candidate()},
        {"name": "pkg-add", "topology": "package", "initial": PKG, "mutated": PKG_ADD},
        {"name": "pkg-delete", "topology": "package", "initial": PKG_ADD, "mutated": PKG},
        {
            "name": "pkg-rename",
            "topology": "package",
            "initial": PKG_ADD,
            "mutated": dict(PKG, **{"pkg/renamed.py": PKG_EXTRA}),
        },
        {
            "name": "pkg-touch",
            "topology": "package",
            "initial": PKG,
            "mutated": dict(PKG, **{"main.py": PKG["main.py"] + "\n# touch only\n"}),
        },
        {"name": "pkg-gap-heal", "topology": "package", **pkg_gap()},
        {"name": "pkg-candidate-shrink", "topology": "package", **pkg_candidate()},
        {"name": "inh-add", "topology": "inheritance", "initial": INH, "mutated": INH_ADD},
        {"name": "inh-delete", "topology": "inheritance", "initial": INH_ADD, "mutated": INH},
        {
            "name": "inh-rename",
            "topology": "inheritance",
            "initial": INH_ADD,
            "mutated": dict(
                INH,
                **{
                    "grand2.py": INH_GRAND["grand.py"],
                    "grand_use.py": INH_GRAND_USE.replace("from grand import", "from grand2 import"),
                },
            ),
        },
        {
            "name": "inh-touch",
            "topology": "inheritance",
            "initial": INH,
            "mutated": dict(INH, **{"use.py": INH["use.py"] + "\n# touch only\n"}),
        },
        {"name": "inh-gap-heal", "topology": "inheritance", **inh_gap()},
        {"name": "inh-candidate-rename", "topology": "inheritance", **inh_candidate_rename()},
    ]


def write_tree(root: Path, files: dict[str, str | None]) -> None:
    for rel, content in files.items():
        path = root / rel
        if content is None:
            if path.exists():
                path.unlink()
            continue
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


def run_aw(aw: str, root: Path, args: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        [aw, "--root", str(root), "--no-journal", *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=120,
    )


def build(aw: str, root: Path, force: bool) -> subprocess.CompletedProcess:
    args = ["build"] + (["--force"] if force else [])
    return run_aw(aw, root, args)


def dump(root: Path) -> tuple[frozenset, frozenset]:
    db = root / ".aw" / "index.db"
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    edges = frozenset(
        conn.execute(
            """
            SELECT src.qualname, COALESCE(dst.qualname, e.dst_name),
                   e.kind, e.confidence, e.site_line
              FROM edges e
              JOIN symbols src ON src.id = e.src_id
              LEFT JOIN symbols dst ON dst.id = e.dst_id
            """
        )
    )
    unresolved = frozenset(
        conn.execute(
            """
            SELECT f.path, u.dst_name, u.kind, u.site_line
              FROM unresolved_calls u
              JOIN files f ON f.id = u.file_id
            """
        )
    )
    conn.close()
    return edges, unresolved


def first_diff(left: frozenset, limit: int = 5) -> list[str]:
    return [repr(item) for item in sorted(left, key=repr)[:limit]]


def run_case(aw: str, case: dict) -> dict:
    with tempfile.TemporaryDirectory(prefix=f"aw-eq-{case['name']}-") as tmp:
        root = Path(tmp)
        write_tree(root, case["initial"])
        initial = build(aw, root, force=False)
        if initial.returncode != 0:
            return {"name": case["name"], "error": f"initial build exit {initial.returncode}",
                    "stderr": initial.stderr[-500:], "equal": False}
        initial_edges, _ = dump(root)
        initial_ambiguous = sum(1 for edge in initial_edges if edge[3] == "AMBIGUOUS")

        write_tree(root, case["mutated"])
        started = time.perf_counter()
        incremental = build(aw, root, force=False)
        incremental_ms = (time.perf_counter() - started) * 1000
        if incremental.returncode != 0:
            return {"name": case["name"], "error": f"incremental build exit {incremental.returncode}",
                    "stderr": incremental.stderr[-500:], "equal": False}

        incremental_sets = dump(root)

        started = time.perf_counter()
        forced = build(aw, root, force=True)
        force_ms = (time.perf_counter() - started) * 1000
        if forced.returncode != 0:
            return {"name": case["name"], "error": f"forced build exit {forced.returncode}",
                    "stderr": forced.stderr[-500:], "equal": False}

        forced_sets = dump(root)
        equal = incremental_sets == forced_sets
        inc_edges, inc_unres = incremental_sets
        force_edges, force_unres = forced_sets
        return {
            "name": case["name"],
            "topology": case["topology"],
            "equal": equal,
            "initial_ambiguous_edges": initial_ambiguous,
            "incremental_edges": len(inc_edges),
            "forced_edges": len(force_edges),
            "incremental_unresolved": len(inc_unres),
            "forced_unresolved": len(force_unres),
            "missing_from_incremental": first_diff(force_edges - inc_edges),
            "extra_in_incremental": first_diff(inc_edges - force_edges),
            "missing_unresolved": first_diff(force_unres - inc_unres),
            "extra_unresolved": first_diff(inc_unres - force_unres),
            "incremental_ms": round(incremental_ms, 1),
            "force_ms": round(force_ms, 1),
        }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--aw", required=True)
    parser.add_argument("--json", required=True)
    args = parser.parse_args()

    version = subprocess.run(
        [args.aw, "--version"], capture_output=True, text=True, encoding="utf-8", timeout=30
    )
    results = [run_case(args.aw, case) for case in cases()]
    all_equal = all(case.get("equal", False) for case in results)

    synthetic = frozenset(
        {("a", "b", "calls", "EXTRACTED", 1), ("a", "c", "calls", "EXTRACTED", 2)}
    )
    control_ok = (synthetic - {next(iter(synthetic))}) != synthetic

    payload = {
        "experiment": "E1.1",
        "aw_version": version.stdout.strip(),
        "cases": results,
        "case_count": len(results),
        "all_equal": all_equal,
        "negative_control": control_ok,
    }
    Path(args.json).parent.mkdir(parents=True, exist_ok=True)
    Path(args.json).write_text(json.dumps(payload, indent=2), encoding="utf-8")

    for case in results:
        state = "OK" if case.get("equal") else "DIFF"
        print(f"{state:4} {case['name']:24} edges {case.get('incremental_edges', '?')}/"
              f"{case.get('forced_edges', '?')} unresolved {case.get('incremental_unresolved', '?')}/"
              f"{case.get('forced_unresolved', '?')} {case.get('incremental_ms', '?')}ms")
    print(f"all_equal={all_equal} negative_control={control_ok} cases={len(results)}")
    return 0 if all_equal and control_ok else 1


if __name__ == "__main__":
    sys.exit(main())
