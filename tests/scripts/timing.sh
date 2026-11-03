#!/usr/bin/env bash
# G1/G3 timing fallback (hyperfine missing): N runs via /usr/bin/time -l.
# usage: timing.sh <n-runs> <label> <cmd...>
set -euo pipefail
N="$1"; LABEL="$2"; shift 2
for i in $(seq 1 "$N"); do
  /usr/bin/time -l "$@" 2>&1 | awk -v label="$LABEL#$i" '
    /^aw:/ { cmd = $0 }
    /real/ { real = $1 }
    /peak memory footprint/ { rss = $1 }
    END { printf "%s real=%ss rss_bytes=%s\n", label, real, rss; if (cmd != "") print cmd }
  '
done
