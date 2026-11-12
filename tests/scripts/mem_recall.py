#!/usr/bin/env python3
"""Phase 3 — `eg mem query` recall against the frozen project-memory oracle.

The fixture (tests/fixtures/memory/oracle-v2.json) freezes 30 notes and 20
queries with their expected keys, plus the fixed seed for the negative
control. The script seeds a throwaway repo through `eg mem write`, runs every
frozen query, and reports:

- recall@limit with a Wilson 95% interval (the shared `wilson` helper);
- a negative control: the term-to-key mapping is deterministically shuffled
  with the frozen seed and re-scored against the SAME retrievals — a real
  ranking must score clearly below the shuffled mapping, so a trivially
  passing store (any query returning everything) cannot clear the gate.

Gate (exit 0): recall >= threshold AND negative-control recall < threshold.
The oracle is frozen before the run; results are recorded as JSON, per the
standing experiment-plan principles (frozen oracle, negative control,
stdlib-only, no new dependencies).

Usage:
    python3 -I tests/scripts/mem_recall.py \
        --eg target/release/eg \
        --fixture tests/fixtures/memory/oracle-v2.json \
        --json docs/baselines/memory-phase3.json
"""
from __future__ import annotations

import argparse
import hashlib
import json
import random
import shutil
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from precision_recall import run_eg, wilson

CAVEAT_PREFIX = "eg: memory"
SHOWING_PREFIX = "showing "


def fixture_digest(fixture: dict) -> str:
    """SHA-256 over the canonical fixture body, excluding its own digest."""
    body = {k: v for k, v in fixture.items() if k != "sha256"}
    canon = json.dumps(body, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(canon).hexdigest()


def parse_query_keys(stdout: str) -> list[str]:
    """Row headers read `key rev kind scope created_at [tag]`; body lines are
    indented, the caveat and cap lines never start with a row header."""
    keys: list[str] = []
    for raw in stdout.splitlines():
        if raw.startswith("  ") or not raw or raw.startswith((CAVEAT_PREFIX, SHOWING_PREFIX)):
            continue
        keys.append(raw.split(" ")[0])
    return keys


def seed_notes(eg: str, root: Path, notes: list[dict]) -> None:
    for note in notes:
        proc = run_eg(
            eg,
            root,
            [
                "mem", "write", note["key"],
                "--body", note["body"],
                "--kind", note["kind"],
                "--tags", note.get("tags", ""),
                "--refs", note.get("refs", ""),
            ],
        )
        if proc.returncode != 0:
            raise RuntimeError(
                f"seeding '{note['key']}' failed ({proc.returncode}): {proc.stderr.strip()}"
            )


def score(retrievals: list[list[str]], expectations: list[list[str]]) -> tuple[int, int, list[bool]]:
    hits: list[bool] = []
    for got, want in zip(retrievals, expectations):
        hits.append(any(w in got for w in want))
    k = sum(hits)
    return k, len(hits), hits


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--eg", required=True, help="Path to the eg binary")
    ap.add_argument(
        "--fixture",
        default=str(Path(__file__).resolve().parents[1] / "fixtures" / "memory" / "oracle-v2.json"),
        help="Frozen oracle fixture (default: tests/fixtures/memory/oracle-v2.json)",
    )
    ap.add_argument("--limit", type=int, default=None, help="Override the fixture's recall@k depth")
    ap.add_argument("--threshold", type=float, default=None, help="Override the fixture's gate threshold")
    ap.add_argument("--json", default=None, help="Write the evidence JSON to this path")
    args = ap.parse_args()

    fixture = json.loads(Path(args.fixture).read_text(encoding="utf-8"))
    expected_digest = fixture.get("sha256")
    actual_digest = fixture_digest(fixture)
    if expected_digest and expected_digest != actual_digest:
        print(
            f"fixture sha256 mismatch: expected {expected_digest}, got {actual_digest}",
            file=sys.stderr,
        )
        return 2
    limit = args.limit if args.limit is not None else int(fixture.get("limit", 5))
    threshold = args.threshold if args.threshold is not None else float(fixture.get("threshold", 0.8))
    seed = int(fixture["seed"])

    root = Path(tempfile.mkdtemp(prefix="eg-mem-recall-"))
    try:
        seed_notes(args.eg, root, fixture["notes"])
        retrievals: list[list[str]] = []
        for query in fixture["queries"]:
            proc = run_eg(args.eg, root, ["mem", "query", query["terms"], "--limit", str(limit)])
            if proc.returncode == 1:
                # No-results is a miss, not a harness failure: the frozen
                # oracle must be able to score a regression instead of aborting.
                retrievals.append([])
                continue
            if proc.returncode != 0:
                raise RuntimeError(
                    f"query '{query['terms']}' failed ({proc.returncode}): {proc.stderr.strip()}"
                )
            retrievals.append(parse_query_keys(proc.stdout))
    finally:
        shutil.rmtree(root, ignore_errors=True)

    expects = [[w] if isinstance(w, str) else list(w) for w in (q["expect"] for q in fixture["queries"])]
    k, n, hits = score(retrievals, expects)
    recall, lo, hi = wilson(k, n)

    # Negative control: the frozen shuffle reassigns expected keys to queries;
    # the SAME retrievals scored against the wrong mapping must sit below the
    # gate, or the store is returning everything and proves nothing.
    shuffled = [w for w in expects]
    random.Random(seed).shuffle(shuffled)
    k_neg, n_neg, _ = score(retrievals, shuffled)
    recall_neg, lo_neg, hi_neg = wilson(k_neg, n_neg)

    gate_pass = recall >= threshold and recall_neg < threshold
    report = {
        "fixture": str(args.fixture),
        "fixture_sha256": actual_digest,
        "limit": limit,
        "threshold": threshold,
        "seed": seed,
        "queries": n,
        "hits": k,
        "recall": round(recall, 4),
        "wilson95": [round(lo, 4), round(hi, 4)],
        "negative_control": {
            "recall": round(recall_neg, 4),
            "wilson95": [round(lo_neg, 4), round(hi_neg, 4)],
            "below_threshold": recall_neg < threshold,
        },
        "per_query": [
            {"terms": q["terms"], "expect": e, "hit": h, "got": g[: limit]}
            for q, e, h, g in zip(fixture["queries"], expects, hits, retrievals)
        ],
        "gate": "PASS" if gate_pass else "FAIL",
    }

    print(
        f"eg mem recall@{limit}: {k}/{n} = {recall:.3f} "
        f"(Wilson 95% [{lo:.3f}, {hi:.3f}]), threshold {threshold:.2f}"
    )
    print(
        f"negative control (seed {seed}): {k_neg}/{n_neg} = {recall_neg:.3f} "
        f"(must stay below {threshold:.2f})"
    )
    print(f"gate: {report['gate']}")
    if args.json:
        out = Path(args.json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"evidence written: {args.json}")
    return 0 if gate_pass else 1


if __name__ == "__main__":
    sys.exit(main())
