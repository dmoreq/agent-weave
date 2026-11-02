#!/usr/bin/env python3
"""Deterministic seeded-clone fixture for Stage 3 (utility A/B of kg clones).

Writes /tmp/utility-corpus-2026-11-02/clones: four seeded clone pairs (the
two files of each pair contain an identical 9-line block at the same line
range), one near-duplicate (same shape, all identifiers renamed -> NOT a
normalized-text clone), and three unique decoys. Emits expected/C1.json.
"""
import json
from pathlib import Path

OUT = Path("/tmp/utility-corpus-2026-11-02/clones")
EXP = Path("/Users/quy.doan/Workspace/kg/docs/baselines/expected")
HEADER = 2  # two UNIQUE header lines per file (raw = normalized block)


def block(tag: str) -> str:
    return (
        f"def process_{tag}(items):\n"
        f"    acc_{tag} = 0\n"
        f"    limit_{tag} = 100\n"
        f"    for item in items:\n"
        f"        acc_{tag} += scale_{tag}(item)\n"
        f"        if acc_{tag} > limit_{tag}:\n"
        f"            acc_{tag} = limit_{tag}\n"
        f"    total_{tag} = acc_{tag}\n"
        f"    return finish_{tag}(total_{tag})\n"
    )


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    pairs = []
    for i in range(1, 5):
        a = f"pair{i:02}_a.py"
        b = f"pair{i:02}_b.py"
        body = block(f"p{i}")
        (OUT / a).write_text(f"# variant a of seeded pair {i}\n# a-side fixture, do not edit\n" + body)
        (OUT / b).write_text(f"# variant b of seeded pair {i}\n# b-side fixture, do not edit\n" + body)
        span = f"{HEADER + 1}-{HEADER + body.count(chr(10))}"
        pairs.append([f"{a}:{span}", f"{b}:{span}"])
    # Near-duplicate: same shape, every identifier renamed.
    near = block("near")
    for word in ("process", "acc", "limit", "scale", "total", "finish"):
        near = near.replace(word, word + "_x")
    (OUT / "near.py").write_text("# looks similar, identifiers differ\n# near-side fixture, do not edit\n" + near)
    # Decoys: unique code, no 5-line window shared with anything.
    for d in range(1, 4):
        (OUT / f"unique{d}.py").write_text(
            f"def solo_{d}(value):\n"
            f"    step_{d} = value * {d + 1}\n"
            f"    if step_{d} % {d + 2} == 0:\n"
            f"        return step_{d} - {d}\n"
            f"    return step_{d} + {d * 7}\n"
        )
    (EXP / "C1.json").write_text(json.dumps({"pairs": pairs}, indent=1) + "\n")
    (EXP / "C2.json").write_text(json.dumps({"exists": False, "pairs": []}) + "\n")
    print(json.dumps({"pairs": pairs}, indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
