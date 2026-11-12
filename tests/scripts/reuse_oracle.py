#!/usr/bin/env python3
"""Validate the Phase 2 reuse oracle label ledger.

Phase 1 ships no oracle gate. This stdlib-only validator fixes the arithmetic
before any Phase 2 model is tuned: 50 tune labels, 60 confirmation positives
(30 eligible + 15 D4 + 15 D3), 30 confirmation negatives, and 20 separate
holdout labels, 160 total.

CSV columns: pair_id,split,label,tier
  split: tune | confirmation | holdout
  label: duplicate | distinct
  tier:  eligible | d4 | d3 | negative | holdout
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
from collections import Counter
from pathlib import Path


def validate(path: Path) -> dict:
    rows = list(csv.DictReader(path.open(newline="", encoding="utf-8")))
    required = {"pair_id", "split", "label", "tier"}
    if not rows:
        raise ValueError("labels file is empty")
    missing = required.difference(rows[0])
    if missing:
        raise ValueError(f"missing columns: {sorted(missing)}")

    ids = {row["pair_id"] for row in rows}
    if len(ids) != len(rows):
        raise ValueError("pair_id values must be unique")

    split = Counter(row["split"] for row in rows)
    tier = Counter(row["tier"] for row in rows)
    confirmation = [row for row in rows if row["split"] == "confirmation"]
    confirmation_labels = Counter(row["label"] for row in confirmation)

    expected = {
        "total": 160,
        "tune": 50,
        "confirmation": 90,
        "confirmation_duplicate": 60,
        "confirmation_distinct": 30,
        "eligible": 30,
        "d4": 15,
        "d3": 15,
        "holdout": 20,
    }
    actual = {
        "total": len(rows),
        "tune": split["tune"],
        "confirmation": split["confirmation"],
        "confirmation_duplicate": confirmation_labels["duplicate"],
        "confirmation_distinct": confirmation_labels["distinct"],
        "eligible": tier["eligible"],
        "d4": tier["d4"],
        "d3": tier["d3"],
        "holdout": split["holdout"],
    }
    mismatches = {k: {"expected": expected[k], "actual": actual[k]} for k in expected if expected[k] != actual[k]}
    if mismatches:
        raise ValueError(f"oracle ledger mismatch: {json.dumps(mismatches, sort_keys=True)}")
    if set(split) - {"tune", "confirmation", "holdout"}:
        raise ValueError(f"unknown split values: {sorted(set(split) - {'tune', 'confirmation', 'holdout'})}")
    if set(tier) - {"eligible", "d4", "d3", "negative", "holdout", "tune"}:
        raise ValueError(f"unknown tier values: {sorted(set(tier) - {'eligible', 'd4', 'd3', 'negative', 'holdout', 'tune'})}")
    return {"labels": str(path), **actual}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--labels", required=True, type=Path)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    try:
        result = validate(args.labels)
    except (OSError, ValueError) as exc:
        print(f"reuse_oracle: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True) if args.json else result)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
