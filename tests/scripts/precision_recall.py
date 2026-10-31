#!/usr/bin/env python3
"""G2 — precision + recall of `kg callers` against the 30 frozen GT rows.

Recall: a GT row hits when kg's caller pointers for the row's callee qualname
contain the row's caller file:line; when kg exits 2 (ambiguous symbol), the
row hits only if the true callee qualname appears in the candidate list
(tasks.md scoring contract). Dynamic rows (G28) are excluded from both
denominators per spec.

Precision: every emitted pointer for the queried callees is verified against
the vihap working tree — the source line must contain the callee's short name
followed by "(" (a real call site). EXTRACTED-tier precision uses the same
verification (kg emits only EXTRACTED-tier pointers for these queries).

Wilson 95% intervals are computed from the binomial standard formulas.

Usage: precision_recall.py <vihap-root> <kg-binary> <ground-truth.md>
Exit 0 always; prints the gate verdict.
"""
import math
import re
import subprocess
import sys

ROW_RE = re.compile(
    r"^\|\s*(G\d+)\s*\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|\s*\[x\]\s*\|\s*$"
)


def wilson(k, n, z=1.96):
    if n == 0:
        return 0.0, 0.0, 0.0
    p = k / n
    denom = 1 + z * z / n
    center = p + z * z / (2 * n)
    spread = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n))
    return p, max(0.0, (center - spread) / denom), min(1.0, (center + spread) / denom)


def parse_gt(path):
    rows = []
    for line in open(path):
        m = ROW_RE.match(line.strip())
        if not m:
            continue
        gid, caller, callee, kind = m.groups()
        # Strip prose annotations from the callee cell ("... (note text)").
        callee = callee.split(" (")[0].strip()
        rows.append((gid, caller, callee, kind))
    return rows


def kg_callers(kg, root, qualname):
    proc = subprocess.run(
        [kg, "callers", qualname, "--limit", "10000", "--root", root],
        capture_output=True, text=True,
    )
    if proc.returncode == 0:
        pointers = set()
        for line in proc.stdout.splitlines():
            if line.startswith("kg:") or line.startswith("showing "):
                continue
            parts = line.strip().split(":")
            if len(parts) >= 2:
                pointers.add(f"{parts[0]}:{parts[1]}")
        return pointers, None
    if proc.returncode == 2:
        candidates = set()
        for line in proc.stdout.splitlines():
            if re.match(r"^\S+:\d+ \S+$", line.strip()):
                candidates.add(line.strip().split(" ", 1)[1])
        return None, candidates
    return set(), None


def main():
    root, kg, gt_path = sys.argv[1], sys.argv[2], sys.argv[3]
    rows = parse_gt(gt_path)
    scored = [r for r in rows if not r[2].startswith("(dynamic)")]
    dynamic = [r for r in rows if r[2].startswith("(dynamic)")]

    callee_pointers = {}
    for row in scored:
        callee = row[2]
        targets = [callee]
        if callee.endswith(".__init__"):
            targets.append(callee[: -len(".__init__")])
        for t in targets:
            if t not in callee_pointers:
                callee_pointers[t] = kg_callers(kg, root, t)

    hits = 0
    misses = []
    for gid, caller, callee, _ in scored:
        # Constructor rows: a class without an explicit __init__ keeps the
        # class symbol as the call target — accept the class qualname too.
        targets = [callee]
        if callee.endswith(".__init__"):
            targets.append(callee[: -len(".__init__")])
        target_data = [callee_pointers[t] for t in targets]
        hit = False
        for pointers, candidates in target_data:
            if candidates is not None and callee in candidates:
                hit = True  # true callee in candidate list (AMBIGUOUS contract)
            elif pointers is not None and caller in pointers:
                hit = True
        if hit:
            hits += 1
        else:
            misses.append(f"{gid}: {caller} not in callers of {callee}")

    recall, rlo, rhi = wilson(hits, len(scored))

    # Precision: verify every emitted pointer against the source. A callee
    # qualname ending in __init__ appears at call sites as the CLASS name.
    verified = 0
    total_checked = 0
    bad = []
    for callee, (pointers, _c) in callee_pointers.items():
        if pointers is None:
            continue
        short = callee.rsplit(".", 1)[-1]
        if short in ("__init__", "constructor"):
            short = callee.rsplit(".", 2)[-2]
        for ptr in sorted(pointers):
            total_checked += 1
            path, line = ptr.split(":", 1)
            try:
                src = open(f"{root}/{path}").read().splitlines()
                text_line = src[int(line) - 1]
            except (OSError, IndexError):
                bad.append(f"{ptr} (unreadable)")
                continue
            if re.search(rf"\b{re.escape(short)}\(", text_line):
                verified += 1
            else:
                bad.append(ptr)
    precision, plo, phi = wilson(verified, total_checked)

    print("| gate | value | wilson 95% | target | verdict |")
    print("|------|-------|------------|--------|---------|")
    print(f"| recall | {hits}/{len(scored)} = {recall:.0%} | [{rlo:.0%}, {rhi:.0%}] | >= 70% | {'PASS' if recall >= 0.70 else 'FAIL'} |")
    print(f"| precision (call-site verified) | {verified}/{total_checked} = {precision:.0%} | [{plo:.0%}, {phi:.0%}] | >= 80% | {'PASS' if precision >= 0.80 else 'FAIL'} |")
    print(f"| EXTRACTED-tier precision | {precision:.0%} | [{plo:.0%}, {phi:.0%}] | >= 95% | {'PASS' if precision >= 0.95 else 'FAIL'} |")
    print(f"dynamic rows excluded: {', '.join(r[0] for r in dynamic) or 'none'}")
    if misses:
        print("recall misses:")
        for m in misses:
            print(f"  {m}")
    if bad:
        print("precision failures:")
        for b in bad[:10]:
            print(f"  {b}")
    gate = recall >= 0.70 and precision >= 0.80
    print(f"G2 VERDICT: {'PASS' if gate else 'FAIL'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
