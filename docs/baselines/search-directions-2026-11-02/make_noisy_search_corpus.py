#!/usr/bin/env python3
"""Deterministic noisy symbol-search corpus. Expected sets are written OUTSIDE the corpus."""
from __future__ import annotations
import json, sys
from pathlib import Path

FILES = 2000
ALPHA = '''# scoring helper
def process(value, *, mercer_street_tolerance: float = 0.0) -> dict[str, float]:
    result = {"value": float(value)}
    result["tolerance"] = mercer_street_tolerance
    return result
'''
BETA = '''# ward coercion helper
class HexWard:
    def __init__(self, value):
        self.value = value

def coerce(items, *, normalize_hex_ward: bool = False) -> list[HexWard]:
    if normalize_hex_ward:
        return [HexWard(str(item)) for item in items]
    return [HexWard(item) for item in items]

def bootstrap():
    return coerce([], normalize_hex_ward=True)
'''
def line_of(body: str, needle: str) -> int:
    for i, line in enumerate(body.splitlines(), 1):
        if needle in line:
            return i
    raise SystemExit(f"missing {needle}")

def noise_file(i: int, tests: bool) -> str:
    prefix = "test_" if tests else ""
    # mercer street tolerance / normalize hex ward appear in comments only;
    # each symbol matches at most two of the three query words.
    return f'''# noise {i}: mercer street tolerance normalize hex ward list HexWard bootstrap helper
def {prefix}process_{i:04}(value, *, mercer_tolerance: float = 0.0, avenue_name: str = ""):
    result = {{"value": value, "mercer": mercer_tolerance, "avenue": avenue_name}}
    return result

def {prefix}coerce_{i:04}(items, *, normalize_hex: bool = False) -> list[dict]:
    return [{{"hex": str(item)}} for item in items]

def {prefix}entry_{i:04}():
    return {prefix}coerce_{i:04}([])
'''

def main(out: str, expected_out: str) -> int:
    root = Path(out)
    exp = Path(expected_out)
    pkg = root / "pkg"
    tests = root / "tests"
    for d in (pkg, tests, exp):
        d.mkdir(parents=True, exist_ok=True)
    (pkg / "alpha.py").write_text(ALPHA)
    (pkg / "beta.py").write_text(BETA)
    for i in range(FILES):
        tests_dir = i % 3 == 0
        name = f"test_noise_{i:04}.py" if tests_dir else f"noise_{i:04}.py"
        (tests if tests_dir else pkg / name).write_text(noise_file(i, tests_dir)) if False else None
        (tests / name if tests_dir else pkg / name).write_text(noise_file(i, tests_dir))
    b1 = {"exists": True, "hops": 0,
          "route": [f"pkg/alpha.py:{line_of(ALPHA, 'def process')}"],
          "qualname": "pkg.alpha.process"}
    b2 = {"exists": True, "hops": 0,
          "route": [f"pkg/beta.py:{line_of(BETA, 'def coerce')}"],
          "qualname": "pkg.beta.coerce"}
    (exp / "B1.json").write_text(json.dumps(b1, indent=1) + "\n")
    (exp / "B2.json").write_text(json.dumps(b2, indent=1) + "\n")
    print(f"noisy corpus: {FILES + 2} files under {root}; expected under {exp}")
    return 0

if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1], sys.argv[2]))
