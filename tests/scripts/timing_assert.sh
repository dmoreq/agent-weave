#!/usr/bin/env bash
# Assert a command's best-of-N wall time is under a millisecond budget.
# One Python harness measures every run, so interpreter startup is not charged
# per sample and semantic exits (0/1/2) do not abort before measurement.
# Usage: timing_assert.sh <runs> <label> <budget-ms> <command...>
set -uo pipefail

if [[ $# -lt 4 ]]; then
    echo "usage: $0 <runs> <label> <budget-ms> <command...>" >&2
    exit 2
fi

python3 -I - "$@" <<'PY'
import subprocess
import sys
import time

runs = int(sys.argv[1])
label = sys.argv[2]
budget_ms = int(sys.argv[3])
cmd = sys.argv[4:]
if runs < 1:
    print("usage: runs must be >= 1", file=sys.stderr)
    raise SystemExit(2)

best = None
for _ in range(runs):
    start = time.perf_counter_ns()
    result = subprocess.run(
        cmd,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    elapsed_ms = (time.perf_counter_ns() - start) / 1_000_000
    if result.returncode not in (0, 1, 2):
        print(
            f"timing: {label} command exited {result.returncode}, not measured",
            file=sys.stderr,
        )
        raise SystemExit(2)
    if best is None or elapsed_ms < best:
        best = elapsed_ms

display = f"{best:.1f}"
print(f"timing: {label} best-of-{runs} {display}ms (budget {budget_ms}ms)")
if best > budget_ms:
    print(f"FAIL: {label} {display}ms > {budget_ms}ms", file=sys.stderr)
    raise SystemExit(1)
PY
