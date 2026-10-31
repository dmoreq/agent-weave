#!/usr/bin/env python3
"""Derive frozen expected set for T1 from the recorded grep output.

Reads transcripts/out/T1-005.txt (rg call-site list) and applies the resolution
map audited from T1-001..T1-004:
  - vihap has exactly ONE definition: vihap/cleaners/unicode.py:47.
  - cleaners/text_utils.py:184, cleaners/__init__.py:158, cleaners/normalizer.py:32
    all re-export that same symbol (normalizer via text_utils).
  - Therefore every vihap/** file and every script importing from vihap.cleaners*
    resolves to vihap.cleaners.unicode.normalize_ascii.
  - Six scripts define their own local normalize_ascii (T1-001) and shadow it:
    build_industrial_zones.py, build_buildings_enriched.py, build_postal_codes.py,
    build_hamlets_enriched.py, build_knowledge_base.py, build_highway_junctions.py.
    Their call lines resolve LOCALLY and are excluded from T1 truth.
Writes expected/T1.json (pointers + per-top-dir grouping + provenance).
"""
import json
import os
import re
import sys
from collections import Counter

BASE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
OUT5 = os.path.join(BASE, "transcripts", "out", "T1-005.txt")
DEST = os.path.join(BASE, "expected", "T1.json")

LOCAL_DEF_SCRIPTS = {
    "scripts/build_industrial_zones.py",
    "scripts/build_buildings_enriched.py",
    "scripts/build_postal_codes.py",
    "scripts/build_hamlets_enriched.py",
    "scripts/build_knowledge_base.py",
    "scripts/build_highway_junctions.py",
}
DEF_RE = re.compile(r"^\s*def normalize_ascii\(")


def main():
    pointers = []
    skipped_local = Counter()
    with open(OUT5) as f:
        for line in f:
            line = line.rstrip("\n")
            if not line:
                continue
            path, lineno, _ = line.split(":", 2)
            path = path[2:] if path.startswith("./") else path
            if DEF_RE.match(_):
                continue
            if path in LOCAL_DEF_SCRIPTS:
                skipped_local[path] += 1
                continue
            pointers.append(f"{path}:{lineno}")

    def top_dir(ptr):
        path = ptr.rsplit(":", 1)[0]
        parts = path.split("/")
        if parts[0] == "vihap" and len(parts) > 2:
            return f"vihap/{parts[1]}"
        if parts[0] == "vihap":
            return "vihap/<module>"
        return parts[0]

    grouping = dict(sorted(Counter(top_dir(p) for p in pointers).items()))
    data = {
        "task": "T1",
        "symbol": "vihap.cleaners.unicode.normalize_ascii",
        "provenance": "derived from transcripts/out/T1-005.txt at vihap f7ca48c; "
        "resolution map audited in T1-001..T1-004 (single def + 3 re-export chains; "
        "6 scripts shadow with local defs)",
        "pointers": pointers,
        "grouping": grouping,
        "excluded_local_shadow": {k: v for k, v in sorted(skipped_local.items())},
    }
    os.makedirs(os.path.dirname(DEST), exist_ok=True)
    with open(DEST, "w") as f:
        json.dump(data, f, indent=1, ensure_ascii=False)
    n_ptr = len(pointers)
    n_files = len({p.rsplit(":", 1)[0] for p in pointers})
    n_skipped = sum(skipped_local.values())
    print(f"T1 expected: {n_ptr} call sites in {n_files} files; "
          f"{n_skipped} call lines excluded (local shadow in {len(skipped_local)} scripts)")
    print("grouping:", json.dumps(grouping))
    return 0


if __name__ == "__main__":
    sys.exit(main())
