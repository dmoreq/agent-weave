#!/usr/bin/env python3
"""Unit tests for bench/run.py. Stdlib only; no eg binary required."""

import importlib.util
import unittest
from pathlib import Path

RUN = Path(__file__).with_name("run.py")
SPEC = importlib.util.spec_from_file_location("eg_bench_run", RUN)
BENCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCH)


class BenchParserTests(unittest.TestCase):
    def test_parse_pointers_skips_headers(self):
        stdout = "eg: header\nshop/a.py:3 shop.a.f\nshowing 1 of 2\n"
        self.assertEqual(BENCH.parse_pointers(stdout), [("shop/a.py", "shop.a.f")])

    def test_parse_context_definition_line(self):
        stdout = "shop/a.py:4-5 function shop.a.f\n"
        self.assertEqual(
            BENCH.parse_pointers(stdout), [("shop/a.py", "function shop.a.f")]
        )

    def test_parse_chain(self):
        stdout = "eg: chain a -> b -> c (2 hops)\n"
        self.assertEqual(BENCH.parse_chain(stdout), ["a", "b", "c"])

    def test_subset_and_forbid(self):
        record = {
            "expect": [{"path": "a.py", "detail_contains": "x"}],
            "forbid": [{"path": "b.py"}],
            "exit": 0,
        }
        problems, count = BENCH.check_query(record, "a.py:1 x\n", "", 0)
        self.assertEqual(problems, [])
        self.assertEqual(count, 1)

    def test_missing_pointer(self):
        record = {"expect": [{"path": "a.py"}], "exit": 0}
        problems, _ = BENCH.check_query(record, "b.py:1 y\n", "", 0)
        self.assertTrue(any("missing" in problem for problem in problems))

    def test_forbidden_pointer(self):
        record = {"expect": [], "forbid": [{"path": "b.py"}], "exit": 0}
        problems, _ = BENCH.check_query(record, "b.py:1 y\n", "", 0)
        self.assertTrue(any("forbidden" in problem for problem in problems))

    def test_exit_mismatch(self):
        record = {"expect": [], "exit": 1}
        problems, _ = BENCH.check_query(record, "", "no-results", 0)
        self.assertTrue(any("exit" in problem for problem in problems))


if __name__ == "__main__":
    unittest.main()
