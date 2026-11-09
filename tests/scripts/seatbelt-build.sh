#!/usr/bin/env bash
# G5 invariant 1 (macOS): the build must complete with network fully denied,
# and the sandbox must actually block a network probe (negative control).
# R6e Blocker 1: the previous version wrote a profile but never applied it.
# The probe targets a controlled local endpoint so a positive control can
# prove the endpoint is reachable and the probe itself works before the
# sandbox is expected to deny it.
set -euo pipefail
AW="$1"; REPO="$2"
PROFILE="$(mktemp -t eg-seatbelt.XXXXXX).sb"
PORT="${ENGRAM_SEATBELT_PORT:-$((20000 + RANDOM % 20000))}"
SERVER_PID=""
cleanup() {
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null || true
  rm -f "$PROFILE"
}
trap cleanup EXIT
cat > "$PROFILE" <<'SB'
(version 1)
(allow default)
(deny network*)
SB

# Controlled reachable endpoint. The positive control runs without the
# profile; a setup/probe error must fail the gate instead of masquerading as
# successful isolation.
/usr/bin/python3 -m http.server "$PORT" --bind 127.0.0.1 >/dev/null 2>&1 &
SERVER_PID=$!
disown "$SERVER_PID" 2>/dev/null || true
if ! /usr/bin/python3 - "$PORT" <<'PY'
import socket, sys, time
port = int(sys.argv[1])
for _ in range(30):
    try:
        s = socket.create_connection(("127.0.0.1", port), timeout=1)
        s.close()
        sys.exit(0)
    except Exception:
        time.sleep(0.1)
sys.exit(1)
PY
then
  echo "G5 seatbelt: positive control cannot reach the local endpoint — probe/setup failed" >&2
  exit 1
fi

# Positive: build succeeds under the profile.
sandbox-exec -f "$PROFILE" "$AW" build --root "$REPO" >/dev/null

# Negative control: the same local probe under the SAME profile must fail.
if sandbox-exec -f "$PROFILE" /usr/bin/python3 - "$PORT" <<'PY' 2>/dev/null
import socket, sys
socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=3)
print("NETWORK REACHED")
PY
then
  echo "G5 seatbelt: negative control REACHED the network — sandbox not enforced" >&2
  exit 1
fi
echo "G5 seatbelt: build completed with network denied (negative control blocked) — PASS"
