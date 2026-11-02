#!/usr/bin/env bash
# G5 dependency policy: exact allowlist, not a network-family denylist
# (R6e Blocker 3). Any package in Cargo.lock that is not listed fails the gate.
set -euo pipefail
LOCK="${1:-Cargo.lock}"
LIST="$(dirname "$0")/allowlist.txt"
set +e
extra=$(python3 - "$LOCK" "$LIST" <<'PY'
import re, sys
lock = open(sys.argv[1]).read()
allowed = set(open(sys.argv[2]).read().split())
names = set(re.findall(r'^name = "([^"]+)"$', lock, re.M))
for n in sorted(names - allowed):
    print(n)
PY
)
set -e
if [ -n "$extra" ]; then
  echo "G5 allowlist: packages not in tests/scripts/allowlist.txt:" >&2
  echo "$extra" >&2
  exit 1
fi
echo "G5 allowlist: every locked package is allowlisted — PASS"
