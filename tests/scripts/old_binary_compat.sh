#!/usr/bin/env bash
# 1.0.4 compatibility check for a migrated memory store.
#
# Runs the tagged release binary against a store created by the new binary,
# then proves the new binary repairs and sees the old binary's write.
#
# Usage: old_binary_compat.sh --old <eg-1.0.4> --new <eg-new>
set -euo pipefail

OLD=""
NEW=""
while [ $# -gt 0 ]; do
  case "$1" in
    --old) OLD="$2"; shift 2 ;;
    --new) NEW="$2"; shift 2 ;;
    *) echo "usage: $0 --old <eg-1.0.4> --new <eg-new>" >&2; exit 2 ;;
  esac
done
[ -n "$OLD" ] && [ -n "$NEW" ] || { echo "usage: $0 --old <eg-1.0.4> --new <eg-new>" >&2; exit 2; }
[ -x "$OLD" ] || { echo "old binary is not executable: $OLD" >&2; exit 2; }
[ -x "$NEW" ] || { echo "new binary is not executable: $NEW" >&2; exit 2; }

ROOT=$(mktemp -d "${TMPDIR:-/tmp}/eg-compat-XXXXXX")
trap 'rm -rf "$ROOT"' EXIT
git -C "$ROOT" init -q
export ENGRAM_GLOBAL_DIR="$ROOT/.test_global"

run() { "$1" --root "$ROOT" --no-journal "${@:2}"; }

# New binary creates the store and both indexes.
run "$NEW" mem write seed --body "Bộ nhớ dự án lưu quyết định" --kind fact >/dev/null

# Old binary reads through the legacy table (accented query keeps its skeleton).
run "$OLD" mem query "bộ nhớ" >/dev/null

# Old binary writes only the legacy table with fts_version = 0.
run "$OLD" mem write oldwrite --body "legacy writer note" --kind fact >/dev/null

# New binary write triggers the repair; the old write must then be visible.
run "$NEW" mem write repair --body "repair trigger" --kind fact >/dev/null
OUT=$(run "$NEW" mem query "legacy writer")
printf '%s' "$OUT" | grep -q '^oldwrite ' || { echo "new binary cannot see the old write" >&2; exit 1; }

VER=$(sqlite3 "$ROOT/.engram/memory.db" "PRAGMA user_version")
[ "$VER" = "2" ] || { echo "unexpected user_version: $VER" >&2; exit 1; }

echo "ok: 1.0.4 read/write compatibility and repair verified"
