#!/usr/bin/env bash
# G5 invariant 1 (macOS): the build must complete with network fully denied,
# and the sandbox must actually block a network probe (negative control).
# R6e Blocker 1: the previous version wrote a profile but never applied it.
set -euo pipefail
KG="$1"; REPO="$2"
PROFILE="$(mktemp -t kg-seatbelt.XXXXXX).sb"
trap 'rm -f "$PROFILE"' EXIT
cat > "$PROFILE" <<'SB'
(version 1)
(allow default)
(deny network*)
SB

# Positive: build succeeds under the profile.
sandbox-exec -f "$PROFILE" "$KG" build --root "$REPO" >/dev/null

# Negative control: a network probe under the SAME profile must fail. If it
# succeeds, the sandbox is not enforcing anything and this gate is red.
if sandbox-exec -f "$PROFILE" /usr/bin/python3 - <<'PY' 2>/dev/null
import socket
socket.create_connection(("1.1.1.1", 443), timeout=3)
print("NETWORK REACHED")
PY
then
  echo "G5 seatbelt: negative control REACHED the network — sandbox not enforced" >&2
  exit 1
fi
echo "G5 seatbelt: build completed with network denied (negative control blocked) — PASS"
