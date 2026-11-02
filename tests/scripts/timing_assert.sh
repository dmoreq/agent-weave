#!/usr/bin/env bash
# Assert a command's best-of-N wall time is under a millisecond budget.
# Usage: timing_assert.sh <runs> <label> <budget-ms> <command...>
set -euo pipefail

if [[ $# -lt 4 ]]; then
    echo "usage: $0 <runs> <label> <budget-ms> <command...>" >&2
    exit 2
fi

runs="$1"
label="$2"
budget_ms="$3"
shift 3

best=""
for _ in $(seq "$runs"); do
    start=$(python3 -c 'import time; print(int(time.time() * 1000))')
    "$@" >/dev/null 2>/dev/null
    end=$(python3 -c 'import time; print(int(time.time() * 1000))')
    ms=$((end - start))
    if [[ -z "$best" || "$ms" -lt "$best" ]]; then
        best="$ms"
    fi
done

echo "timing: $label best-of-$runs ${best}ms (budget ${budget_ms}ms)"
if [[ "$best" -gt "$budget_ms" ]]; then
    echo "FAIL: $label ${best}ms > ${budget_ms}ms" >&2
    exit 1
fi
