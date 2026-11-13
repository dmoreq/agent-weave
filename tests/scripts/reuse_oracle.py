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

Optional `--predictions FILE` reads `pair_id,confidence` and reports
confirmation precision, recall, confusion counts, and the Wilson upper bound.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
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


def wilson_upper(successes: int, total: int, z: float = 1.96) -> float:
    if total == 0:
        return 1.0
    p = successes / total
    denom = 1 + z * z / total
    centre = p + z * z / (2 * total)
    margin = z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total))
    return min(1.0, (centre + margin) / denom)


def evaluate(labels_path: Path, predictions_path: Path, threshold: float) -> dict:
    labels = list(csv.DictReader(labels_path.open(newline="", encoding="utf-8")))
    predictions = {
        row["pair_id"]: float(row["confidence"])
        for row in csv.DictReader(predictions_path.open(newline="", encoding="utf-8"))
    }
    confirmation = [row for row in labels if row["split"] == "confirmation"]
    tp = fp = fn = tn = 0
    missing = 0
    for row in confirmation:
        confidence = predictions.get(row["pair_id"])
        if confidence is None:
            missing += 1
            continue
        predicted = confidence >= threshold
        actual = row["label"] == "duplicate"
        if predicted and actual:
            tp += 1
        elif predicted and not actual:
            fp += 1
        elif not predicted and actual:
            fn += 1
        else:
            tn += 1
    precision = tp / (tp + fp) if tp + fp else 0.0
    recall = tp / (tp + fn) if tp + fn else 0.0
    return {
        "labels": str(labels_path),
        "predictions": str(predictions_path),
        "threshold": threshold,
        "confirmation_pairs": len(confirmation),
        "missing_predictions": missing,
        "tp": tp,
        "fp": fp,
        "fn": fn,
        "tn": tn,
        "precision": round(precision, 4),
        "recall": round(recall, 4),
        "precision_wilson_upper": round(wilson_upper(tp, tp + fp), 4),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--labels", required=True, type=Path)
    parser.add_argument("--predictions", type=Path)
    parser.add_argument("--threshold", default=0.5, type=float)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    try:
        result = validate(args.labels)
        if args.predictions is not None:
            if not 0.0 <= args.threshold <= 1.0:
                raise ValueError("threshold must be 0..1")
            result = evaluate(args.labels, args.predictions, args.threshold)
    except (OSError, ValueError) as exc:
        print(f"reuse_oracle: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True) if args.json else result)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
