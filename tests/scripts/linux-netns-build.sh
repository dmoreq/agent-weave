#!/usr/bin/env bash
# G5 invariant 1 (Linux): the build must complete with network denied, and the
# namespace must actually block a probe. The positive control runs outside the
# namespace against a controlled local endpoint, so a setup/probe error cannot
# masquerade as successful isolation.
set -euo pipefail
AW="$1"; REPO="$2"
PORT="${ENGRAM_NETNS_PORT:-$((20000 + RANDOM % 20000))}"
SERVER_PID=""
cleanup() {
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null || true
}
trap cleanup EXIT

python3 -m http.server "$PORT" --bind 127.0.0.1 >/dev/null 2>&1 &
SERVER_PID=$!
disown "$SERVER_PID" 2>/dev/null || true
sleep 0.5
if ! python3 - "$PORT" <<'PY'
import socket, sys
socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=3)
PY
then
  echo "G5 netns: positive control cannot reach the local endpoint — probe/setup failed" >&2
  exit 1
fi

# Negative control: the same local probe inside an unshared network namespace.
if bwrap --unshare-net --dev-bind / / python3 - "$PORT" <<'PY' 2>/dev/null
import socket, sys
socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=3)
print("NETWORK REACHED")
PY
then
  echo "G5 netns: negative control REACHED the network — namespace not enforced" >&2
  exit 1
fi

bwrap --unshare-net --dev-bind / / "$AW" build --root "$REPO" >/dev/null
echo "G5 netns: build completed with network denied (positive control ok) — PASS"
