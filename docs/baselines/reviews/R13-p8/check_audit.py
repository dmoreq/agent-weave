#!/usr/bin/env python3
"""Validate R13 six-lens audit coverage and independence."""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
FEATURES = [f"F{i}" for i in range(1, 12)]
DECISIONS = ("KEEP", "IMPROVE", "DEMOTE", "REMOVE", "DEFER", "OWNER")
REPORTS = [f"L{i}.md" for i in range(1, 7)]
BANNED = ("R8-P8", "R9-P8", "R10-P8", "R11-P8", "R12-P8",
          "p8-utility-2026-11-03.md §4", "p8-utility-2026-11-03.md §6")


def main() -> int:
    errors: list[str] = []
    for report in REPORTS:
        path = ROOT / "reports" / report
        if not path.exists():
            errors.append(f"missing {path}")
            continue
        text = path.read_text(encoding="utf-8")
        if report != "L6.md":
            for feature in FEATURES:
                if not re.search(rf"\b{feature}\b", text):
                    errors.append(f"{report}: missing {feature}")
            for banned in BANNED:
                if banned in text:
                    errors.append(f"{report}: cites banned verdict source {banned}")
            if not any(decision in text for decision in DECISIONS):
                errors.append(f"{report}: no decision vocabulary")
        else:
            if "TRUSTWORTHY" not in text and "INSUFFICIENT" not in text:
                errors.append("L6.md: no integrity verdict")
    matrix = ROOT / "decision-matrix.md"
    if not matrix.exists():
        errors.append("missing decision-matrix.md")
    else:
        text = matrix.read_text(encoding="utf-8")
        for feature in FEATURES:
            if not re.search(rf"\b{feature}\b", text):
                errors.append(f"decision-matrix.md: missing {feature}")
        if "INSUFFICIENT" not in text:
            errors.append("decision-matrix.md: missing L6 gate")
    if errors:
        for error in errors:
            print(f"AUDIT: {error}")
        return 1
    print("OK: six reports cover F1-F11; decisions present; L6 gate present")
    return 0


if __name__ == "__main__":
    sys.exit(main())
