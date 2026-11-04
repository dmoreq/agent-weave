#!/usr/bin/env python3
"""Regression tests for the public evaluation scorers."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent


class RecallScorerTests(unittest.TestCase):
    def run_scorer(self, script, fake_output, exit_code, input_text):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "aw"
            binary.write_text(f"#!/bin/sh\nprintf '%s\\n' '{fake_output}'\nexit {exit_code}\n")
            binary.chmod(0o700)
            source = root / "input"
            source.write_text(input_text)
            if script == "search_recall.py":
                args = ["--queries", str(source), str(binary), "--aw-root", directory,
                        "--vihap-root", directory]
            else:
                args = [directory, str(binary), str(source)]
            return subprocess.run([sys.executable, "-I", str(SCRIPTS / script), *args],
                                  capture_output=True, text=True, timeout=10)

    def test_search_requires_qualified_suffix_and_distinct_results(self):
        queries = json.dumps({"repo": "aw", "query": "x", "expect": ["pkg.foo", "other.foo"]})
        result = self.run_scorer("search_recall.py", "src.py:1 foo", 0, queries)
        self.assertIn("pooled symbol recall: 0/2", result.stdout)
        self.assertEqual(result.returncode, 1)
        result = self.run_scorer("search_recall.py", "src.py:1 pkg.foo", 0, queries)
        self.assertIn("pooled symbol recall: 1/2", result.stdout)

    def test_search_fails_on_command_error(self):
        queries = json.dumps({"repo": "aw", "query": "x", "expect": ["foo"]})
        result = self.run_scorer("search_recall.py", "", 6, queries)
        self.assertEqual(result.returncode, 1)
        self.assertIn("exit 6", result.stderr)

    def test_ambiguous_candidate_is_not_a_caller_site(self):
        result = self.run_scorer("precision_recall.py", "mod.py:1 pkg.foo", 2,
                                 "| G1 | caller.py:7 | pkg.foo | call | [x] |\n")
        self.assertIn("| recall | 1/1", result.stdout)
        self.assertIn("| caller-site recall (no ambiguity credit) | 0/1", result.stdout)
        self.assertEqual(result.returncode, 1)


if __name__ == "__main__":
    unittest.main()
