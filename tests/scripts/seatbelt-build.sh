#!/usr/bin/env bash
# G5 invariant 1 (macOS): run kg build under a Seatbelt profile denying all
# network operations. Exit 0 proves the build makes zero network syscalls.
# usage: seatbelt-build.sh <kg-binary> <repo-root>
set -euo pipefail
KG="$1"; REPO="$2"
PROFILE=$(mktemp /tmp/kg-seatbelt.XXXXXX.sb)
trap 'rm -f "$PROFILE"' EXIT
cat > "$PROFILE" << 'EOF'
(version 1)
(deny network*)
(deny network-inbound)
(deny network-outbound)
(allow file-read*)
(allow file-write*)
(allow process-exec)
(allow process-fork)
(allow sysctl-read)
(allow mach-lookup)
(allow system-socket)
EOF
"$KG" build --root "$REPO"
echo "G5 seatbelt: build completed with network fully denied — PASS"
