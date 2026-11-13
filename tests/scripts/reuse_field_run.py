#!/usr/bin/env python3
"""Validate the frozen field-calibration sample for the reuse detectors.

The Phase 2 protocol fixture (`tests/fixtures/reuse/oracle-*.csv`) is a
synthetic 160-label ledger. This validator scores a *field* sample: real
repository code, real `eg reuse` predictions, and hand-inspected labels.

Schema of the frozen JSON (single argument):
  head            commit the corpus and predictions were frozen at
  eg_version      binary version that produced the predictions
  corpus          index build counters (files/symbols/edges/...)
  command         the exact `eg reuse` invocation
  selection       how the labeled subset was chosen from `candidates`
  candidates      every predicted pair: pair_id, target, canonical, tier,
                  detector, callers
  labels          labeled subset: pair_id, label (duplicate|distinct),
                  rationale

`--check` recomputes the stored `metrics` block and exits 1 on any drift.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Optional

TIERS = ("high", "medium", "provisional", "low")
LABELS = ("duplicate", "distinct")


def load(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    required = {"head", "eg_version", "corpus", "command", "selection", "candidates", "labels"}
    missing = required.difference(data)
    if missing:
        raise ValueError(f"missing fields: {sorted(missing)}")
    return data


def validate(data: dict[str, Any]) -> None:
    candidates = {c["pair_id"]: c for c in data["candidates"]}
    if len(candidates) != len(data["candidates"]):
        raise ValueError("candidate pair_id values must be unique")
    for candidate in data["candidates"]:
        if candidate["tier"] not in TIERS:
            raise ValueError(f"unknown tier: {candidate['tier']}")
    seen: set[str] = set()
    for label in data["labels"]:
        pair_id = label["pair_id"]
        if pair_id not in candidates:
            raise ValueError(f"label {pair_id} is not a frozen candidate")
        if pair_id in seen:
            raise ValueError(f"label {pair_id} is duplicated")
        seen.add(pair_id)
        if label["label"] not in LABELS:
            raise ValueError(f"label {pair_id} has invalid label {label['label']}")
        if not label.get("rationale"):
            raise ValueError(f"label {pair_id} needs a rationale")


def metrics(data: dict[str, Any]) -> dict[str, Any]:
    by_id = {c["pair_id"]: c for c in data["candidates"]}
    overall_total = len(data["labels"])
    overall_dup = 0
    per_tier: dict[str, dict[str, Any]] = {}
    for tier in TIERS:
        rows = [label for label in data["labels"] if by_id[label["pair_id"]]["tier"] == tier]
        duplicates = sum(1 for label in rows if label["label"] == "duplicate")
        per_tier[tier] = {
            "labeled": len(rows),
            "duplicate": duplicates,
            "precision": round(duplicates / len(rows), 4) if rows else None,
        }
        overall_dup += duplicates
    candidates_by_tier = {
        tier: sum(1 for c in data["candidates"] if c["tier"] == tier) for tier in TIERS
    }
    return {
        "overall": {
            "labeled": overall_total,
            "duplicate": overall_dup,
            "precision": round(overall_dup / overall_total, 4) if overall_total else None,
        },
        "per_tier": per_tier,
        "candidates_by_tier": candidates_by_tier,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("field_json", type=Path)
    parser.add_argument("--check", action="store_true", help="verify stored metrics drift-free")
    args = parser.parse_args()
    data = load(args.field_json)
    validate(data)
    computed = metrics(data)
    if args.check and data.get("metrics") != computed:
        print("field metrics drift:", json.dumps({"stored": data.get("metrics"), "computed": computed}))
        return 1
    print(json.dumps({"head": data["head"], **computed}, indent=1))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"reuse_field_run: {error}", file=sys.stderr)
        sys.exit(2)
