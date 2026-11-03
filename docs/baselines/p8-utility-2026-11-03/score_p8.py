#!/usr/bin/env python3
"""Score P8 utility answers against the frozen source-derived oracles.

Usage:
  python3 -I score_p8.py [--answers DIR] [--runids ID ...]
Prints a markdown table; exit 1 when any present answer fails.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
EXPECTED_DIR = HERE / "expected"


def norm(task: str, value: str) -> str:
    text = re.sub(r"\s+", " ", value.strip())
    # Test/implementer rows may use the bare symbol name; the file:line is
    # the identity. Normalize both sides to the short name for set equality.
    if task in {"T1", "H1"} and " " in text:
        ptr, name = text.split(" ", 1)
        text = f"{ptr} {name.rsplit('.', 1)[-1]}"
    return text


def score_answer(task: str, answer_path: Path) -> tuple[bool, str]:
    try:
        answer = json.loads(answer_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        return False, f"invalid JSON: {exc}"
    expected = json.loads((EXPECTED_DIR / f"{task}.json").read_text())
    if not isinstance(answer, dict):
        return False, "answer is not an object"
    if answer.get("task") != task:
        return False, f"task field {answer.get('task')!r} != {task!r}"
    items = answer.get("items")
    if not isinstance(items, list):
        return False, "items is not a list"
    if not isinstance(answer.get("answer"), str) or not answer["answer"].strip():
        return False, "missing answer prose"
    got_items = [norm(task, str(item)) for item in items]
    if len(got_items) != len(set(got_items)):
        return False, "duplicate items after normalization"
    want_items = [norm(task, str(item)) for item in expected["items"]]
    if len(want_items) != len(set(want_items)):
        return False, "oracle contains duplicate items"
    got = sorted(set(got_items))
    want = sorted(set(want_items))
    if got != want:
        missing = sorted(set(want) - set(got))
        extra = sorted(set(got) - set(want))
        detail = []
        if missing:
            detail.append(f"missing={missing}")
        if extra:
            detail.append(f"extra={extra}")
        return False, "; ".join(detail)
    return True, "exact set match"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--answers", type=Path, default=HERE / "answers")
    ap.add_argument("--runids", nargs="*", default=[])
    args = ap.parse_args()
    answers = sorted(args.answers.glob("*.json")) if args.answers.exists() else []
    if args.runids:
        wanted = set(args.runids)
        answers = [path for path in answers if path.stem in wanted]
        missing = sorted(wanted - {path.stem for path in answers})
        if missing:
            print(f"missing runids: {', '.join(missing)}")
            return 1
    if not answers:
        print("| runid | task | correct | detail |")
        print("|---|---|---|---|")
        print("\nNo answers found.")
        return 0
    print("| runid | task | correct | detail |")
    print("|---|---|---|---|")
    failures = 0
    for path in answers:
        stem = path.stem
        task = stem.rsplit("-", 1)[-1]
        ok, detail = score_answer(task, path)
        failures += not ok
        print(f"| {stem} | {task} | {'OK' if ok else 'FAIL'} | {detail} |")
    print(f"\nscored: {len(answers)} answers, {failures} failures")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
