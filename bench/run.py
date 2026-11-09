#!/usr/bin/env python3
"""Public navigation regression benchmark for eg.

Usage:
    python3 -I bench/run.py target/release/eg

The benchmark copies each corpus root to a temporary directory, builds the
index with the given eg binary, runs the committed queries, and checks that
expected pointers are present and forbidden pointers are absent. It is a
regression corpus for static navigation output; it does not measure agent
utility, token savings, or dynamic-dispatch coverage.
"""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
CORPUS = HERE / "corpus"
QUERIES = HERE / "queries.jsonl"

# A pointer line starts with path:line (optionally a line range) followed by
# an optional detail. Headers and section labels are filtered separately.
POINTER_RE = re.compile(r"^(\S+?):(\d+)(?:-\d+)?\s*(.*)$")
SKIP_PREFIXES = ("eg:", "showing ", "callers (", "callees (")


def parse_pointers(stdout: str) -> list[tuple[str, str]]:
    """Return (path, detail) pairs from pointer-shaped stdout lines."""
    pointers: list[tuple[str, str]] = []
    for raw in stdout.splitlines():
        line = raw.strip()
        if not line or line.startswith(SKIP_PREFIXES):
            continue
        match = POINTER_RE.match(line)
        if match:
            pointers.append((match.group(1), match.group(3).strip()))
    return pointers


def parse_chain(stdout: str) -> list[str]:
    """Return the qualnames from the final `eg: chain ...` line, if present."""
    for line in stdout.splitlines():
        if line.startswith("eg: chain "):
            body = line[len("eg: chain ") :]
            body = body.rsplit(" (", 1)[0]
            return [part.strip() for part in body.split(" -> ")]
    return []


def matches(pointer: tuple[str, str], expected: dict) -> bool:
    path, detail = pointer
    if path != expected["path"]:
        return False
    needle = expected.get("detail_contains")
    return needle is None or needle in detail


def check_query(
    record: dict, stdout: str, stderr: str, exit_code: int
) -> tuple[list[str], int]:
    """Check one query result. Returns (problems, pointer_count)."""
    problems: list[str] = []
    expected_exit = record.get("exit", 0)
    if exit_code != expected_exit:
        problems.append(f"exit {exit_code} != {expected_exit}")

    if "expect_chain" in record:
        chain = parse_chain(stdout)
        if chain != record["expect_chain"]:
            problems.append(f"chain {chain!r} != {record['expect_chain']!r}")

    pointers = parse_pointers(stdout)
    for expected in record.get("expect", []):
        if not any(matches(pointer, expected) for pointer in pointers):
            problems.append(f"missing {expected}")
    for forbidden in record.get("forbid", []):
        if any(matches(pointer, forbidden) for pointer in pointers):
            problems.append(f"forbidden {forbidden}")

    needle = record.get("stderr_contains")
    if needle and needle not in stderr:
        problems.append(f"stderr missing {needle!r}")
    return problems, len(pointers)


def run_eg(eg: str, root: Path, args: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        [eg, "--root", str(root), "--no-journal", *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=60,
    )


def main(argv: list[str]) -> int:
    eg = argv[1] if len(argv) > 1 else "eg"
    records = [
        json.loads(line)
        for line in QUERIES.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    if not records:
        print("eg bench: no queries found", file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory(prefix="eg-bench-") as tmp:
        roots: dict[str, Path] = {}
        for corpus in sorted({record["corpus"] for record in records}):
            source = CORPUS / corpus
            destination = Path(tmp) / corpus
            shutil.copytree(source, destination, ignore=shutil.ignore_patterns(".engram"))
            build = run_eg(eg, destination, ["build"])
            if build.returncode != 0:
                print(
                    f"eg bench: build failed for {corpus} (exit {build.returncode})",
                    file=sys.stderr,
                )
                print(build.stdout, end="", file=sys.stderr)
                print(build.stderr, end="", file=sys.stderr)
                return 1
            roots[corpus] = destination

        failures = 0
        print(f"{'id':<32} {'command':<10} {'pointers':>8}  verdict")
        for record in records:
            result = run_eg(
                eg, roots[record["corpus"]], [record["command"], *record.get("args", [])]
            )
            problems, pointer_count = check_query(
                record, result.stdout, result.stderr, result.returncode
            )
            verdict = "PASS" if not problems else "FAIL"
            print(f"{record['id']:<32} {record['command']:<10} {pointer_count:>8}  {verdict}")
            if problems:
                failures += 1
                for problem in problems:
                    print(f"    {problem}")
                stdout = result.stdout.strip().replace("\n", "\n            ")
                stderr = result.stderr.strip().replace("\n", "\n            ")
                if stdout:
                    print(f"    stdout: {stdout}")
                if stderr:
                    print(f"    stderr: {stderr}")

        total = len(records)
        print(f"eg bench: {total - failures}/{total} queries passed")
        return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
