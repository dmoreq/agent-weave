#!/usr/bin/env python3
"""Stdlib tests for the reuse oracle ledger and McNemar evaluator."""

from __future__ import annotations

import csv
import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("reuse_oracle.py")
SPEC = importlib.util.spec_from_file_location("reuse_oracle", SCRIPT)
ORACLE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(ORACLE)


def write_csv(path: Path, header: list[str], rows: list[tuple]) -> None:
    with path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(header)
        writer.writerows(rows)


def ledger_rows() -> list[tuple]:
    rows = []
    index = 0

    def add(count, split, label, tier, confidence, baseline):
        nonlocal index
        for _ in range(count):
            rows.append((f"p{index}", split, label, tier, confidence, baseline))
            index += 1

    add(50, "tune", "distinct", "tune", 0.1, 0.1)
    add(30, "confirmation", "duplicate", "eligible", 0.9, 0.4)
    add(15, "confirmation", "duplicate", "d4", 0.8, 0.4)
    add(15, "confirmation", "duplicate", "d3", 0.7, 0.4)
    add(30, "confirmation", "distinct", "negative", 0.2, 0.6)
    add(20, "holdout", "duplicate", "holdout", 0.9, 0.9)
    return rows


class OracleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        rows = ledger_rows()
        self.labels = self.root / "labels.csv"
        self.predictions = self.root / "predictions.csv"
        self.baseline = self.root / "baseline.csv"
        write_csv(self.labels, ["pair_id", "split", "label", "tier"], [r[:4] for r in rows])
        write_csv(self.predictions, ["pair_id", "confidence"], [(r[0], r[4]) for r in rows])
        write_csv(self.baseline, ["pair_id", "confidence"], [(r[0], r[5]) for r in rows])

    def tearDown(self):
        self.temp.cleanup()

    def test_valid_ledger_and_mcnemar(self):
        result = ORACLE.validate(self.labels)
        self.assertEqual(result["total"], 160)
        self.assertEqual(result["negative"], 30)
        evaluation = ORACLE.evaluate(
            self.labels, self.predictions, 0.5, self.baseline
        )
        self.assertEqual(evaluation["tp"], 60)
        self.assertEqual(evaluation["tn"], 30)
        self.assertEqual(evaluation["mcnemar"]["b"], 90)
        self.assertEqual(evaluation["mcnemar"]["c"], 0)
        self.assertGreaterEqual(evaluation["mcnemar"]["power"], 0.99)
        self.assertGreater(evaluation["mcnemar"]["mdd"], 0.0)

    def test_holdout_split_is_scored(self):
        evaluation = ORACLE.evaluate(
            self.labels, self.predictions, 0.5, None, 0.05, 0.8, "holdout"
        )
        self.assertEqual(evaluation["split"], "holdout")
        self.assertEqual(evaluation["confirmation_pairs"], 20)
        self.assertEqual(evaluation["tp"], 20)

    def test_missing_negative_tier_is_rejected(self):
        rows = [r for r in ledger_rows() if r[3] != "negative"]
        write_csv(self.labels, ["pair_id", "split", "label", "tier"], [r[:4] for r in rows])
        with self.assertRaises(ValueError):
            ORACLE.validate(self.labels)

    def test_tune_row_must_use_tune_tier(self):
        rows = ledger_rows()
        rows[0] = (rows[0][0], "tune", "duplicate", "eligible", 0.9, 0.4)
        write_csv(self.labels, ["pair_id", "split", "label", "tier"], [r[:4] for r in rows])
        with self.assertRaises(ValueError):
            ORACLE.validate(self.labels)


if __name__ == "__main__":
    unittest.main()
